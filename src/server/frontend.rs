//! Serves the web console from a deployed frontend directory.
//!
//! `frontend.dir` points at a Vite output directory. `index.html` is read on
//! every page request and receives a fresh CSP nonce; fingerprinted files
//! under `assets/` are served directly from disk.

use std::{convert::Infallible, path::Path};

use axum::{
    body::Body,
    http::{HeaderValue, Method, Request, StatusCode, Uri, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use tokio::io::AsyncReadExt;
use tower::ServiceExt;
use tower_http::services::ServeDir;
use url::{Host, Url};

use crate::core::ApiError;

use super::response;

/// URL prefix of the fingerprinted Vite assets (`base: "/admin/"`).
pub const ASSET_PREFIX: &str = "/admin/assets/";

const CSP_NONCE_PLACEHOLDER: &str = "__CODEX_ROUTER_CSP_NONCE__";
const MAX_INDEX_BYTES: u64 = 1024 * 1024;

#[cfg(debug_assertions)]
const DEV_SERVER_ORIGIN: &str = "http://127.0.0.1:5173";
#[cfg(debug_assertions)]
const DEV_SERVER_WEBSOCKET_ORIGIN: &str = "ws://127.0.0.1:5173";
/// Set by the `cargo run` development supervisor on the backend child process.
#[cfg(debug_assertions)]
const DEV_CHILD_ENV: &str = "CODEX_ROUTER_DEV_CHILD";

/// Serves the single console application shell with a per-request CSP nonce.
pub async fn application_page(method: &Method, frontend_dir: &Path, client_url: &Url) -> Response {
    let nonce = URL_SAFE_NO_PAD.encode(uuid::Uuid::new_v4().as_bytes());

    #[cfg(debug_assertions)]
    if std::env::var_os(DEV_CHILD_ENV).is_some() {
        return page_response(
            method,
            development_page(&nonce),
            dev_csp(&nonce),
            client_url,
        );
    }

    match read_index(frontend_dir, &nonce).await {
        Ok(html) => page_response(method, html, production_csp(&nonce), client_url),
        Err(error) => {
            tracing::error!(
                event = "frontend_index",
                status = "unavailable",
                dir = %frontend_dir.display(),
                error = %error.message,
            );
            response::api_error(&error)
        }
    }
}

/// Serves a file below `<frontend.dir>/assets` for a request under [`ASSET_PREFIX`].
pub async fn asset_response(request: Request<Body>, frontend_dir: &Path) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return uncacheable(response::empty(405));
    }
    let Some(relative) = request.uri().path().strip_prefix(ASSET_PREFIX) else {
        return uncacheable(response::empty(404));
    };
    if relative.is_empty() || relative.ends_with('/') {
        return uncacheable(response::empty(404));
    }
    let Ok(uri) = Uri::builder()
        .path_and_query(format!("/{relative}"))
        .build()
    else {
        return uncacheable(response::empty(404));
    };
    let (mut parts, body) = request.into_parts();
    parts.uri = uri;
    let request = Request::from_parts(parts, body);

    let service = ServeDir::new(frontend_dir.join("assets"))
        .append_index_html_on_directories(false)
        .precompressed_br()
        .precompressed_gzip();
    let Ok::<_, Infallible>(output) = service.oneshot(request).await;
    let (mut parts, body) = output.into_parts();
    let headers = &mut parts.headers;
    if parts.status.is_success() || parts.status == StatusCode::NOT_MODIFIED {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    } else {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    Response::from_parts(parts, Body::new(body))
}

fn uncacheable(mut output: Response) -> Response {
    output
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    output
}

async fn read_index(frontend_dir: &Path, nonce: &str) -> Result<String, ApiError> {
    let path = frontend_dir.join("index.html");
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| application_unavailable())?;
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| application_unavailable())?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(application_unavailable());
    }
    let template = String::from_utf8(bytes).map_err(|_| application_unavailable())?;
    if !template.contains(CSP_NONCE_PLACEHOLDER) {
        return Err(application_unavailable());
    }
    Ok(template.replace(CSP_NONCE_PLACEHOLDER, nonce))
}

fn production_csp(nonce: &str) -> String {
    format!(
        "default-src 'none'; script-src 'self' 'nonce-{nonce}'; style-src 'self' 'nonce-{nonce}'; connect-src 'self'; img-src 'self' data:; font-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
    )
}

#[cfg(debug_assertions)]
fn dev_csp(nonce: &str) -> String {
    format!(
        "default-src 'none'; script-src 'nonce-{nonce}' {DEV_SERVER_ORIGIN}; style-src 'nonce-{nonce}' {DEV_SERVER_ORIGIN}; connect-src 'self' {DEV_SERVER_ORIGIN} {DEV_SERVER_WEBSOCKET_ORIGIN}; img-src 'self' data:; font-src 'self' {DEV_SERVER_ORIGIN}; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
    )
}

fn page_response(
    method: &Method,
    html: String,
    content_security_policy: String,
    client_url: &Url,
) -> Response {
    let length = html.len();
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(html)
    };
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CONTENT_LENGTH, length)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_SECURITY_POLICY, content_security_policy)
        .header("cross-origin-resource-policy", "same-origin")
        .header(
            "permissions-policy",
            "camera=(), geolocation=(), microphone=()",
        )
        .header(header::REFERRER_POLICY, "same-origin")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header("x-frame-options", "DENY");
    if trustworthy_origin(client_url) {
        response = response.header("cross-origin-opener-policy", "same-origin");
    }
    response.body(body).unwrap_or_else(|_| response::empty(500))
}

fn trustworthy_origin(url: &Url) -> bool {
    url.scheme() == "https"
        || match url.host() {
            Some(Host::Domain(domain)) => {
                let host = domain.strip_suffix('.').unwrap_or(domain);
                host == "localhost" || host.ends_with(".localhost")
            }
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        }
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

fn application_unavailable() -> ApiError {
    ApiError::new(503, "The web console is not installed.")
        .with_kind("configuration_error")
        .with_code("application_unavailable")
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("codex-router-web-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        dir
    }

    #[test]
    fn development_shell_contains_vite_and_fresh_nonce_hooks() {
        let html = development_page("test-nonce");
        assert!(html.contains("http://127.0.0.1:5173/@vite/client"));
        assert!(html.contains("http://127.0.0.1:5173/@react-refresh"));
        assert!(html.contains("nonce=\"test-nonce\""));
        assert!(html.contains("/src/main.tsx"));
    }

    #[tokio::test]
    async fn serves_index_with_a_fresh_nonce_and_matching_csp() {
        let dir = temp_dir();
        std::fs::write(
            dir.join("index.html"),
            format!("<script nonce=\"{CSP_NONCE_PLACEHOLDER}\"></script>"),
        )
        .unwrap();
        let url = Url::parse("https://router.example/admin").unwrap();
        let output = application_page(&Method::GET, &dir, &url).await;
        assert_eq!(output.status(), StatusCode::OK);
        let csp = output.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(output.into_body(), 1024).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(!body.contains(CSP_NONCE_PLACEHOLDER));
        let nonce = body.split('"').nth(1).unwrap();
        assert!(csp.contains(&format!("'nonce-{nonce}'")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn missing_or_invalid_index_is_a_local_503() {
        let dir = temp_dir();
        let url = Url::parse("https://router.example/admin").unwrap();
        let missing = application_page(&Method::GET, &dir, &url).await;
        assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);
        std::fs::write(dir.join("index.html"), "<p>no placeholder</p>").unwrap();
        let invalid = application_page(&Method::GET, &dir, &url).await;
        assert_eq!(invalid.status(), StatusCode::SERVICE_UNAVAILABLE);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn serves_assets_without_escaping_the_asset_directory() {
        let dir = temp_dir();
        std::fs::write(dir.join("assets/app-abc.js"), "console.log(1)").unwrap();
        std::fs::write(dir.join("index.html"), "secret").unwrap();
        let request = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();

        let found = asset_response(request("/admin/assets/app-abc.js"), &dir).await;
        assert_eq!(found.status(), StatusCode::OK);
        assert!(
            found.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("immutable")
        );
        assert!(
            found.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .contains("javascript")
        );

        for uri in [
            "/admin/assets/missing.js",
            "/admin/assets/../index.html",
            "/admin/assets/%2e%2e/index.html",
            "/admin/assets/",
        ] {
            let output = asset_response(request(uri), &dir).await;
            assert_eq!(output.status(), StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(output.headers()[header::CACHE_CONTROL], "no-store");
        }

        let post = asset_response(
            Request::post("/admin/assets/app-abc.js")
                .body(Body::empty())
                .unwrap(),
            &dir,
        )
        .await;
        assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn opener_isolation_follows_the_console_origin() {
        for (origin, isolated) in [
            ("http://192.168.160.172:8317", false),
            ("http://router.example", false),
            ("https://router.example", true),
            ("http://localhost:8787", true),
            ("http://console.localhost:8787", true),
            ("http://127.0.0.2:8787", true),
            ("http://[::1]:8787", true),
        ] {
            let output = page_response(
                &Method::GET,
                String::new(),
                "default-src 'none'".into(),
                &Url::parse(origin).unwrap(),
            );
            assert_eq!(
                output
                    .headers()
                    .get("cross-origin-opener-policy")
                    .and_then(|value| value.to_str().ok()),
                isolated.then_some("same-origin"),
                "{origin}",
            );
            assert_eq!(
                output.headers()[header::CONTENT_SECURITY_POLICY],
                "default-src 'none'"
            );
        }
    }
}
