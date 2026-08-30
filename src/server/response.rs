use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use futures_util::StreamExt;
use serde::Serialize;

use crate::{
    core::{ApiError, AppResult},
    http::{self, HeadersDto, ResponseBodyDto, ResponseDto},
};

use super::usage::UsageTracker;

pub fn empty(status: u16) -> Response {
    from_dto(http::empty_response(status)).unwrap_or_else(|_| internal_server_error())
}

pub fn json<T: Serialize + ?Sized>(value: &T, status: u16) -> AppResult<Response> {
    from_dto(http::json_response(value, status)?)
}

pub fn api_error(error: &ApiError) -> Response {
    json(&error.openai_payload(), error.status).unwrap_or_else(|_| internal_server_error())
}

pub fn with_cors(response: Response, origin: &str) -> Response {
    map_response_head(response, |head| http::with_cors(head, origin))
}

pub fn upstream_json(response: reqwest::Response) -> Response {
    upstream(response, http::upstream_json_response)
}

pub fn upstream_error(response: reqwest::Response) -> Response {
    upstream(response, http::upstream_error_response)
}

pub fn upstream_proxy(response: reqwest::Response) -> Response {
    upstream(response, http::upstream_proxy_response)
}

pub fn upstream_proxy_tracked(response: reqwest::Response, tracker: UsageTracker) -> Response {
    let status = response.status().as_u16();
    let headers = headers_dto(response.headers());
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let policy = http::upstream_proxy_response(ResponseDto {
        status,
        headers,
        body: ResponseBodyDto::Passthrough,
    });
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
    build_response(policy.status, &policy.headers, Body::from_stream(output))
        .unwrap_or_else(|_| internal_server_error())
}

pub fn suppress_html_body(response: Response) -> Response {
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        || !response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(http::is_html_content_type)
    {
        return response;
    }
    let (parts, _) = response.into_parts();
    let policy = http::suppress_html_body(ResponseDto {
        status: parts.status.as_u16(),
        headers: headers_dto(&parts.headers),
        body: ResponseBodyDto::Passthrough,
    });
    build_response(policy.status, &policy.headers, Body::empty())
        .unwrap_or_else(|_| internal_server_error())
}

pub fn headers_dto(source: &HeaderMap) -> HeadersDto {
    let mut output = HeadersDto::new();
    for (name, value) in source {
        if let Ok(value) = value.to_str() {
            output.append(name.as_str(), value);
        }
    }
    output
}

pub fn header_map(source: &HeadersDto) -> AppResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in source.iter() {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid_headers())?;
        let value = HeaderValue::from_str(value).map_err(|_| invalid_headers())?;
        headers.append(name, value);
    }
    Ok(headers)
}

pub fn from_dto(response: ResponseDto) -> AppResult<Response> {
    let body = match response.body {
        ResponseBodyDto::Empty => Body::empty(),
        ResponseBodyDto::Bytes(bytes) => Body::from(bytes),
        ResponseBodyDto::Passthrough => return Err(invalid_response()),
    };
    build_response(response.status, &response.headers, body)
}

fn upstream(
    response: reqwest::Response,
    policy: impl FnOnce(ResponseDto) -> ResponseDto,
) -> Response {
    let status = response.status().as_u16();
    let headers = headers_dto(response.headers());
    let policy = policy(ResponseDto {
        status,
        headers,
        body: ResponseBodyDto::Passthrough,
    });
    let body = Body::from_stream(response.bytes_stream());
    build_response(policy.status, &policy.headers, body).unwrap_or_else(|_| internal_server_error())
}

fn map_response_head(
    response: Response,
    policy: impl FnOnce(ResponseDto) -> ResponseDto,
) -> Response {
    let (parts, body) = response.into_parts();
    let policy = policy(ResponseDto {
        status: parts.status.as_u16(),
        headers: headers_dto(&parts.headers),
        body: ResponseBodyDto::Passthrough,
    });
    build_response(policy.status, &policy.headers, body).unwrap_or_else(|_| internal_server_error())
}

fn build_response(status: u16, headers: &HeadersDto, body: Body) -> AppResult<Response> {
    let status = StatusCode::from_u16(status).map_err(|_| invalid_response())?;
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = header_map(headers)?;
    Ok(response)
}

fn internal_server_error() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response
}

fn invalid_headers() -> ApiError {
    ApiError::new(500, "The response headers are invalid.")
        .with_kind("internal_error")
        .with_code("invalid_response_headers")
}

fn invalid_response() -> ApiError {
    ApiError::new(500, "The response could not be created.")
        .with_kind("internal_error")
        .with_code("invalid_response")
}
