use axum::{body::Body, http::HeaderMap};
use futures_util::StreamExt;

use crate::{
    core::{ApiError, AppResult, JsonObject},
    http::{LimitedBodyCollector, parse_json_body},
};

pub const MAX_JSON_BODY_BYTES: usize = 100 * 1024 * 1024;

pub async fn request_json(headers: &HeaderMap, body: Body) -> AppResult<JsonObject> {
    let content_encoding = headers
        .get("content-encoding")
        .and_then(|value| value.to_str().ok());
    let bytes = read_limited_body(headers, body, MAX_JSON_BODY_BYTES).await?;
    parse_json_body(&bytes, content_encoding)
}

pub async fn read_limited_body(
    headers: &HeaderMap,
    body: Body,
    max_bytes: usize,
) -> AppResult<Vec<u8>> {
    let declared_length = headers
        .get("content-length")
        .and_then(|value| value.to_str().ok());
    let mut collector =
        LimitedBodyCollector::new(max_bytes, declared_length).map_err(|_| request_too_large())?;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| body_unavailable())?;
        collector
            .push_chunk(&chunk)
            .map_err(|_| request_too_large())?;
    }
    Ok(collector.finish())
}

pub async fn read_limited_response(
    response: reqwest::Response,
    max_bytes: usize,
) -> AppResult<Vec<u8>> {
    let declared_length = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok());
    let mut collector = LimitedBodyCollector::new(max_bytes, declared_length)
        .map_err(|_| response_unavailable())?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| response_unavailable())?;
        collector
            .push_chunk(&chunk)
            .map_err(|_| response_unavailable())?;
    }
    Ok(collector.finish())
}

fn body_unavailable() -> ApiError {
    ApiError::new(400, "The request body could not be read.")
        .with_kind("invalid_request_error")
        .with_code("invalid_request_body")
}

fn request_too_large() -> ApiError {
    ApiError::new(413, "The request body is too large.")
        .with_kind("invalid_request_error")
        .with_code("request_too_large")
}

fn response_unavailable() -> ApiError {
    ApiError::new(502, "The Codex backend returned an invalid response.")
        .with_kind("upstream_error")
        .with_code("invalid_codex_response")
}
