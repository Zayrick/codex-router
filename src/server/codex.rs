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
        images::ImageRequest,
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
    body,
    chatgpt_proxy::ChatgptTransport,
    oauth::current_time_ms,
    usage::{UsageRequest, UsageTracker},
    websocket,
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
        default_client_version: &str,
    ) -> AppResult<reqwest::Response> {
        let credentials = self.credentials().await?;
        let source = header_bag(source_headers);
        let target = resolve_models_url(client_url, Some(&source), default_client_version);
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
        tracker: &UsageTracker,
    ) -> AppResult<reqwest::Response> {
        let credentials = self.credentials().await?;
        let source = header_bag(source_headers);
        let target = responses_url();
        let headers = codex_headers(&credentials, "text/event-stream", Some(&source), true);
        let adapted = apply_converted_response_egress_policy(body);
        tracker.observe_request_object(adapted.as_ref());
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

    pub async fn send_images(
        &self,
        request: &ImageRequest,
        source_headers: &HeaderMap,
        client_url: &Url,
    ) -> AppResult<reqwest::Response> {
        let credentials = self.credentials().await?;
        let mut client_url = client_url.clone();
        client_url.set_path(request.operation.path());
        let target = resolve_codex_proxy_url(&client_url, "POST");
        let mut source = json_headers(&header_bag(source_headers));
        source.set(
            "accept",
            if request.body.get("stream").and_then(Value::as_bool) == Some(true) {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
        let headers = proxy_request_headers(&source, &credentials, target.path(), false);
        let bytes = serde_json::to_vec(&request.body).map_err(|_| json_serialization_error())?;
        self.send(
            &target,
            Method::POST,
            headers,
            Some(reqwest::Body::from(bytes)),
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
        let mut prepared = Self::prepare_proxy(request, client_url, route).await?;
        if let (Some(tracker), Some(request)) = (tracker, prepared.usage_request()) {
            tracker.record_request(request);
        }
        self.send_prepared_with(&credentials, &mut prepared).await
    }

    /// Reads and adapts a proxied request without choosing an account yet.
    pub async fn prepare_proxy(
        request: Request<Body>,
        client_url: &Url,
        route: CodexProxyRoute,
    ) -> AppResult<PreparedProxy> {
        let (parts, body) = request.into_parts();
        let source = header_bag(&parts.headers);
        let target = resolve_codex_proxy_url(client_url, parts.method.as_str());
        let body = prepare_proxy_body(
            body,
            &parts.headers,
            client_url,
            &parts.method,
            route,
            source,
        )
        .await?;
        Ok(PreparedProxy {
            method: parts.method,
            target,
            body,
        })
    }

    /// Sends a prepared request with this client's account. A replayable body can
    /// be sent again, e.g. through another account after an overload.
    pub async fn send_prepared(
        &self,
        prepared: &mut PreparedProxy,
    ) -> AppResult<reqwest::Response> {
        let credentials = self.credentials().await?;
        self.send_prepared_with(&credentials, prepared).await
    }

    async fn send_prepared_with(
        &self,
        credentials: &CodexCredentials,
        prepared: &mut PreparedProxy,
    ) -> AppResult<reqwest::Response> {
        let headers = proxy_request_headers(
            &prepared.body.headers,
            credentials,
            prepared.target.path(),
            false,
        );
        let body = match &prepared.body.replay {
            Some(bytes) => Some(reqwest::Body::from(bytes.clone())),
            None => prepared.body.body.take(),
        };
        self.send(
            &prepared.target,
            prepared.method.clone(),
            headers,
            body,
            false,
        )
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
        let credentials = self.credentials().await.inspect_err(|error| {
            if let Some(tracker) = &tracker {
                tracker.fail(error);
            }
        })?;
        let source = header_bag(source_headers);
        let target = resolve_codex_proxy_url(client_url, method.as_str());
        let headers = proxy_request_headers(&source, &credentials, target.path(), true);
        websocket::proxy(
            upgrade,
            target,
            headers,
            websocket::ResponsesOptions {
                adapt: route == CodexProxyRoute::Responses,
                // Reconnecting lets routing move the session to a healthier account.
                close_on_overload: route == CodexProxyRoute::Responses,
            },
            tracker,
            self.transport.proxy(),
            self.transport.websocket_idle_timeout(),
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

/// A proxied request read from the client, ready to send through any account.
pub struct PreparedProxy {
    method: Method,
    target: Url,
    body: PreparedProxyBody,
}

impl PreparedProxy {
    pub fn usage_request(&self) -> Option<UsageRequest> {
        self.body.usage_request.clone()
    }

    /// Whether the request can be sent again through another account.
    pub fn replayable(&self) -> bool {
        self.body.replay.is_some()
    }

    /// Drops state the previous account issued before replaying on another one.
    pub fn forget_account_state(&mut self) {
        self.body.headers.remove("x-codex-turn-state");
    }
}

struct PreparedProxyBody {
    headers: HeaderBag,
    body: Option<reqwest::Body>,
    /// Complete, self-contained JSON body that may be sent more than once.
    replay: Option<Bytes>,
    usage_request: Option<UsageRequest>,
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
    let adapted = adapt(&parsed.body);
    let usage_request = Some(UsageRequest::from_body(adapted.as_ref()));
    let (headers, bytes) = if matches!(adapted, Cow::Borrowed(_)) {
        (headers, Bytes::from(parsed.encoded_body))
    } else {
        let bytes = serde_json::to_vec(adapted.as_ref()).map_err(|_| json_serialization_error())?;
        (json_headers(&headers), Bytes::from(bytes))
    };
    // Uploaded files only resolve on the account that stored them.
    let self_contained = !references_file(&Value::Object(adapted.as_ref().clone()));
    let (body, replay) = if self_contained {
        (None, Some(bytes))
    } else {
        (Some(reqwest::Body::from(bytes)), None)
    };
    Ok(PreparedProxyBody {
        headers,
        body,
        replay,
        usage_request,
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
        replay: None,
        usage_request: None,
    })
}

/// Whether a request body refers to an uploaded file anywhere.
fn references_file(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| (key == "file_id" && !value.is_null()) || references_file(value)),
        Value::Array(items) => items.iter().any(references_file),
        _ => false,
    }
}

fn passthrough_body(body: Body, method: &Method, headers: HeaderBag) -> PreparedProxyBody {
    let body = (!matches!(*method, Method::GET | Method::HEAD))
        .then(|| reqwest::Body::wrap_stream(body.into_data_stream()));
    PreparedProxyBody {
        headers,
        body,
        replay: None,
        usage_request: None,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::references_file;

    #[test]
    fn finds_file_references_anywhere_in_a_request() {
        assert!(references_file(&json!({
            "input": [{"role": "user", "content": [{"type": "input_file", "file_id": "file-1"}]}]
        })));
        assert!(!references_file(&json!({
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "\"file_id\""}]}]
        })));
        assert!(!references_file(&json!({"input": [], "file_id": null})));
    }
}
