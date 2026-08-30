use axum::{
    Router,
    body::Body,
    extract::{FromRequestParts, State, ws::WebSocketUpgrade},
    http::Request,
    response::Response,
};
use tower_http::trace::TraceLayer;
use url::Url;

use crate::{
    application::{
        AdminRoute, StatusRoute, is_admin_path_family, is_known_api_path, match_admin_route,
        match_api_route, match_status_route,
    },
    auth::{ApiKeyRepository, OAuthRepository, client_token},
    upstream::relay::is_backend_api_path,
};

use super::{
    admin::handle_admin, api::handle_api, frontend, oauth::current_time_ms, relay::handle_relay,
    response, state::AppState, status::usage_snapshot,
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
    let config = state.config.snapshot().await;
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

    if let Some(response) = frontend::asset_response(request.method(), &path) {
        return response;
    }

    let output = if let Some(matched) = match_admin_route(&method, &path, &config.admin.path) {
        if matched.route == AdminRoute::Page {
            return frontend::application_page();
        }
        handle_admin(matched, request, client_url, &config, &state).await
    } else if is_admin_path_family(&path, &config.admin.path) {
        response::empty(404)
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
            let oauth = OAuthRepository::new(state.config.as_ref());
            match oauth.require_valid(current_time_ms()).await {
                Ok(_) => response::empty(204),
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
    } else if let Some(route) = match_status_route(&method, &path) {
        match route {
            StatusRoute::Page => return frontend::application_page(),
            StatusRoute::Usage => usage_snapshot(&state).await,
        }
    } else if matches!(path.as_str(), "/status/usage" | "/status/usage/data") {
        response::empty(404)
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
            let keys = ApiKeyRepository::new(state.config.as_ref());
            if keys.authenticate(token.as_deref()).await.is_err() {
                response::empty(404)
            } else {
                handle_api(route, request, client_url, websocket, &config, &state).await
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

    use crate::auth::ClientApiKey;

    use super::super::config::{
        AdminConfig, AppConfig, ConfigStore, NotificationConfig, PersistentState, ServerConfig,
        UpstreamConfig,
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
    async fn serves_local_apis_without_worker_bindings() {
        let path =
            std::env::temp_dir().join(format!("codex-router-test-{}.toml", uuid::Uuid::new_v4()));
        let config = AppConfig {
            server: ServerConfig {
                public_origin: "http://router.example".into(),
                ..ServerConfig::default()
            },
            admin: AdminConfig {
                path: "secret".into(),
                secret: "admin-secret".into(),
            },
            upstream: UpstreamConfig {
                chatgpt_relay_url: "https://relay.example".into(),
                codex_resets_url: "https://codex-resets.com/api/v1/status".into(),
            },
            notifications: NotificationConfig::default(),
            state: PersistentState {
                api_keys: vec![ClientApiKey {
                    id: "00000000-0000-4000-8000-000000000001".into(),
                    name: "test".into(),
                    key: "sk-test-value-123!".into(),
                    enabled: true,
                }],
                ..PersistentState::default()
            },
        };
        tokio::fs::write(&path, toml::to_string_pretty(&config).unwrap())
            .await
            .unwrap();
        let store = ConfigStore::load(path.clone()).await.unwrap();
        let app = build(AppState::new(store).unwrap());

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

        for page in ["/status/usage", "/secret/admin"] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(page).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                "text/html; charset=utf-8"
            );
            assert!(
                response
                    .headers()
                    .get("content-security-policy")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .contains("nonce-")
            );
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            assert!(
                body.windows(b"/@vite/client".len())
                    .any(|value| value == b"/@vite/client")
            );
        }

        let status = app
            .oneshot(
                Request::builder()
                    .uri("/status/usage/data")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);

        let _ = tokio::fs::remove_file(path).await;
    }
}
