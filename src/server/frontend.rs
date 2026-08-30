use axum::{
    body::Body,
    http::{Method, StatusCode, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

use super::response;

const DEV_SERVER_ORIGIN: &str = "http://127.0.0.1:5173";
const DEV_SERVER_WEBSOCKET_ORIGIN: &str = "ws://127.0.0.1:5173";

#[cfg(not(debug_assertions))]
use crate::core::ApiError;
#[cfg(not(debug_assertions))]
use bytes::Bytes;
#[cfg(not(debug_assertions))]
use rust_embed::RustEmbed;
#[cfg(not(debug_assertions))]
use std::borrow::Cow;

#[cfg(not(debug_assertions))]
const CSP_NONCE_PLACEHOLDER: &str = "__CODEX_ROUTER_CSP_NONCE__";

#[cfg(not(debug_assertions))]
#[derive(RustEmbed)]
#[folder = "$CODEX_ROUTER_FRONTEND_DIST/"]
struct FrontendAssets;

/// Serves the shared admin/account application shell with a per-request CSP nonce.
pub fn application_page() -> Response {
    let nonce = URL_SAFE_NO_PAD.encode(uuid::Uuid::new_v4().as_bytes());

    #[cfg(debug_assertions)]
    let html = development_page(&nonce);

    #[cfg(not(debug_assertions))]
    let html = match embedded_index(&nonce) {
        Ok(html) => html,
        Err(error) => return response::api_error(&error),
    };

    page_response(html, &nonce)
}

/// Returns an embedded, fingerprinted Vite asset with immutable cache policy.
pub fn asset_response(method: &Method, path: &str) -> Option<Response> {
    if !path.starts_with("/assets/") {
        return None;
    }

    #[cfg(debug_assertions)]
    {
        let _ = method;
        Some(response::empty(404))
    }

    #[cfg(not(debug_assertions))]
    {
        let relative = path
            .strip_prefix('/')
            .expect("asset paths start with a slash");
        let asset = match FrontendAssets::get(relative) {
            Some(asset) => asset,
            None => return Some(response::empty(404)),
        };
        if !matches!(*method, Method::GET | Method::HEAD) {
            return Some(response::empty(404));
        }

        let content_type = mime_guess::from_path(relative)
            .first_raw()
            .unwrap_or("application/octet-stream");
        let content_length = asset.data.len();
        let body = if method == Method::HEAD {
            Body::empty()
        } else {
            let bytes = match asset.data {
                Cow::Borrowed(bytes) => Bytes::from_static(bytes),
                Cow::Owned(bytes) => Bytes::from(bytes),
            };
            Body::from(bytes)
        };
        Some(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, content_length)
                .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header("cross-origin-resource-policy", "same-origin")
                .body(body)
                .unwrap_or_else(|_| response::empty(500)),
        )
    }
}

fn page_response(html: String, nonce: &str) -> Response {
    let content_security_policy = if cfg!(debug_assertions) {
        format!(
            "default-src 'none'; script-src 'nonce-{nonce}' {DEV_SERVER_ORIGIN}; style-src 'nonce-{nonce}' {DEV_SERVER_ORIGIN}; connect-src 'self' {DEV_SERVER_ORIGIN} {DEV_SERVER_WEBSOCKET_ORIGIN}; img-src 'self' data:; font-src 'self' {DEV_SERVER_ORIGIN}; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
        )
    } else {
        format!(
            "default-src 'none'; script-src 'self' 'nonce-{nonce}'; style-src 'self' 'nonce-{nonce}'; connect-src 'self'; img-src 'self' data:; font-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
        )
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_SECURITY_POLICY, content_security_policy)
        .header("cross-origin-opener-policy", "same-origin")
        .header("cross-origin-resource-policy", "same-origin")
        .header(
            "permissions-policy",
            "camera=(), geolocation=(), microphone=()",
        )
        .header(header::REFERRER_POLICY, "same-origin")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header("x-frame-options", "DENY")
        .body(Body::from(html))
        .unwrap_or_else(|_| response::empty(500))
}

#[cfg(debug_assertions)]
fn development_page(nonce: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="zh-CN">
	<head>
		<meta charset="UTF-8" />
		<meta name="viewport" content="width=device-width, initial-scale=1.0" />
		<meta property="csp-nonce" nonce="{nonce}" />
		<meta name="description" content="Codex Router 的账户用量查询与管理面板。" />
		<title>Codex Router</title>
	</head>
	<body>
		<div id="root"></div>
		<script type="module" nonce="{nonce}">
			import RefreshRuntime from "{DEV_SERVER_ORIGIN}/@react-refresh";
			RefreshRuntime.injectIntoGlobalHook(window);
			window.$RefreshReg$ = () => {{}};
			window.$RefreshSig$ = () => (type) => type;
			window.__vite_plugin_react_preamble_installed__ = true;
		</script>
		<script type="module" nonce="{nonce}" src="{DEV_SERVER_ORIGIN}/@vite/client"></script>
		<script type="module" nonce="{nonce}" src="{DEV_SERVER_ORIGIN}/src/main.tsx"></script>
	</body>
</html>
"#
    )
}

#[cfg(not(debug_assertions))]
fn embedded_index(nonce: &str) -> Result<String, ApiError> {
    let asset = FrontendAssets::get("index.html").ok_or_else(application_unavailable)?;
    let template =
        std::str::from_utf8(asset.data.as_ref()).map_err(|_| application_unavailable())?;
    Ok(template.replace(CSP_NONCE_PLACEHOLDER, nonce))
}

#[cfg(not(debug_assertions))]
fn application_unavailable() -> ApiError {
    ApiError::new(500, "The web application is unavailable.")
        .with_kind("configuration_error")
        .with_code("application_unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_shell_contains_vite_and_fresh_nonce_hooks() {
        let html = development_page("test-nonce");
        assert!(html.contains("http://127.0.0.1:5173/@vite/client"));
        assert!(html.contains("http://127.0.0.1:5173/@react-refresh"));
        assert!(html.contains("nonce=\"test-nonce\""));
        assert!(html.contains("/src/main.tsx"));
    }

    #[test]
    fn development_asset_paths_never_fall_through_to_the_relay() {
        let response = asset_response(&Method::GET, "/assets/missing.js")
            .expect("asset path must be handled locally");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(asset_response(&Method::GET, "/not-an-asset.js").is_none());
    }
}
