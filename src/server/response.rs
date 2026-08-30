use axum::{
    body::Body,
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
            CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE,
        },
    },
    response::Response,
};
use futures_util::StreamExt;
use serde::Serialize;

use crate::core::{ApiError, AppResult};

use super::usage::UsageTracker;

const CORS_ALLOWED_HEADERS: &[&str] = &[
    "Authorization",
    "Content-Type",
    "Range",
    "X-Api-Key",
    "X-Goog-Api-Key",
    "X-Goog-Api-Client",
    "X-Goog-User-Project",
    "Idempotency-Key",
    "Version",
    "OpenAI-Alpha",
    "OpenAI-Beta",
    "OpenAI-Organization",
    "OpenAI-Project",
    "Anthropic-Version",
    "Anthropic-Beta",
    "Anthropic-Dangerous-Direct-Browser-Access",
    "Session-Id",
    "Thread-Id",
    "Last-Event-ID",
    "X-Client-Request-Id",
    "X-Codex-Beta-Features",
    "X-Codex-Turn-Metadata",
    "X-Codex-Turn-State",
    "X-Oai-Attestation",
    "X-Stainless-Arch",
    "X-Stainless-Helper-Method",
    "X-Stainless-Lang",
    "X-Stainless-OS",
    "X-Stainless-Package-Version",
    "X-Stainless-Retry-Count",
    "X-Stainless-Runtime",
    "X-Stainless-Runtime-Version",
    "X-Stainless-Timeout",
];

const CORS_EXPOSED_HEADERS: &[&str] = &[
    "Accept-Ranges",
    "Anthropic-Ratelimit-Input-Tokens-Limit",
    "Anthropic-Ratelimit-Input-Tokens-Remaining",
    "Anthropic-Ratelimit-Input-Tokens-Reset",
    "Anthropic-Ratelimit-Output-Tokens-Limit",
    "Anthropic-Ratelimit-Output-Tokens-Remaining",
    "Anthropic-Ratelimit-Output-Tokens-Reset",
    "Anthropic-Ratelimit-Requests-Limit",
    "Anthropic-Ratelimit-Requests-Remaining",
    "Anthropic-Ratelimit-Requests-Reset",
    "Anthropic-Ratelimit-Tokens-Limit",
    "Anthropic-Ratelimit-Tokens-Remaining",
    "Anthropic-Ratelimit-Tokens-Reset",
    "Content-Disposition",
    "Content-Length",
    "Content-Range",
    "ETag",
    "Location",
    "OpenAI-Processing-Ms",
    "OpenAI-Request-Id",
    "OpenAI-Version",
    "Request-Id",
    "Retry-After",
    "X-Codex-Turn-State",
    "X-Goog-Request-Id",
    "X-Ratelimit-Limit-Requests",
    "X-Ratelimit-Limit-Tokens",
    "X-Ratelimit-Remaining-Requests",
    "X-Ratelimit-Remaining-Tokens",
    "X-Ratelimit-Reset-Requests",
    "X-Ratelimit-Reset-Tokens",
    "X-Request-Id",
];

const BLOCKED_PROXY_RESPONSE_HEADERS: &[&str] = &[
    "alt-svc",
    "clear-site-data",
    "connection",
    "keep-alive",
    "nel",
    "proxy-authenticate",
    "proxy-authorization",
    "report-to",
    "sec-websocket-accept",
    "sec-websocket-extensions",
    "server",
    "set-cookie",
    "set-cookie2",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub fn empty(status: u16) -> Response {
    response(status, HeaderMap::new(), Body::empty())
}

pub fn json<T: Serialize + ?Sized>(value: &T, status: u16) -> AppResult<Response> {
    let bytes = serde_json::to_vec(value).map_err(json_serialization_error)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response(status, headers, Body::from(bytes)))
}

pub fn api_error(error: &ApiError) -> Response {
    json(&error.openai_payload(), error.status).unwrap_or_else(|_| empty(500))
}

pub fn with_cors(mut response: Response, origin: &str) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(origin).expect("server.cors_origin is validated at startup"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        header_list(CORS_ALLOWED_HEADERS),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS"),
    );
    headers.insert(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        header_list(CORS_EXPOSED_HEADERS),
    );
    headers.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));
    response
}

pub fn upstream_json(response: reqwest::Response) -> Response {
    upstream(response, json_headers)
}

pub fn upstream_error(response: reqwest::Response) -> Response {
    upstream(response, error_headers)
}

pub fn upstream_proxy(response: reqwest::Response) -> Response {
    upstream(response, proxy_headers)
}

pub fn upstream_proxy_tracked(response: reqwest::Response, tracker: UsageTracker) -> Response {
    let status = response.status();
    let headers = proxy_headers(response.headers());
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut source = response.bytes_stream();
    let mut observer = tracker.wire_observer(content_type.as_deref());
    let output = async_stream::stream! {
        while let Some(chunk) = source.next().await {
            if let Ok(bytes) = &chunk {
                observer.push(bytes);
            }
            yield chunk;
        }
        observer.finish();
    };
    response_with_status(status, headers, Body::from_stream(output))
}

pub fn suppress_html_body(response: Response) -> Response {
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        || !response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(is_html_content_type)
    {
        return response;
    }
    let (mut parts, _) = response.into_parts();
    parts.headers.remove(CONTENT_LENGTH);
    parts.headers.remove(CONTENT_ENCODING);
    Response::from_parts(parts, Body::empty())
}

pub(crate) fn proxy_headers(source: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in source {
        if !blocked_proxy_response_header(name) {
            headers.append(name, value.clone());
        }
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

fn json_headers(source: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        source
            .get(CONTENT_TYPE)
            .cloned()
            .unwrap_or_else(|| HeaderValue::from_static("application/json; charset=utf-8")),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    copy_header(source, &mut headers, "x-codex-turn-state");
    headers
}

fn error_headers(source: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    for name in [
        "content-type",
        "retry-after",
        "x-request-id",
        "openai-request-id",
        "x-codex-turn-state",
    ] {
        copy_header(source, &mut headers, name);
    }
    headers
}

fn copy_header(source: &HeaderMap, target: &mut HeaderMap, name: &'static str) {
    let name = HeaderName::from_static(name);
    if let Some(value) = source.get(&name) {
        target.insert(name, value.clone());
    }
}

fn blocked_proxy_response_header(name: &HeaderName) -> bool {
    let name = name.as_str();
    name.starts_with("cf-") || BLOCKED_PROXY_RESPONSE_HEADERS.contains(&name)
}

fn is_html_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|media_type| {
            media_type.eq_ignore_ascii_case("text/html")
                || media_type.eq_ignore_ascii_case("application/xhtml+xml")
        })
}

fn header_list(values: &[&str]) -> HeaderValue {
    HeaderValue::from_str(&values.join(", ")).expect("static header names form a valid value")
}

fn upstream(upstream: reqwest::Response, policy: impl FnOnce(&HeaderMap) -> HeaderMap) -> Response {
    let status = upstream.status();
    let headers = policy(upstream.headers());
    response_with_status(status, headers, Body::from_stream(upstream.bytes_stream()))
}

fn response(status: u16, headers: HeaderMap, body: Body) -> Response {
    let status = StatusCode::from_u16(status).expect("application response statuses are valid");
    response_with_status(status, headers, body)
}

fn response_with_status(status: StatusCode, headers: HeaderMap, body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

fn json_serialization_error(_: serde_json::Error) -> ApiError {
    ApiError::new(500, "Failed to serialize the JSON response.")
        .with_kind("internal_error")
        .with_code("json_serialization_error")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in values {
            headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        headers
    }

    #[test]
    fn filters_proxy_headers() {
        let filtered = proxy_headers(&headers(&[
            ("content-type", "application/json"),
            ("content-encoding", "zstd"),
            ("set-cookie", "secret=true"),
            ("cf-ray", "internal"),
            ("x-request-id", "req_123"),
        ]));
        assert!(!filtered.contains_key("set-cookie"));
        assert!(!filtered.contains_key("cf-ray"));
        assert_eq!(filtered.get("x-request-id").unwrap(), "req_123");
        assert_eq!(filtered.get(CACHE_CONTROL).unwrap(), "no-store");
    }

    #[test]
    fn suppresses_html_bodies() {
        let html = response(
            200,
            headers(&[
                ("content-type", "text/html; charset=utf-8"),
                ("content-length", "1024"),
                ("content-encoding", "gzip"),
            ]),
            Body::from("private"),
        );
        let html = suppress_html_body(html);
        assert!(!html.headers().contains_key(CONTENT_LENGTH));
        assert!(!html.headers().contains_key(CONTENT_ENCODING));

        let json = response(
            200,
            headers(&[("content-type", "application/json")]),
            Body::from("{}"),
        );
        assert_eq!(
            suppress_html_body(json).headers()[CONTENT_TYPE],
            "application/json"
        );
    }

    #[test]
    fn error_headers_use_the_public_allowlist() {
        let filtered = error_headers(&headers(&[
            ("content-type", "application/json"),
            ("retry-after", "2"),
            ("x-secret", "do-not-copy"),
            ("x-codex-turn-state", "turn-1"),
        ]));
        assert_eq!(filtered.get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(filtered.get("retry-after").unwrap(), "2");
        assert_eq!(filtered.get("x-codex-turn-state").unwrap(), "turn-1");
        assert!(!filtered.contains_key("x-secret"));
    }
}
