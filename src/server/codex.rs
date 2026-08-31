use std::{borrow::Cow, time::Duration};

use axum::{
    body::Body,
    extract::ws::WebSocketUpgrade,
    http::{HeaderMap, Method, Request},
    response::Response,
};
use bytes::Bytes;
use futures_util::stream;
use serde_json::Value;
use url::Url;

use crate::{
    auth::OAuthRepository,
    core::{ApiError, AppResult, JsonObject},
    http::parse_json_body_with_source,
    protocol::openai::{
        request_policy::apply_converted_response_egress_policy,
        responses::{adapt_compact_body, adapt_responses_create_body},
    },
    upstream::codex::{
        CODEX_USAGE_REQUEST_TIMEOUT_MS, CodexCredentials, CodexSubscriptionMetadata, HeaderBag,
        MAX_CODEX_USAGE_RESPONSE_BYTES, codex_headers, codex_subscription_metadata,
        codex_usage_unavailable, codex_usage_upstream_error, invalid_codex_usage_response,
        proxy_request_headers, resolve_codex_proxy_url, resolve_models_url, responses_url,
        usage_headers, usage_url,
    },
};

use super::{
    body, chatgpt_proxy::ChatgptTransport, oauth::current_time_ms, usage::UsageTracker, websocket,
};

const MAX_LIVE_BOOTSTRAP_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexProxyRoute {
    Responses,
    Compact,
    Proxy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexUsageDocument {
    pub payload: Value,
    pub metadata: CodexSubscriptionMetadata,
}

pub struct CodexClient<'repository, 'store> {
    oauth: &'repository OAuthRepository<'store>,
    transport: &'repository ChatgptTransport,
}

impl<'repository, 'store> CodexClient<'repository, 'store> {
    pub fn new(
        oauth: &'repository OAuthRepository<'store>,
        transport: &'repository ChatgptTransport,
    ) -> Self {
        Self { oauth, transport }
    }

    pub async fn fetch_models(
        &self,
        client_url: &Url,
        source_headers: &HeaderMap,
    ) -> AppResult<reqwest::Response> {
        let credentials = self.credentials().await?;
        let source = header_bag(source_headers);
        let target = resolve_models_url(client_url, Some(&source));
        let headers = codex_headers(&credentials, "application/json", Some(&source), false);
        self.send(&target, Method::GET, headers, None, true).await
    }

    pub async fn fetch_usage(&self) -> AppResult<CodexUsageDocument> {
        let stored = self.oauth.require_valid(current_time_ms()).await?;
        let metadata = codex_subscription_metadata(stored.id_token.as_deref());
        let credentials = CodexCredentials {
            token: stored.access_token,
            account_id: stored.account_id,
        };
        let target = usage_url();
        let response = self
            .request(&target, Method::GET, usage_headers(&credentials), None)
            .timeout(Duration::from_millis(CODEX_USAGE_REQUEST_TIMEOUT_MS))
            .send()
            .await
            .map_err(|_| codex_usage_unavailable())?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(codex_usage_upstream_error(status));
        }
        let bytes = body::read_limited_response(response, MAX_CODEX_USAGE_RESPONSE_BYTES)
            .await
            .map_err(|_| invalid_codex_usage_response())?;
        if bytes.is_empty() {
            return Err(invalid_codex_usage_response());
        }
        let payload = serde_json::from_slice(&bytes).map_err(|_| invalid_codex_usage_response())?;
        Ok(CodexUsageDocument { payload, metadata })
    }

    pub async fn send_converted_responses(
        &self,
        body: &JsonObject,
        source_headers: &HeaderMap,
    ) -> AppResult<reqwest::Response> {
        let credentials = self.credentials().await?;
        let source = header_bag(source_headers);
        let target = responses_url();
        let headers = codex_headers(&credentials, "text/event-stream", Some(&source), true);
        let adapted = apply_converted_response_egress_policy(body);
        let body = serde_json::to_vec(adapted.as_ref()).map_err(|_| json_serialization_error())?;
        self.send(
            &target,
            Method::POST,
            headers,
            Some(reqwest::Body::from(body)),
            true,
        )
        .await
    }

    pub async fn forward_proxy(
        &self,
        request: Request<Body>,
        client_url: &Url,
        route: CodexProxyRoute,
        tracker: Option<&UsageTracker>,
    ) -> AppResult<reqwest::Response> {
        let credentials = self.credentials().await?;
        let (parts, body) = request.into_parts();
        let source = header_bag(&parts.headers);
        let target = resolve_codex_proxy_url(client_url, parts.method.as_str());
        let prepared = prepare_proxy_body(
            body,
            &parts.headers,
            client_url,
            &parts.method,
            route,
            source,
        )
        .await?;
        if let (Some(tracker), Some(model)) = (tracker, prepared.requested_model.as_deref()) {
            tracker.set_requested_model(model);
        }
        let headers = proxy_request_headers(&prepared.headers, &credentials, target.path(), false);
        self.send(&target, parts.method, headers, prepared.body, false)
            .await
    }

    pub async fn forward_websocket(
        &self,
        upgrade: WebSocketUpgrade,
        client_url: &Url,
        method: &Method,
        source_headers: &HeaderMap,
        route: CodexProxyRoute,
        tracker: Option<UsageTracker>,
    ) -> AppResult<Response> {
        let credentials = self.credentials().await?;
        let source = header_bag(source_headers);
        let target = resolve_codex_proxy_url(client_url, method.as_str());
        let headers = proxy_request_headers(&source, &credentials, target.path(), true);
        websocket::proxy(
            upgrade,
            target,
            headers,
            route == CodexProxyRoute::Responses,
            tracker,
            self.transport.proxy(),
        )
        .await
    }

    async fn credentials(&self) -> AppResult<CodexCredentials> {
        let stored = self.oauth.require_valid(current_time_ms()).await?;
        Ok(CodexCredentials {
            token: stored.access_token,
            account_id: stored.account_id,
        })
    }

    async fn send(
        &self,
        target: &Url,
        method: Method,
        headers: HeaderBag,
        body: Option<reqwest::Body>,
        require_success_body: bool,
    ) -> AppResult<reqwest::Response> {
        let response = self
            .request(target, method, headers, body)
            .send()
            .await
            .map_err(|_| codex_unavailable())?;
        if require_success_body
            && response.status().is_success()
            && response.content_length() == Some(0)
        {
            return Err(empty_codex_response());
        }
        Ok(response)
    }

    fn request(
        &self,
        target: &Url,
        method: Method,
        headers: HeaderBag,
        body: Option<reqwest::Body>,
    ) -> reqwest::RequestBuilder {
        let mut request = self.transport.client().request(method, target.as_str());
        for (name, value) in headers.iter() {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        request
    }
}

struct PreparedProxyBody {
    headers: HeaderBag,
    body: Option<reqwest::Body>,
    requested_model: Option<String>,
}

async fn prepare_proxy_body(
    body: Body,
    source_headers: &HeaderMap,
    client_url: &Url,
    method: &Method,
    route: CodexProxyRoute,
    headers: HeaderBag,
) -> AppResult<PreparedProxyBody> {
    if method != Method::POST {
        return Ok(passthrough_body(body, method, headers));
    }
    match route {
        CodexProxyRoute::Responses => {
            adapt_json_body(body, source_headers, headers, adapt_responses_create_body).await
        }
        CodexProxyRoute::Compact => {
            adapt_json_body(body, source_headers, headers, adapt_compact_body).await
        }
        CodexProxyRoute::Proxy if is_live_multipart(client_url.path(), &headers) => {
            adapt_live_bootstrap(body, source_headers, headers).await
        }
        CodexProxyRoute::Proxy => Ok(passthrough_body(body, method, headers)),
    }
}

async fn adapt_json_body<'a>(
    body: Body,
    source_headers: &HeaderMap,
    headers: HeaderBag,
    adapt: impl for<'body> Fn(&'body JsonObject) -> Cow<'body, JsonObject>,
) -> AppResult<PreparedProxyBody> {
    let content_encoding = headers.get("content-encoding").map(str::to_owned);
    let encoded = body::read_limited_body(source_headers, body, body::MAX_JSON_BODY_BYTES).await?;
    let parsed = parse_json_body_with_source(encoded, content_encoding.as_deref())?;
    let requested_model = parsed
        .body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned);
    let adapted = adapt(&parsed.body);
    if matches!(adapted, Cow::Borrowed(_)) {
        return Ok(PreparedProxyBody {
            headers,
            body: Some(reqwest::Body::from(parsed.encoded_body)),
            requested_model,
        });
    }
    let bytes = serde_json::to_vec(adapted.as_ref()).map_err(|_| json_serialization_error())?;
    Ok(PreparedProxyBody {
        headers: json_headers(&headers),
        body: Some(reqwest::Body::from(bytes)),
        requested_model,
    })
}

async fn adapt_live_bootstrap(
    body: Body,
    source_headers: &HeaderMap,
    headers: HeaderBag,
) -> AppResult<PreparedProxyBody> {
    let content_type = headers
        .get("content-type")
        .ok_or_else(|| invalid_live_request("The live multipart body is invalid."))?;
    let boundary = multer::parse_boundary(content_type)
        .map_err(|_| invalid_live_request("The live multipart body is invalid."))?;
    let bytes = body::read_limited_body(source_headers, body, MAX_LIVE_BOOTSTRAP_BODY_BYTES)
        .await
        .map_err(|error| {
            if error.status == 413 {
                live_request_too_large()
            } else {
                invalid_live_request("The live multipart body is invalid.")
            }
        })?;
    if bytes.is_empty() {
        return Err(invalid_live_request("The live request body is empty."));
    }
    let source = stream::once(async move { Ok::<Bytes, std::io::Error>(Bytes::from(bytes)) });
    let mut multipart = multer::Multipart::new(source, boundary);
    let mut sdp = None;
    let mut session = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| invalid_live_request("The live multipart body is invalid."))?
    {
        let name = field.name().map(str::to_owned);
        let bytes = field
            .bytes()
            .await
            .map_err(|_| invalid_live_request("The live multipart body is invalid."))?;
        match name.as_deref() {
            Some("sdp") if sdp.is_none() => {
                let value = std::str::from_utf8(&bytes)
                    .map_err(|_| invalid_live_request("The live 'sdp' field must be UTF-8."))?;
                sdp = Some(value.to_owned());
            }
            Some("session") if session.is_none() => {
                let value = serde_json::from_slice(&bytes).map_err(|_| {
                    invalid_live_request("The live 'session' field must contain valid JSON.")
                })?;
                session = Some(value);
            }
            _ => {}
        }
    }
    let sdp = sdp
        .ok_or_else(|| invalid_live_request("The live multipart body requires an 'sdp' field."))?;
    let mut payload = JsonObject::new();
    payload.insert("sdp".into(), Value::String(sdp));
    if let Some(session) = session {
        payload.insert("session".into(), session);
    }
    let bytes = serde_json::to_vec(&payload).map_err(|_| json_serialization_error())?;
    Ok(PreparedProxyBody {
        headers: json_headers(&headers),
        body: Some(reqwest::Body::from(bytes)),
        requested_model: None,
    })
}

fn passthrough_body(body: Body, method: &Method, headers: HeaderBag) -> PreparedProxyBody {
    let body = (!matches!(*method, Method::GET | Method::HEAD))
        .then(|| reqwest::Body::wrap_stream(body.into_data_stream()));
    PreparedProxyBody {
        headers,
        body,
        requested_model: None,
    }
}

fn is_live_multipart(pathname: &str, headers: &HeaderBag) -> bool {
    matches!(pathname, "/v1/live" | "/v1/realtime/calls")
        && headers.get("content-type").is_some_and(|value| {
            value.split(';').next().is_some_and(|media_type| {
                media_type
                    .trim()
                    .eq_ignore_ascii_case("multipart/form-data")
            })
        })
}

fn json_headers(source: &HeaderBag) -> HeaderBag {
    let mut headers = HeaderBag::new();
    for (name, value) in source.iter() {
        if !matches!(name, "content-encoding" | "content-length" | "content-type") {
            headers.append(name, value);
        }
    }
    headers.set("content-type", "application/json");
    headers
}

pub fn header_bag(headers: &HeaderMap) -> HeaderBag {
    HeaderBag::from_pairs(headers.iter().filter_map(|(name, value)| {
        value
            .to_str()
            .ok()
            .map(|value| (name.as_str().to_owned(), value.to_owned()))
    }))
}

fn json_serialization_error() -> ApiError {
    ApiError::new(500, "The request could not be encoded.")
        .with_kind("internal_error")
        .with_code("json_serialization_failed")
}

fn codex_unavailable() -> ApiError {
    ApiError::new(502, "The Codex backend is unavailable.")
        .with_kind("upstream_error")
        .with_code("codex_unavailable")
}

fn empty_codex_response() -> ApiError {
    ApiError::new(502, "The Codex backend returned an empty response.")
        .with_kind("upstream_error")
        .with_code("empty_codex_response")
}

fn invalid_live_request(message: &str) -> ApiError {
    ApiError::new(400, message)
        .with_kind("invalid_request_error")
        .with_code("invalid_live_request")
}

fn live_request_too_large() -> ApiError {
    ApiError::new(413, "The live multipart body is too large.")
        .with_kind("invalid_request_error")
        .with_code("live_request_too_large")
}
