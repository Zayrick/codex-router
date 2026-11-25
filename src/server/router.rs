use axum::{
    Router,
    body::Body,
    extract::{FromRequestParts, State, ws::WebSocketUpgrade},
    http::{Request, StatusCode},
    response::Response,
};
use tower_http::trace::TraceLayer;
use url::Url;

use crate::{
    application::{
        is_known_api_path, is_legacy_admin_path_family, legacy_admin_base_path, match_api_route,
        match_console_route,
    },
    auth::{UserRepository, client_token},
    upstream::relay::is_backend_api_path,
};

use super::{
    account_router::RouteHints,
    api::{RoutedApiIdentity, handle_api},
    console::{console_redirect, handle_console},
    console_security::{apply_proxy_origin, strip_console_cookies},
    oauth::current_time_ms,
    relay::handle_relay,
    response,
    state::AppState,
};

pub fn build(state: AppState) -> Router {
    Router::new()
        .fallback(dispatch)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn dispatch(State(state): State<AppState>, request: Request<Body>) -> Response {
    let (mut parts, body) = request.into_parts();
    let websocket = WebSocketUpgrade::from_request_parts(&mut parts, &state)
        .await
        .ok();
    let request = Request::from_parts(parts, body);
    let mut config = state.config.snapshot().await;
    if let Err(status) = apply_proxy_origin(&request, &mut config.server) {
        return response::empty(status.as_u16());
    }
    let client_url = match client_url(&config.server.public_origin, request.uri()) {
        Some(url) => url,
        None => return response::empty(400),
    };
    let method = request.method().as_str().to_owned();
    let path = client_url.path().to_owned();
    let has_websocket_upgrade = request
        .headers()
        .get("upgrade")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("websocket"));

    if let Some(route) = match_console_route(&method, &path) {
        return handle_console(route, request, client_url, &config, &state).await;
    }
    // Console cookies are never forwarded to upstream services.
    let mut request = request;
    strip_console_cookies(request.headers_mut());

    let output = if let Some(legacy) = legacy_admin_base_path(config.admin.path.as_deref())
        .filter(|base| is_legacy_admin_path_family(&path, base))
    {
        if path == legacy && matches!(method.as_str(), "GET" | "HEAD") {
            console_redirect(StatusCode::PERMANENT_REDIRECT)
        } else {
            gone()
        }
    } else if path == "/account/data" || path.starts_with("/assets/") {
        // Former public endpoints stay local instead of falling through to the relay.
        gone()
    } else if is_backend_api_path(&path) {
        if has_websocket_upgrade && websocket.is_none() {
            response::empty(400)
        } else {
            handle_relay(request, client_url, websocket, &config, &state).await
        }
    } else if path == "/healthz" {
        if method != "GET" {
            response::empty(404)
        } else {
            match state
                .account_router
                .has_available_account(state.config.as_ref(), current_time_ms())
                .await
            {
                Ok(true) => response::empty(204),
                Ok(false) => response::empty(404),
                Err(error) => {
                    tracing::warn!(
                        event = "health_check",
                        status = "failed",
                        code = error.code.as_deref().unwrap_or("health_check_failed")
                    );
                    response::empty(404)
                }
            }
        }
    } else if path == "/" {
        if matches!(method.as_str(), "GET" | "HEAD") {
            console_redirect(StatusCode::FOUND)
        } else {
            response::empty(404)
        }
    } else if method == "OPTIONS" && is_known_api_path(&path) {
        response::with_cors(response::empty(204), &config.server.cors_origin)
    } else if let Some(route) = match_api_route(&method, &client_url, has_websocket_upgrade) {
        if has_websocket_upgrade && websocket.is_none() {
            response::empty(400)
        } else {
            let authorization = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok());
            let api_key = request
                .headers()
                .get("x-api-key")
                .and_then(|value| value.to_str().ok());
            let google_api_key = request
                .headers()
                .get("x-goog-api-key")
                .and_then(|value| value.to_str().ok());
            let token = client_token(authorization, api_key, google_api_key);
            let users = UserRepository::new(state.config.as_ref());
            match users.authenticate_api_key(token.as_deref()).await {
                Ok(authenticated) => {
                    match state
                        .account_router
                        .resolve_with(
                            state.config.as_ref(),
                            &authenticated.user.id,
                            request.headers(),
                            RouteHints {
                                model: None,
                                generative: route.is_generative(),
                            },
                            current_time_ms(),
                        )
                        .await
                    {
                        Ok(Some(account_route)) => {
                            let identity = super::usage::UsageIdentity::from(&authenticated)
                                .with_account_route(
                                    &account_route.account.id,
                                    &account_route.account.name,
                                    account_route
                                        .group
                                        .as_ref()
                                        .map(|group| (group.id.as_str(), group.name.as_str())),
                                );
                            handle_api(
                                route,
                                request,
                                client_url,
                                websocket,
                                &config,
                                &state,
                                RoutedApiIdentity {
                                    usage: identity,
                                    account: account_route,
                                },
                            )
                            .await
                        }
                        Ok(None) | Err(_) => response::empty(404),
                    }
                }
                Err(_) => response::empty(404),
            }
        }
    } else if is_known_api_path(&path) {
        response::empty(404)
    } else if has_websocket_upgrade && websocket.is_none() {
        response::empty(400)
    } else {
        handle_relay(request, client_url, websocket, &config, &state).await
    };

    response::suppress_html_body(output)
}

fn gone() -> Response {
    let mut output = response::empty(410);
    output.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    output
}

fn client_url(public_origin: &str, uri: &axum::http::Uri) -> Option<Url> {
    let path_and_query = uri.path_and_query().map_or("/", |value| value.as_str());
    Url::parse(&format!("{public_origin}{path_and_query}")).ok()
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        http::{Method, StatusCode},
    };
    use tower::ServiceExt;

    use crate::auth::{
        AccountRoutingState, CodexAccount, CredentialKind, RouteAssignment, RouteTargetKind, User,
        UserCredential,
    };

    use super::super::config::{
        AdminConfig, AppConfig, ConfigStore, FrontendConfig, NotificationConfig, PersistentState,
        PublicAccountConfig, ServerConfig, SessionConfig, UpstreamConfig,
    };
    use super::*;

    #[test]
    fn builds_urls_from_the_configured_public_origin() {
        let uri = "/v1/models?client_version=1.2.3".parse().unwrap();
        let url = client_url("https://router.example", &uri).unwrap();
        assert_eq!(
            url.as_str(),
            "https://router.example/v1/models?client_version=1.2.3"
        );
    }

    #[tokio::test]
    async fn serves_local_apis() {
        let directory =
            std::env::temp_dir().join(format!("codex-router-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("web/assets")).unwrap();
        std::fs::write(
            directory.join("web/index.html"),
            r#"<script type="module" nonce="__CODEX_ROUTER_CSP_NONCE__" src="/admin/assets/app.js"></script>"#,
        )
        .unwrap();
        std::fs::write(directory.join("web/assets/app.js"), "export {}").unwrap();
        let path = directory.join("config.toml");
        let usage_path = directory.join("usage.sqlite3");
        let config = AppConfig {
            server: ServerConfig {
                public_origin: ORIGIN.into(),
                ..ServerConfig::default()
            },
            admin: AdminConfig {
                path: Some("secret".into()),
                secret: "admin-secret".into(),
            },
            upstream: UpstreamConfig {
                chatgpt_proxy: None,
                codex_resets_url: "https://codex-resets.com/api/v1/status".into(),
                codex_client_version: crate::upstream::codex::DEFAULT_CODEX_CLIENT_VERSION.into(),
                websocket_idle_timeout_seconds: 0,
            },
            usage_tracking: super::super::config::UsageTrackingConfig {
                database_path: usage_path.display().to_string(),
                ..Default::default()
            },
            notifications: NotificationConfig::default(),
            public_account: PublicAccountConfig::default(),
            frontend: FrontendConfig { dir: "web".into() },
            session: SessionConfig::default(),
            state: PersistentState {
                users: vec![
                    User {
                        id: USER_ID.into(),
                        name: "test".into(),
                        enabled: true,
                        credentials: vec![
                            credential(API_KEY_ID, CredentialKind::ApiKey, "sk-test-value-123!"),
                            credential(ACCOUNT_ID_ID, CredentialKind::AccountId, "account-test"),
                        ],
                    },
                    User {
                        id: "00000000-0000-4000-8000-000000000004".into(),
                        name: "unassigned".into(),
                        enabled: true,
                        credentials: vec![credential(
                            "00000000-0000-4000-8000-000000000014",
                            CredentialKind::ApiKey,
                            "sk-unassigned-123!",
                        )],
                    },
                ],
                account_routing: AccountRoutingState {
                    accounts: vec![CodexAccount {
                        id: "00000000-0000-4000-8000-000000000003".into(),
                        name: "test-codex".into(),
                        enabled: true,
                    }],
                    groups: vec![],
                    routes: vec![RouteAssignment {
                        user_id: USER_ID.into(),
                        target_type: RouteTargetKind::Account,
                        target_id: "00000000-0000-4000-8000-000000000003".into(),
                    }],
                },
                ..PersistentState::default()
            },
        };
        tokio::fs::write(&path, toml::to_string_pretty(&config).unwrap())
            .await
            .unwrap();
        let store = ConfigStore::load(path.clone()).await.unwrap();
        let app = build(AppState::new(store).await.unwrap());

        let preflight = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/v1/messages/count_tokens")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            preflight
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            "*"
        );

        let hidden = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/messages/count_tokens")
                    .header("authorization", "Bearer wrong")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"gpt-5","messages":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

        let unassigned = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/messages/count_tokens")
                    .header("authorization", "Bearer sk-unassigned-123!")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"gpt-5","messages":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unassigned.status(), StatusCode::NOT_FOUND);

        let counted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/messages/count_tokens")
                    .header("authorization", "Bearer sk-test-value-123!")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"gpt-5","messages":[{"role":"user","content":"hello"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(counted.status(), StatusCode::OK);
        let body = to_bytes(counted.into_body(), 1024 * 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(payload["input_tokens"].as_u64().is_some());

        // The console shell and its assets are served from frontend.dir.
        let page = send(&app, Request::get("/admin")).await;
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
        let csp = page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(page.into_body(), 1024 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(!body.contains("__CODEX_ROUTER_CSP_NONCE__"));
        let nonce = body
            .split("nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(csp.contains(&format!("'nonce-{nonce}'")));
        let asset = send(&app, Request::get("/admin/assets/app.js")).await;
        assert_eq!(asset.status(), StatusCode::OK);

        // Root, the trailing slash, and the deprecated hidden path all lead to /admin.
        for (uri, status) in [
            ("/", StatusCode::FOUND),
            ("/admin/", StatusCode::PERMANENT_REDIRECT),
            ("/secret/admin", StatusCode::PERMANENT_REDIRECT),
        ] {
            let response = send(&app, Request::get(uri)).await;
            assert_eq!(response.status(), status, "{uri}");
            assert_eq!(response.headers()["location"], "/admin");
        }
        // Former and unknown console paths are answered locally, never relayed.
        for (method, uri, status) in [
            (Method::GET, "/secret/admin/state", StatusCode::GONE),
            (Method::POST, "/account/data", StatusCode::GONE),
            (Method::GET, "/assets/old.js", StatusCode::GONE),
            (Method::GET, "/admin/unknown", StatusCode::NOT_FOUND),
            (Method::GET, "/admin/api/unknown", StatusCode::NOT_FOUND),
            (
                Method::GET,
                "/admin/assets/missing.js",
                StatusCode::NOT_FOUND,
            ),
        ] {
            let response = send(&app, Request::builder().method(method).uri(uri)).await;
            assert_eq!(response.status(), status, "{uri}");
        }

        // Anonymous session lookups succeed with no session; protected APIs need one.
        let anonymous = send(&app, Request::get("/admin/api/session")).await;
        assert_eq!(anonymous.status(), StatusCode::OK);
        assert_eq!(
            json_body(anonymous).await["session"],
            serde_json::Value::Null
        );
        let anonymous = send(&app, Request::get("/admin/api/account?range=24h")).await;
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

        // Sign-in requires a same-origin JSON request.
        let cross_site = send(
            &app,
            Request::post("/admin/api/session")
                .header("origin", "https://evil.example")
                .header("content-type", "application/json"),
        )
        .await;
        assert_eq!(cross_site.status(), StatusCode::FORBIDDEN);
        let wrong = sign_in(&app, "sk-wrong-value-123!").await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert!(wrong.headers().get("set-cookie").is_none());

        // Every credential of a user opens that user's console, which sees only
        // the user's own usage and cannot use management APIs.
        for credential in ["sk-test-value-123!", "account-test"] {
            let signed_in = sign_in(&app, credential).await;
            assert_eq!(signed_in.status(), StatusCode::OK);
            let cookie = session_cookie(&signed_in);
            let set_cookie = signed_in.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .to_owned();
            assert!(set_cookie.starts_with("__Host-codex-session="));
            for attribute in ["Path=/", "Secure", "HttpOnly", "SameSite=Lax", "Max-Age="] {
                assert!(set_cookie.contains(attribute), "{set_cookie}");
            }
            let session = json_body(signed_in).await;
            assert_eq!(session["session"]["role"], "user");

            let account = send(
                &app,
                Request::get("/admin/api/account?range=7d").header("cookie", &cookie),
            )
            .await;
            assert_eq!(account.status(), StatusCode::OK);
            assert_eq!(account.headers()["cache-control"], "no-store");
            let payload = json_body(account).await;
            assert_eq!(payload["usage"]["range"], "7d");

            let quota = send(
                &app,
                Request::get("/admin/api/account/quota").header("cookie", &cookie),
            )
            .await;
            assert_eq!(quota.status(), StatusCode::OK);
            let quota = json_body(quota).await;
            assert_eq!(quota["identity"]["name"], "test");
            assert_eq!(quota["route"]["targetType"], "account");
            let cycle = send(
                &app,
                Request::get("/admin/api/account?range=cycle").header("cookie", &cookie),
            )
            .await;
            assert_eq!(cycle.status(), StatusCode::BAD_REQUEST);

            let forbidden = send(
                &app,
                Request::get("/admin/api/state").header("cookie", &cookie),
            )
            .await;
            assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        }

        // The administrator signs in with the same form and reaches management APIs.
        let admin = sign_in(&app, "admin-secret").await;
        assert_eq!(admin.status(), StatusCode::OK);
        let admin_cookie = session_cookie(&admin);
        assert_eq!(json_body(admin).await["session"]["role"], "admin");
        let state_response = send(
            &app,
            Request::get("/admin/api/state").header("cookie", &admin_cookie),
        )
        .await;
        assert_eq!(state_response.status(), StatusCode::OK);
        let user_view = send(
            &app,
            Request::get("/admin/api/account?range=24h").header("cookie", &admin_cookie),
        )
        .await;
        assert_eq!(user_view.status(), StatusCode::FORBIDDEN);

        // The live view streams Server-Sent Events, starting with a snapshot.
        let live = send(
            &app,
            Request::get("/admin/api/live").header("cookie", &admin_cookie),
        )
        .await;
        assert_eq!(live.status(), StatusCode::OK);
        assert!(
            live.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        let mut frames = live.into_body().into_data_stream();
        use futures_util::StreamExt;
        let retry = frames.next().await.unwrap().unwrap();
        assert!(retry.starts_with(b"retry: "));
        let snapshot = String::from_utf8(frames.next().await.unwrap().unwrap().to_vec()).unwrap();
        assert!(snapshot.starts_with("event: snapshot\ndata: {"));
        assert!(snapshot.contains("\"connections\":[]"));
        let user_live = send(
            &app,
            Request::get("/admin/api/live").header(
                "cookie",
                &session_cookie(&sign_in(&app, "sk-test-value-123!").await),
            ),
        )
        .await;
        assert_eq!(user_live.status(), StatusCode::FORBIDDEN);

        // Disabling a key ends its sessions, and re-enabling does not revive them.
        let user = sign_in(&app, "sk-test-value-123!").await;
        let user_cookie = session_cookie(&user);
        for enabled in [false, true] {
            let update = send(
                &app,
                Request::put("/admin/api/users")
                    .header("cookie", &admin_cookie)
                    .header("origin", ORIGIN)
                    .header("content-type", "application/json")
                    .body_json(user_body("test", "sk-test-value-123!", enabled)),
            )
            .await;
            assert_eq!(update.status(), StatusCode::OK);
        }
        let revoked = send(
            &app,
            Request::get("/admin/api/account?range=24h").header("cookie", &user_cookie),
        )
        .await;
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

        // Rotating a key's value also ends sessions bound to the old value.
        let user = sign_in(&app, "sk-test-value-123!").await;
        let user_cookie = session_cookie(&user);
        let rotate = send(
            &app,
            Request::put("/admin/api/users")
                .header("cookie", &admin_cookie)
                .header("origin", ORIGIN)
                .header("content-type", "application/json")
                .body_json(user_body("test", "sk-rotated-value-123!", true)),
        )
        .await;
        assert_eq!(rotate.status(), StatusCode::OK);
        let rotated = send(
            &app,
            Request::get("/admin/api/session").header("cookie", &user_cookie),
        )
        .await;
        assert_eq!(json_body(rotated).await["session"], serde_json::Value::Null);

        // Restoring the previous value does not revive sessions bound to it.
        let user = sign_in(&app, "sk-rotated-value-123!").await;
        let rotated_cookie = session_cookie(&user);
        for key in ["sk-test-value-123!", "sk-rotated-value-123!"] {
            let restore = send(
                &app,
                Request::put("/admin/api/users")
                    .header("cookie", &admin_cookie)
                    .header("origin", ORIGIN)
                    .header("content-type", "application/json")
                    .body_json(user_body("test", key, true)),
            )
            .await;
            assert_eq!(restore.status(), StatusCode::OK);
        }
        for cookie in [&user_cookie, &rotated_cookie] {
            let revived = send(
                &app,
                Request::get("/admin/api/session").header("cookie", cookie),
            )
            .await;
            assert_eq!(json_body(revived).await["session"], serde_json::Value::Null);
        }
        // An unchanged update keeps existing sessions.
        let user = sign_in(&app, "sk-rotated-value-123!").await;
        let kept_cookie = session_cookie(&user);
        let rename = send(
            &app,
            Request::put("/admin/api/users")
                .header("cookie", &admin_cookie)
                .header("origin", ORIGIN)
                .header("content-type", "application/json")
                .body_json(user_body("renamed", "sk-rotated-value-123!", true)),
        )
        .await;
        assert_eq!(rename.status(), StatusCode::OK);
        let kept = send(
            &app,
            Request::get("/admin/api/session").header("cookie", &kept_cookie),
        )
        .await;
        assert_eq!(json_body(kept).await["session"]["role"], "user");

        // A session opened with the account id survives changes to the API key
        // only when they do not touch it; removing it ends the session.
        let relay_user = sign_in(&app, "account-test").await;
        let relay_cookie = session_cookie(&relay_user);
        let mut without_account_id = user_body("renamed", "sk-rotated-value-123!", true);
        without_account_id["credentials"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        let removed = send(
            &app,
            Request::put("/admin/api/users")
                .header("cookie", &admin_cookie)
                .header("origin", ORIGIN)
                .header("content-type", "application/json")
                .body_json(without_account_id),
        )
        .await;
        assert_eq!(removed.status(), StatusCode::OK);
        let ended = send(
            &app,
            Request::get("/admin/api/session").header("cookie", &relay_cookie),
        )
        .await;
        assert_eq!(json_body(ended).await["session"], serde_json::Value::Null);
        assert_eq!(
            sign_in(&app, "account-test").await.status(),
            StatusCode::UNAUTHORIZED
        );

        // A credential equal to the admin secret is refused.
        let conflict = send(
            &app,
            Request::post("/admin/api/users")
                .header("cookie", &admin_cookie)
                .header("origin", ORIGIN)
                .header("content-type", "application/json")
                .body_json(serde_json::json!({
                    "name": "conflict",
                    "enabled": true,
                    "credentials": [
                        { "type": "api_key", "value": "admin-secret", "enabled": true }
                    ],
                })),
        )
        .await;
        assert_eq!(conflict.status(), StatusCode::BAD_REQUEST);

        // Signing out revokes the server-side session and clears the cookie.
        let sign_out = send(
            &app,
            Request::delete("/admin/api/session")
                .header("cookie", &admin_cookie)
                .header("origin", ORIGIN),
        )
        .await;
        assert_eq!(sign_out.status(), StatusCode::NO_CONTENT);
        assert!(
            sign_out.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        let after = send(
            &app,
            Request::get("/admin/api/state").header("cookie", &admin_cookie),
        )
        .await;
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

        let _ = tokio::fs::remove_dir_all(directory).await;
    }

    const ORIGIN: &str = "https://router.example";
    const USER_ID: &str = "00000000-0000-4000-8000-000000000001";
    const API_KEY_ID: &str = "00000000-0000-4000-8000-000000000011";
    const ACCOUNT_ID_ID: &str = "00000000-0000-4000-8000-000000000012";

    fn credential(id: &str, kind: CredentialKind, value: &str) -> UserCredential {
        UserCredential {
            id: id.into(),
            kind,
            value: value.into(),
            name: String::new(),
            enabled: true,
        }
    }

    /// The test user with its API key set to `key` and its account id kept.
    fn user_body(name: &str, key: &str, enabled: bool) -> serde_json::Value {
        serde_json::json!({
            "id": USER_ID,
            "name": name,
            "enabled": enabled,
            "credentials": [
                { "id": API_KEY_ID, "type": "api_key", "value": key, "enabled": true },
                { "id": ACCOUNT_ID_ID, "type": "account_id", "value": "account-test", "enabled": true },
            ],
        })
    }

    trait JsonBody {
        fn body_json(self, value: serde_json::Value) -> Request<Body>;
    }

    impl JsonBody for axum::http::request::Builder {
        fn body_json(self, value: serde_json::Value) -> Request<Body> {
            self.body(Body::from(value.to_string())).unwrap()
        }
    }

    trait IntoTestRequest {
        fn into_request(self) -> Request<Body>;
    }

    impl IntoTestRequest for axum::http::request::Builder {
        fn into_request(self) -> Request<Body> {
            self.body(Body::empty()).unwrap()
        }
    }

    impl IntoTestRequest for Request<Body> {
        fn into_request(self) -> Request<Body> {
            self
        }
    }

    async fn send(app: &Router, request: impl IntoTestRequest) -> Response {
        app.clone().oneshot(request.into_request()).await.unwrap()
    }

    async fn sign_in(app: &Router, credential: &str) -> Response {
        send(
            app,
            Request::post("/admin/api/session")
                .header("origin", ORIGIN)
                .header("content-type", "application/json")
                .body_json(serde_json::json!({ "credential": credential })),
        )
        .await
    }

    fn session_cookie(response: &Response) -> String {
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}
