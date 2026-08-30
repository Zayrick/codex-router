use axum::{
    http::{HeaderMap, HeaderName, HeaderValue},
    response::Response,
};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;

use crate::{
    http::LimitedBodyCollector,
    protocol::{anthropic, gemini},
};

use super::response;

const GEMINI_MAX_UPSTREAM_ERROR_BYTES: usize = 1024 * 1024;
const FORWARDED_ERROR_HEADERS: &[&str] = &[
    "Retry-After",
    "Request-Id",
    "X-Request-Id",
    "OpenAI-Request-Id",
    "X-Codex-Turn-State",
    "X-Goog-Request-Id",
];

pub async fn anthropic_upstream_error_response(upstream: reqwest::Response) -> Response {
    let status = upstream.status().as_u16();
    let source_headers = ForwardedHeaders::capture(upstream.headers());
    let request_id = source_headers.anthropic_request_id().map(str::to_owned);
    let body = read_bounded_body(upstream, anthropic::MAX_UPSTREAM_ERROR_BYTES)
        .await
        .unwrap_or_default();
    let payload = anthropic::anthropic_upstream_error_payload(status, &body, request_id.as_deref());
    json_error_response(status, &payload, &source_headers, request_id.as_deref())
}

pub async fn gemini_upstream_error_response(upstream: reqwest::Response) -> Response {
    let status = upstream.status().as_u16();
    let source_headers = ForwardedHeaders::capture(upstream.headers());
    let body = read_bounded_body(upstream, GEMINI_MAX_UPSTREAM_ERROR_BYTES).await;
    let parsed = body
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
    let error = gemini::gemini_upstream_error(status, parsed.as_ref());
    let payload = gemini::gemini_error_payload(&error);
    json_error_response(status, &payload, &source_headers, None)
}

#[derive(Debug, Default)]
struct ForwardedHeaders {
    values: Vec<(&'static str, String)>,
}

impl ForwardedHeaders {
    fn capture(source: &HeaderMap) -> Self {
        let values = FORWARDED_ERROR_HEADERS
            .iter()
            .filter_map(|&name| trimmed_header(source, name).map(|value| (name, value)))
            .collect();
        Self { values }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn anthropic_request_id(&self) -> Option<&str> {
        ["Request-Id", "X-Request-Id", "OpenAI-Request-Id"]
            .into_iter()
            .find_map(|name| self.get(name))
    }
}

fn trimmed_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn json_error_response<T: Serialize + ?Sized>(
    status: u16,
    payload: &T,
    source_headers: &ForwardedHeaders,
    canonical_request_id: Option<&str>,
) -> Response {
    let mut output =
        response::json(payload, status).unwrap_or_else(|error| response::api_error(&error));
    for (name, value) in &source_headers.values {
        insert_header(output.headers_mut(), name, value);
    }
    if let Some(request_id) = canonical_request_id {
        insert_header(output.headers_mut(), "Request-Id", request_id);
        insert_header(output.headers_mut(), "X-Request-Id", request_id);
    }
    output
}

async fn read_bounded_body(upstream: reqwest::Response, max_bytes: usize) -> Option<Vec<u8>> {
    let declared_length = trimmed_header(upstream.headers(), "Content-Length");
    let mut collector = LimitedBodyCollector::new(max_bytes, declared_length.as_deref()).ok()?;
    let mut source = upstream.bytes_stream();
    while let Some(chunk) = source.next().await {
        let chunk = chunk.ok()?;
        collector.push_chunk(&chunk).ok()?;
    }
    Some(collector.finish())
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) {
    if let (Ok(name), Ok(value)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(value),
    ) {
        headers.insert(name, value);
    }
}
