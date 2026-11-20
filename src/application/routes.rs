use crate::{
    protocol::gemini::{GeminiAction, match_gemini_action_path, match_gemini_model_path},
    protocol::openai::images::ImageOperation,
    upstream::codex::{is_codex_proxy_path, is_codex_proxy_request_allowed},
};
use url::Url;

use super::RequestAdapter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolFamily {
    OpenAi,
    Anthropic,
    Gemini,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiRoute {
    Models,
    Images(ImageOperation),
    Responses,
    Compact,
    Proxy,
    ChatCompletions,
    Completions,
    Messages,
    MessageTokens,
    GeminiModels,
    GeminiModel { model: String },
    GeminiGenerate { model: String, stream: bool },
    GeminiTokens { model: String },
}

impl ApiRoute {
    pub const fn family(&self) -> ProtocolFamily {
        match self {
            Self::Messages | Self::MessageTokens => ProtocolFamily::Anthropic,
            Self::GeminiModels
            | Self::GeminiModel { .. }
            | Self::GeminiGenerate { .. }
            | Self::GeminiTokens { .. } => ProtocolFamily::Gemini,
            Self::Responses | Self::Compact | Self::Proxy => ProtocolFamily::Codex,
            Self::Models | Self::Images(_) | Self::ChatCompletions | Self::Completions => {
                ProtocolFamily::OpenAi
            }
        }
    }

    pub fn adapter(&self) -> Option<RequestAdapter> {
        match self {
            Self::ChatCompletions => Some(RequestAdapter::OpenAiChat),
            Self::Completions => Some(RequestAdapter::OpenAiCompletion),
            Self::Messages => Some(RequestAdapter::AnthropicMessages),
            Self::GeminiGenerate { model, stream } => Some(RequestAdapter::GeminiContent {
                model: model.clone(),
                stream: *stream,
            }),
            _ => None,
        }
    }
}

pub fn match_api_route(
    method: &str,
    client_url: &Url,
    websocket_upgrade: bool,
) -> Option<ApiRoute> {
    let route = match_api_path(client_url.path())?;
    match &route {
        ApiRoute::Models | ApiRoute::GeminiModels | ApiRoute::GeminiModel { .. } => {
            (method == "GET").then_some(route)
        }
        ApiRoute::Responses => {
            ((method == "POST") || (method == "GET" && websocket_upgrade)).then_some(route)
        }
        ApiRoute::Proxy => {
            is_codex_proxy_request_allowed(method, client_url, websocket_upgrade).then_some(route)
        }
        _ => (method == "POST").then_some(route),
    }
}

pub fn is_known_api_path(pathname: &str) -> bool {
    match_api_path(pathname).is_some()
}

fn match_api_path(pathname: &str) -> Option<ApiRoute> {
    match pathname {
        "/v1/models" => Some(ApiRoute::Models),
        "/v1/images/generations" | "/v1/images/generations/" => {
            Some(ApiRoute::Images(ImageOperation::Generate))
        }
        "/v1/images/edits" | "/v1/images/edits/" => Some(ApiRoute::Images(ImageOperation::Edit)),
        "/v1/responses" | "/v1/responses/" => Some(ApiRoute::Responses),
        "/v1/responses/compact" | "/v1/responses/compact/" => Some(ApiRoute::Compact),
        "/v1/chat/completions" => Some(ApiRoute::ChatCompletions),
        "/v1/completions" => Some(ApiRoute::Completions),
        "/v1/messages" | "/v1/messages/" => Some(ApiRoute::Messages),
        "/v1/messages/count_tokens" | "/v1/messages/count_tokens/" => Some(ApiRoute::MessageTokens),
        "/v1beta/models" | "/v1beta/models/" => Some(ApiRoute::GeminiModels),
        _ => {
            if let Some(target) = match_gemini_action_path(pathname) {
                return Some(match target.action {
                    GeminiAction::GenerateContent => ApiRoute::GeminiGenerate {
                        model: target.model,
                        stream: false,
                    },
                    GeminiAction::StreamGenerateContent => ApiRoute::GeminiGenerate {
                        model: target.model,
                        stream: true,
                    },
                    GeminiAction::CountTokens => ApiRoute::GeminiTokens {
                        model: target.model,
                    },
                });
            }
            if let Some(model) = match_gemini_model_path(pathname) {
                return Some(ApiRoute::GeminiModel { model });
            }
            is_codex_proxy_path(pathname).then_some(ApiRoute::Proxy)
        }
    }
}

/// Management API routes below `/admin/api/`; all require an admin session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminRoute {
    State,
    CodexAccountSubscription,
    Usage,
    UsageAnalytics,
    UsageEvents,
    UsageLastSeen,
    /// Server-Sent Events: live connections and change signals.
    Live,
    Quota,
    NotificationTest,
    SettingsGet,
    SettingsUpdate,
    PricingGet,
    PricingUpdate,
    PricingSync,
    CodexAccountUpdate,
    CodexAccountDelete,
    CodexAccountImport,
    CodexAccountOAuthStart,
    CodexAccountOAuthPoll,
    AccountRoutingGet,
    AccountRoutingUpdate,
    UsersGet,
    UsersCreate,
    UsersUpdate,
    UsersDelete,
}

/// Every request below `/admin` is answered locally; it never reaches the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleRoute {
    /// `GET|HEAD /admin`: the single-page application shell.
    Page,
    /// `GET|HEAD /admin/`: canonicalized to `/admin`.
    PageRedirect,
    /// `/admin/assets/*`: built frontend files.
    Asset,
    /// `GET /admin/api/session`: the current session, if any.
    SessionGet,
    /// `POST /admin/api/session`: sign in with any supported credential.
    SessionCreate,
    /// `DELETE /admin/api/session`: sign out.
    SessionDelete,
    /// `GET /admin/api/account`: usage of the signed-in API key or account.
    Account,
    /// `GET /admin/api/account/analytics`: analytics limited to the signed-in identity.
    AccountAnalytics,
    /// `GET /admin/api/account/events`: request log of the signed-in identity.
    AccountEvents,
    /// `GET /admin/api/account/quota`: quota forecasts of the identity's route targets.
    AccountQuota,
    Admin(AdminRoute),
    /// Unknown path or unsupported method inside the console namespace.
    NotFound,
}

pub const CONSOLE_BASE_PATH: &str = "/admin";
const CONSOLE_API_PREFIX: &str = "/admin/api/";

pub fn is_console_path(pathname: &str) -> bool {
    pathname == CONSOLE_BASE_PATH
        || pathname
            .strip_prefix(CONSOLE_BASE_PATH)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Matches a console request. Returns `None` only outside the `/admin` namespace.
pub fn match_console_route(method: &str, pathname: &str) -> Option<ConsoleRoute> {
    if !is_console_path(pathname) {
        return None;
    }
    let page_method = matches!(method, "GET" | "HEAD");
    if pathname == CONSOLE_BASE_PATH {
        return Some(if page_method {
            ConsoleRoute::Page
        } else {
            ConsoleRoute::NotFound
        });
    }
    if pathname == "/admin/" {
        return Some(if page_method {
            ConsoleRoute::PageRedirect
        } else {
            ConsoleRoute::NotFound
        });
    }
    if pathname.starts_with("/admin/assets/") {
        return Some(ConsoleRoute::Asset);
    }
    let Some(relative) = pathname.strip_prefix(CONSOLE_API_PREFIX) else {
        return Some(ConsoleRoute::NotFound);
    };
    let route = match (method, relative) {
        ("GET", "session") => ConsoleRoute::SessionGet,
        ("POST", "session") => ConsoleRoute::SessionCreate,
        ("DELETE", "session") => ConsoleRoute::SessionDelete,
        ("GET", "account") => ConsoleRoute::Account,
        ("GET", "account/analytics") => ConsoleRoute::AccountAnalytics,
        ("GET", "account/events") => ConsoleRoute::AccountEvents,
        ("GET", "account/quota") => ConsoleRoute::AccountQuota,
        _ => match_admin_api(method, relative).map_or(ConsoleRoute::NotFound, ConsoleRoute::Admin),
    };
    Some(route)
}

fn match_admin_api(method: &str, relative: &str) -> Option<AdminRoute> {
    Some(match (method, relative) {
        ("GET", "state") => AdminRoute::State,
        ("GET", "codex-accounts/subscription") => AdminRoute::CodexAccountSubscription,
        ("GET", "usage") => AdminRoute::Usage,
        ("GET", "usage/analytics") => AdminRoute::UsageAnalytics,
        ("GET", "usage/events") => AdminRoute::UsageEvents,
        ("GET", "usage/last-seen") => AdminRoute::UsageLastSeen,
        ("GET", "live") => AdminRoute::Live,
        ("GET", "quota") => AdminRoute::Quota,
        ("POST", "settings/notifications/test") => AdminRoute::NotificationTest,
        ("GET", "settings") => AdminRoute::SettingsGet,
        ("PUT", "settings") => AdminRoute::SettingsUpdate,
        ("GET", "pricing") => AdminRoute::PricingGet,
        ("PUT", "pricing") => AdminRoute::PricingUpdate,
        ("POST", "pricing/sync") => AdminRoute::PricingSync,
        ("PUT", "codex-accounts") => AdminRoute::CodexAccountUpdate,
        ("DELETE", "codex-accounts") => AdminRoute::CodexAccountDelete,
        ("POST", "codex-accounts/import") => AdminRoute::CodexAccountImport,
        ("POST", "codex-accounts/oauth/device") => AdminRoute::CodexAccountOAuthStart,
        ("POST", "codex-accounts/oauth/device/poll") => AdminRoute::CodexAccountOAuthPoll,
        ("GET", "account-routing") => AdminRoute::AccountRoutingGet,
        ("PUT", "account-routing") => AdminRoute::AccountRoutingUpdate,
        ("GET", "users") => AdminRoute::UsersGet,
        ("POST", "users") => AdminRoute::UsersCreate,
        ("PUT", "users") => AdminRoute::UsersUpdate,
        ("DELETE", "users") => AdminRoute::UsersDelete,
        _ => return None,
    })
}

/// The deprecated `/<admin.path>/admin` page path, when `admin.path` is still configured.
pub fn legacy_admin_base_path(configured_path: Option<&str>) -> Option<String> {
    let configured_path = configured_path?.trim();
    valid_route_id(configured_path).then(|| format!("/{configured_path}/admin"))
}

pub fn is_legacy_admin_path_family(pathname: &str, base_path: &str) -> bool {
    pathname == base_path
        || pathname
            .strip_prefix(base_path)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn valid_route_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_method_mismatches_but_recognizes_options_paths() {
        let url = Url::parse("https://worker.example/v1/chat/completions").unwrap();
        assert_eq!(
            match_api_route("POST", &url, false),
            Some(ApiRoute::ChatCompletions)
        );
        assert_eq!(match_api_route("GET", &url, false), None);
        assert!(is_known_api_path(url.path()));
    }

    #[test]
    fn image_creation_routes_use_adaptation_and_other_image_paths_remain_proxies() {
        for operation in [ImageOperation::Generate, ImageOperation::Edit] {
            for suffix in ["", "/"] {
                let url = Url::parse(&format!(
                    "https://worker.example{}{suffix}",
                    operation.path()
                ))
                .unwrap();
                assert_eq!(
                    match_api_route("POST", &url, false),
                    Some(ApiRoute::Images(operation))
                );
                assert_eq!(match_api_route("GET", &url, false), None);
                assert!(is_known_api_path(url.path()));
            }
        }
        let url = Url::parse("https://worker.example/v1/images/variations").unwrap();
        assert_eq!(match_api_route("POST", &url, false), Some(ApiRoute::Proxy));
    }

    #[test]
    fn captures_gemini_model_and_stream_action() {
        let url =
            Url::parse("https://worker.example/v1beta/models/gemini%2Dtest:streamGenerateContent")
                .unwrap();
        assert_eq!(
            match_api_route("POST", &url, false),
            Some(ApiRoute::GeminiGenerate {
                model: "gemini-test".into(),
                stream: true,
            })
        );
    }

    #[test]
    fn console_namespace_is_strictly_local() {
        use ConsoleRoute::*;
        assert_eq!(match_console_route("GET", "/admin"), Some(Page));
        assert_eq!(match_console_route("HEAD", "/admin"), Some(Page));
        assert_eq!(match_console_route("POST", "/admin"), Some(NotFound));
        assert_eq!(match_console_route("GET", "/admin/"), Some(PageRedirect));
        assert_eq!(
            match_console_route("GET", "/admin/assets/a.js"),
            Some(Asset)
        );
        assert_eq!(
            match_console_route("GET", "/admin/api/session"),
            Some(SessionGet)
        );
        assert_eq!(
            match_console_route("POST", "/admin/api/session"),
            Some(SessionCreate)
        );
        assert_eq!(
            match_console_route("DELETE", "/admin/api/session"),
            Some(SessionDelete)
        );
        assert_eq!(
            match_console_route("GET", "/admin/api/account"),
            Some(Account)
        );
        assert_eq!(
            match_console_route("PUT", "/admin/api/settings"),
            Some(Admin(AdminRoute::SettingsUpdate))
        );
        assert_eq!(
            match_console_route("POST", "/admin/api/codex-accounts/oauth/device"),
            Some(Admin(AdminRoute::CodexAccountOAuthStart))
        );
        for (method, path) in [
            ("GET", "/admin/api/unknown"),
            ("GET", "/admin/api/session/"),
            ("PATCH", "/admin/api/settings"),
            ("GET", "/admin/other"),
            ("GET", "/admin/api"),
        ] {
            assert_eq!(
                match_console_route(method, path),
                Some(NotFound),
                "{method} {path}"
            );
        }
        assert_eq!(match_console_route("GET", "/administrator"), None);
        assert_eq!(match_console_route("GET", "/v1/models"), None);
    }

    #[test]
    fn legacy_admin_path_is_bounded() {
        assert_eq!(
            legacy_admin_base_path(Some("secret")).as_deref(),
            Some("/secret/admin")
        );
        assert_eq!(legacy_admin_base_path(Some("bad/path")), None);
        assert_eq!(legacy_admin_base_path(None), None);
        assert!(is_legacy_admin_path_family(
            "/secret/admin/state",
            "/secret/admin"
        ));
        assert!(!is_legacy_admin_path_family(
            "/secret/administrator",
            "/secret/admin"
        ));
    }
}
