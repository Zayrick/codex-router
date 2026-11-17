//! The unified web console below `/admin`: one sign-in for administrators and
//! downstream users (with any of their credentials), backed by server-side sessions.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    response::Response,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use url::Url;

use crate::{
    application::{CONSOLE_BASE_PATH, ConsoleRoute},
    auth::{CredentialKind, User, UserRepository, constant_time_equal},
    core::{ApiError, AppResult},
};

use super::{
    account::{signed_in_account_dashboard, signed_in_account_quota, user_scope},
    admin::handle_admin,
    body,
    config::AppConfig,
    console_security::{
        clear_session_cookies, client_ip, has_json_content_type, is_same_origin_write,
        session_cookie, session_token, uses_https,
    },
    frontend,
    oauth::current_time_ms,
    response,
    session_store::{Principal, SessionRecord},
    state::AppState,
    usage_api,
};

const MAX_LOGIN_BODY_BYTES: usize = 8 * 1024;
const MAX_CREDENTIAL_CHARS: usize = 512;
const BINDING_PURPOSE: &[u8] = b"codex-router/console-session-binding/v1";

pub async fn handle_console(
    route: ConsoleRoute,
    request: Request<Body>,
    client_url: Url,
    config: &AppConfig,
    state: &AppState,
) -> Response {
    let frontend_dir = state.config.resolve_path(&config.frontend.dir);
    match route {
        ConsoleRoute::Page => frontend::application_page(request.method(), &frontend_dir).await,
        ConsoleRoute::PageRedirect => redirect(CONSOLE_BASE_PATH, StatusCode::PERMANENT_REDIRECT),
        ConsoleRoute::Asset => frontend::asset_response(request, &frontend_dir).await,
        ConsoleRoute::NotFound => no_store(response::empty(404)),
        _ => {
            let output = match api(route, request, client_url, config, state).await {
                Ok(output) => output,
                Err(error) => response::api_error(&error),
            };
            no_store(output)
        }
    }
}

async fn api(
    route: ConsoleRoute,
    request: Request<Body>,
    client_url: Url,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let now_ms = current_time_ms();
    if request.method() != Method::GET && !is_same_origin_write(request.headers(), config) {
        return Err(invalid_origin());
    }
    match route {
        ConsoleRoute::SessionCreate => return sign_in(request, config, state, now_ms).await,
        ConsoleRoute::SessionDelete => return sign_out(request.headers(), config, state).await,
        _ => {}
    }

    let session = authenticate(request.headers(), config, state, now_ms).await?;
    let mut output = match (route, session.as_ref()) {
        (ConsoleRoute::SessionGet, session) => response::json(
            &json!({ "session": session.map(|session| SessionView::from(&session.record)) }),
            200,
        )?,
        (_, None) => return Err(session_required()),
        (ConsoleRoute::Account, Some(session)) => {
            let range = client_url
                .query_pairs()
                .find(|(name, _)| name == "range")
                .map(|(_, value)| value.into_owned());
            signed_in_account_dashboard(&session.record.principal, range.as_deref(), config, state)
                .await?
        }
        (ConsoleRoute::AccountAnalytics, Some(session)) => {
            let scope = user_scope(&session.record.principal)?;
            usage_api::analytics(&client_url, scope, config, state).await?
        }
        (ConsoleRoute::AccountEvents, Some(session)) => {
            let scope = user_scope(&session.record.principal)?;
            usage_api::events(&client_url, scope, config, state).await?
        }
        (ConsoleRoute::AccountQuota, Some(session)) => {
            signed_in_account_quota(&session.record.principal, config, state).await?
        }
        (ConsoleRoute::Admin(route), Some(session)) => {
            if !session.record.principal.is_admin() {
                return Err(admin_required());
            }
            handle_admin(route, request, client_url, config, state).await
        }
        (_, Some(_)) => response::empty(404),
    };
    if let Some(session) = session.filter(|session| session.renewed) {
        append_cookie(
            &mut output,
            &session_cookie(
                config,
                &session.token,
                cookie_max_age(&session.record, now_ms),
            ),
        );
    }
    Ok(output)
}

struct AuthenticatedSession {
    token: String,
    record: SessionRecord,
    /// Whether this request extended the session (and must re-issue the cookie).
    renewed: bool,
}

/// Resolves the session cookie, re-checking on every request that the
/// principal still exists, is enabled, and still holds the credential used to
/// sign in. Database failures are errors, never anonymous access.
async fn authenticate(
    headers: &HeaderMap,
    config: &AppConfig,
    state: &AppState,
    now_ms: i64,
) -> AppResult<Option<AuthenticatedSession>> {
    let Some(token) = session_token(headers, config) else {
        return Ok(None);
    };
    let Some(mut record) = state
        .sessions
        .find(&token, now_ms)
        .await
        .map_err(session_store_error)?
    else {
        return Ok(None);
    };
    let binding_key = binding_key(config)?;
    let identities = Identities::load(config, state).await?;
    let still_bound = identities
        .current_credentials(&record.principal)
        .into_iter()
        .map(|credential| credential_tag(&binding_key, &record.principal, credential))
        .fold(false, |bound, tag| {
            bound | bool::from(tag.ct_eq(&record.credential_tag))
        });
    if !still_bound {
        if let Err(error) = state.sessions.revoke(record.token_hash).await {
            tracing::warn!(event = "console_session_revoke", status = "failed", error = %error);
        }
        return Ok(None);
    }
    let renewed = state
        .sessions
        .renew_if_due(&mut record, now_ms)
        .await
        .map_err(session_store_error)?;
    Ok(Some(AuthenticatedSession {
        token,
        record,
        renewed,
    }))
}

#[derive(Deserialize)]
struct SignInInput {
    credential: String,
}

async fn sign_in(
    request: Request<Body>,
    config: &AppConfig,
    state: &AppState,
    now_ms: i64,
) -> AppResult<Response> {
    if !has_json_content_type(request.headers()) {
        return Err(invalid_sign_in_request());
    }
    let ip = client_ip(&request, config);
    // Reserve the attempt before reading the body, so concurrent requests
    // cannot all pass the throttle check before any failure is recorded.
    let attempt = match state.login_throttle.begin(ip, now_ms) {
        Ok(attempt) => attempt,
        Err(seconds) => {
            let mut output = response::api_error(&too_many_attempts());
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                output.headers_mut().insert(header::RETRY_AFTER, value);
            }
            return Ok(output);
        }
    };
    let (parts, body) = request.into_parts();
    let bytes = body::read_limited_body(&parts.headers, body, MAX_LOGIN_BODY_BYTES).await?;
    let input: SignInInput =
        serde_json::from_slice(&bytes).map_err(|_| invalid_sign_in_request())?;
    let credential = input.credential;
    if credential.is_empty() || credential.chars().count() > MAX_CREDENTIAL_CHARS {
        attempt.fail(now_ms);
        return Err(invalid_credentials());
    }

    // Identification and session creation run under the credential lock, so a
    // concurrent disable, delete, or credential change either completes first
    // (and this sign-in fails) or revokes the session created here.
    let _credentials = state.credential_lock.lock().await;
    let config = state.config.snapshot().await;
    let identities = Identities::load(&config, state).await?;
    let (principal, stored_credential) = match identities.identify(&credential) {
        Identified::One(principal, stored) => (principal, stored),
        Identified::Ambiguous => {
            tracing::error!(
                event = "console_sign_in",
                status = "rejected",
                reason = "credential_matches_multiple_identity_types"
            );
            attempt.fail(now_ms);
            return Err(invalid_credentials());
        }
        Identified::None => {
            tracing::info!(event = "console_sign_in", status = "failed", client = ?ip);
            attempt.fail(now_ms);
            return Err(invalid_credentials());
        }
    };
    // A valid credential ends this attempt without clearing earlier failures
    // from the same client, so user credentials cannot reset admin guessing.
    drop(attempt);
    if principal.is_admin() && insecure_public_origin(&config) {
        return Err(insecure_admin_origin());
    }

    let binding_key = binding_key(&config)?;
    let tag = credential_tag(&binding_key, &principal, &stored_credential);
    let (token, record) = state
        .sessions
        .create(principal, tag, now_ms)
        .await
        .map_err(session_store_error)?;
    tracing::info!(
        event = "console_sign_in",
        status = "succeeded",
        kind = record.principal.kind()
    );
    let mut output = response::json(&json!({ "session": SessionView::from(&record) }), 200)?;
    append_cookie(
        &mut output,
        &session_cookie(&config, &token, cookie_max_age(&record, now_ms)),
    );
    Ok(output)
}

async fn sign_out(
    headers: &HeaderMap,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    if let Some(token) = session_token(headers, config) {
        state
            .sessions
            .revoke(super::session_store::token_hash(&token))
            .await
            .map_err(session_store_error)?;
    }
    let mut output = response::empty(204);
    for cookie in clear_session_cookies(config) {
        append_cookie(&mut output, &cookie);
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
enum Identified {
    None,
    /// The principal and the stored credential value that matched.
    One(Principal, String),
    /// The same string is configured for the administrator and a user.
    /// Signing in is refused rather than silently preferring the admin.
    Ambiguous,
}

/// Current sign-in credentials.
struct Identities {
    admin_secret: String,
    users: Vec<User>,
}

impl Identities {
    async fn load(config: &AppConfig, state: &AppState) -> AppResult<Self> {
        Ok(Self {
            admin_secret: config.admin.secret.clone(),
            users: UserRepository::new(state.config.as_ref()).read().await?,
        })
    }

    /// Identifies a credential as the admin secret or one of a user's API
    /// keys or account ids. Disabled records still count towards ambiguity.
    fn identify(&self, credential: &str) -> Identified {
        let admin = constant_time_equal(credential, &self.admin_secret);
        let trimmed = credential.trim();
        let mut matched = None;
        for user in &self.users {
            for entry in &user.credentials {
                let candidate = match entry.kind {
                    CredentialKind::ApiKey => credential,
                    CredentialKind::AccountId => trimmed,
                };
                if !candidate.is_empty()
                    && constant_time_equal(candidate, &entry.value)
                    && matched.is_none()
                {
                    matched = Some((user, entry));
                }
            }
        }
        match (admin, matched) {
            (true, Some(_)) => Identified::Ambiguous,
            (true, None) => Identified::One(Principal::Admin, self.admin_secret.clone()),
            (false, Some((user, entry))) if user.enabled && entry.enabled => {
                Identified::One(Principal::User(user.id.clone()), entry.value.clone())
            }
            (false, _) => Identified::None,
        }
    }

    /// Credentials that may currently back a session of the principal.
    fn current_credentials(&self, principal: &Principal) -> Vec<&str> {
        match principal {
            Principal::Admin => vec![self.admin_secret.as_str()],
            Principal::User(id) => self
                .users
                .iter()
                .filter(|user| !id.is_empty() && user.id == *id)
                .flat_map(User::active_credentials)
                .map(|credential| credential.value.as_str())
                .collect(),
        }
    }
}

/// Binds a session to the exact credential that created it, so rotating the
/// admin secret or removing, disabling, or changing a user credential ends the
/// sessions it created.
fn credential_tag(key: &[u8; 32], principal: &Principal, credential: &str) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key");
    for part in [
        BINDING_PURPOSE,
        principal.kind().as_bytes(),
        principal.id().as_bytes(),
        credential.as_bytes(),
    ] {
        mac.update(&(part.len() as u64).to_be_bytes());
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

fn binding_key(config: &AppConfig) -> AppResult<[u8; 32]> {
    config.session_binding_key().ok_or_else(|| {
        ApiError::new(500, "The session binding key is unavailable.")
            .with_kind("configuration_error")
            .with_code("session_unavailable")
    })
}

/// Plain HTTP on a non-loopback host exposes admin cookies to the network.
pub fn insecure_public_origin(config: &AppConfig) -> bool {
    if uses_https(config) || config.session.allow_insecure_http {
        return false;
    }
    let Ok(url) = Url::parse(&config.server.public_origin) else {
        return true;
    };
    match url.host() {
        Some(url::Host::Domain(domain)) => !domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => !ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => !ip.is_loopback(),
        None => true,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionView {
    role: &'static str,
    created_at: i64,
    expires_at: i64,
    absolute_expires_at: i64,
}

impl From<&SessionRecord> for SessionView {
    fn from(record: &SessionRecord) -> Self {
        Self {
            role: if record.principal.is_admin() {
                "admin"
            } else {
                "user"
            },
            created_at: record.created_at_ms,
            expires_at: record.expires_at_ms(),
            absolute_expires_at: record.absolute_expires_at_ms,
        }
    }
}

fn cookie_max_age(record: &SessionRecord, now_ms: i64) -> i64 {
    (record.expires_at_ms() - now_ms) / 1_000
}

fn append_cookie(output: &mut Response, cookie: &str) {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        output.headers_mut().append(header::SET_COOKIE, value);
    }
}

fn no_store(mut output: Response) -> Response {
    output
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    output
}

fn redirect(location: &'static str, status: StatusCode) -> Response {
    let mut output = response::empty(status.as_u16());
    output
        .headers_mut()
        .insert(header::LOCATION, HeaderValue::from_static(location));
    output
}

/// Redirect used for `/` and the deprecated hidden admin page.
pub fn console_redirect(status: StatusCode) -> Response {
    no_store(redirect(CONSOLE_BASE_PATH, status))
}

fn session_store_error(error: anyhow::Error) -> ApiError {
    tracing::error!(event = "console_session_store", status = "failed", error = %error);
    ApiError::new(503, "The session service is temporarily unavailable.")
        .with_kind("server_error")
        .with_code("session_unavailable")
}

fn session_required() -> ApiError {
    ApiError::new(401, "The console session is missing or expired.")
        .with_kind("authentication_error")
        .with_code("invalid_session")
}

fn admin_required() -> ApiError {
    ApiError::new(403, "This action requires an administrator session.")
        .with_kind("permission_error")
        .with_code("admin_required")
}

fn invalid_credentials() -> ApiError {
    ApiError::new(401, "The credential is invalid or disabled.")
        .with_kind("authentication_error")
        .with_code("invalid_credentials")
}

fn too_many_attempts() -> ApiError {
    ApiError::new(429, "Too many failed sign-in attempts. Try again later.")
        .with_kind("rate_limit_error")
        .with_code("too_many_sign_in_attempts")
}

fn insecure_admin_origin() -> ApiError {
    ApiError::new(
        403,
        "Administrator sign-in requires HTTPS. Set session.allow_insecure_http to override.",
    )
    .with_kind("permission_error")
    .with_code("insecure_admin_origin")
}

fn invalid_sign_in_request() -> ApiError {
    ApiError::new(
        400,
        "The sign-in request must be a JSON object with a credential.",
    )
    .with_kind("invalid_request_error")
    .with_code("invalid_sign_in_request")
}

fn invalid_origin() -> ApiError {
    ApiError::new(403, "The request origin is not allowed.")
        .with_kind("permission_error")
        .with_code("invalid_origin")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identities(config: &AppConfig) -> Identities {
        Identities {
            admin_secret: config.admin.secret.clone(),
            users: config.state.users.clone(),
        }
    }

    fn identify(credential: &str, config: &AppConfig) -> Identified {
        identities(config).identify(credential)
    }

    const USER: &str = "00000000-0000-4000-8000-000000000001";

    fn config() -> AppConfig {
        toml::from_str(
            r#"
            [admin]
            secret = "admin-secret"
            [upstream]
            [state]
            session_binding_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            [[state.users]]
            id = "00000000-0000-4000-8000-000000000001"
            name = "alice"
            enabled = true
            [[state.users.credentials]]
            id = "00000000-0000-4000-8000-000000000011"
            type = "api_key"
            value = "sk-enabled-123!"
            enabled = true
            [[state.users.credentials]]
            id = "00000000-0000-4000-8000-000000000012"
            type = "api_key"
            value = "sk-disabled-123!"
            enabled = false
            [[state.users.credentials]]
            id = "00000000-0000-4000-8000-000000000013"
            type = "account_id"
            value = "account-123"
            enabled = true
            "#,
        )
        .unwrap()
    }

    #[test]
    fn every_credential_signs_in_as_its_user() {
        let config = config();
        assert_eq!(
            identify("admin-secret", &config),
            Identified::One(Principal::Admin, "admin-secret".into())
        );
        assert_eq!(
            identify("sk-enabled-123!", &config),
            Identified::One(Principal::User(USER.into()), "sk-enabled-123!".into())
        );
        assert_eq!(
            identify(" account-123 ", &config),
            Identified::One(Principal::User(USER.into()), "account-123".into())
        );
        assert_eq!(identify("sk-disabled-123!", &config), Identified::None);
        assert_eq!(identify("nope", &config), Identified::None);

        let mut disabled = config.clone();
        disabled.state.users[0].enabled = false;
        assert_eq!(identify("sk-enabled-123!", &disabled), Identified::None);
    }

    #[test]
    fn sessions_stay_bound_to_active_credentials() {
        let mut config = config();
        let principal = Principal::User(USER.into());
        assert_eq!(
            identities(&config).current_credentials(&principal),
            vec!["sk-enabled-123!", "account-123"]
        );
        config.state.users[0].credentials[0].enabled = false;
        assert_eq!(
            identities(&config).current_credentials(&principal),
            vec!["account-123"]
        );
        config.state.users[0].enabled = false;
        assert!(
            identities(&config)
                .current_credentials(&principal)
                .is_empty()
        );
    }

    #[test]
    fn refuses_credentials_shared_with_the_admin() {
        let mut config = config();
        config.state.users[0].credentials[1].value = "admin-secret".into();
        assert_eq!(identify("admin-secret", &config), Identified::Ambiguous);
    }

    #[test]
    fn binding_tag_changes_with_the_credential() {
        let key = [9; 32];
        let principal = Principal::User("id".into());
        let tag = credential_tag(&key, &principal, "secret");
        assert_eq!(tag, credential_tag(&key, &principal, "secret"));
        assert_ne!(tag, credential_tag(&key, &principal, "rotated"));
        assert_ne!(tag, credential_tag(&[8; 32], &principal, "secret"));
        assert_ne!(
            tag,
            credential_tag(&key, &Principal::User("other".into()), "secret")
        );
    }

    #[test]
    fn insecure_origin_detection_allows_loopback_and_https() {
        let mut config = config();
        for (origin, insecure) in [
            ("http://127.0.0.1:8787", false),
            ("http://localhost:8787", false),
            ("http://[::1]:8787", false),
            ("https://router.example", false),
            ("http://router.example", true),
            ("http://192.168.1.10:8787", true),
        ] {
            config.server.public_origin = origin.into();
            assert_eq!(insecure_public_origin(&config), insecure, "{origin}");
        }
        config.session.allow_insecure_http = true;
        assert!(!insecure_public_origin(&config));
    }
}
