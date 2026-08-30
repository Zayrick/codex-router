use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::StreamExt;

use crate::{
    auth::{OAuthClock, OAuthHttpClient, OAuthHttpFailure, OAuthHttpRequest, OAuthHttpResponse},
    http::LimitedBodyCollector,
};

pub struct ReqwestOAuthHttpClient<'a> {
    client: &'a reqwest::Client,
}

impl<'a> ReqwestOAuthHttpClient<'a> {
    pub const fn new(client: &'a reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl OAuthHttpClient for ReqwestOAuthHttpClient<'_> {
    async fn execute(
        &self,
        request: OAuthHttpRequest,
    ) -> Result<OAuthHttpResponse, OAuthHttpFailure> {
        let mut outgoing = self
            .client
            .post(&request.url)
            .timeout(Duration::from_millis(request.timeout_ms))
            .body(request.body);
        for (name, value) in request.headers {
            outgoing = outgoing.header(&name, &value);
        }
        let response = outgoing.send().await.map_err(map_transport_error)?;
        let status = response.status().as_u16();
        let declared_length = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok());
        let mut collector = LimitedBodyCollector::new(request.max_response_bytes, declared_length)
            .map_err(|_| OAuthHttpFailure::ResponseTooLarge)?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_transport_error)?;
            collector
                .push_chunk(&chunk)
                .map_err(|_| OAuthHttpFailure::ResponseTooLarge)?;
        }
        Ok(OAuthHttpResponse {
            status,
            body: collector.finish(),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

#[async_trait]
impl OAuthClock for SystemClock {
    async fn now_ms(&self) -> i64 {
        current_time_ms()
    }

    async fn sleep_ms(&self, delay_ms: u64) {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

#[must_use]
pub fn current_time_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

fn map_transport_error(error: reqwest::Error) -> OAuthHttpFailure {
    if error.is_timeout() {
        OAuthHttpFailure::TimedOut
    } else {
        OAuthHttpFailure::Network
    }
}
