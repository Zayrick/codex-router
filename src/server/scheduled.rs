use std::time::Duration;

use futures_util::{StreamExt, stream};

use crate::{
    application::{PushNotification, evaluate_codex_usage, reset_watch_notification},
    auth::{ApiKeyRepository, OAuthProvider, OAuthRefreshService, OAuthRepository},
    core::{ApiError, AppResult},
    http::LimitedBodyCollector,
    upstream::{
        bark::{
            BARK_PUSH_REQUEST_TIMEOUT_MS, bark_push_payload, bark_push_unavailable,
            parse_bark_push_url,
        },
        codex::codex_subscription_from_usage,
        codex_resets::{
            CODEX_RESETS_MAX_RESPONSE_BYTES, CODEX_RESETS_REQUEST_TIMEOUT_MS,
            codex_resets_unavailable, parse_codex_reset_status,
        },
        dingtalk::{
            DINGTALK_MAX_RESPONSE_BYTES, DINGTALK_REQUEST_TIMEOUT_MS, DingTalkResponse,
            dingtalk_markdown_notification_payload, dingtalk_notification_payload,
            dingtalk_unavailable, signed_dingtalk_webhook,
        },
    },
};

use super::{
    codex::CodexClient,
    config::AppConfig,
    oauth::{ReqwestOAuthHttpClient, SystemClock, current_time_ms},
    state::AppState,
    usage_store::CodexUsageStateRepository,
};

const AUTH_PROXY_REFRESH_CONCURRENCY: usize = 4;

pub fn spawn(state: AppState, interval_seconds: u64) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let period = Duration::from_secs(interval_seconds);
        let start = tokio::time::Instant::now() + period;
        let mut interval = tokio::time::interval_at(start, period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            run_once(&state).await;
        }
    })
}

pub async fn run_once(state: &AppState) {
    let config = state.config.snapshot().await;
    let now_ms = current_time_ms();
    let oauth = OAuthRepository::new(state.config.as_ref());

    if let Err(error) = monitor_reset_watch(state, &config, now_ms).await {
        log_failure("scheduled_reset_watch", &error);
    }
    if let Err(error) = monitor_usage(state, &config, &oauth, now_ms).await {
        log_failure("scheduled_usage_monitor", &error);
    }

    let clock = SystemClock;
    let http = ReqwestOAuthHttpClient::new(&state.client);
    let provider = OAuthProvider::new(&http, &clock);
    let service = OAuthRefreshService::new(&oauth, &provider, &clock);
    if let Err(error) = service.refresh(Some(now_ms)).await {
        log_failure("scheduled_oauth_refresh", &error);
    }

    let accounts = match ApiKeyRepository::new(state.config.as_ref())
        .read_auth_proxy_accounts()
        .await
    {
        Ok(accounts) => accounts,
        Err(error) => {
            log_failure("scheduled_auth_proxy_oauth_refresh", &error);
            return;
        }
    };
    stream::iter(accounts)
        .for_each_concurrent(AUTH_PROXY_REFRESH_CONCURRENCY, |account| {
            let provider = &provider;
            let clock = &clock;
            async move {
                let oauth =
                    OAuthRepository::for_auth_proxy_account(state.config.as_ref(), &account.id);
                let service = OAuthRefreshService::new(&oauth, provider, clock);
                if let Err(error) = service.refresh(Some(now_ms)).await {
                    log_failure("scheduled_auth_proxy_oauth_refresh", &error);
                }
            }
        })
        .await;
}

async fn monitor_reset_watch(state: &AppState, config: &AppConfig, now_ms: i64) -> AppResult<()> {
    let response = state
        .client
        .get(&config.upstream.codex_resets_url)
        .timeout(Duration::from_millis(CODEX_RESETS_REQUEST_TIMEOUT_MS))
        .send()
        .await
        .map_err(|_| codex_resets_unavailable())?;
    if !response.status().is_success() {
        return Err(codex_resets_unavailable());
    }
    let body = bounded_bytes(response, CODEX_RESETS_MAX_RESPONSE_BYTES)
        .await
        .ok_or_else(codex_resets_unavailable)?;
    let status = parse_codex_reset_status(&body)?;
    if let Some(notification) = reset_watch_notification(&status, now_ms) {
        deliver_notification(state, config, &notification, now_ms).await;
    }
    Ok(())
}

async fn monitor_usage(
    state: &AppState,
    config: &AppConfig,
    oauth: &OAuthRepository<'_>,
    now_ms: i64,
) -> AppResult<()> {
    let client = CodexClient::new(oauth, &state.chatgpt);
    let usage = client.fetch_usage().await?;
    let subscription =
        codex_subscription_from_usage(&usage.payload, usage.metadata, now_ms as f64)?;
    let repository = CodexUsageStateRepository::new(state.config.as_ref());
    let previous = repository.read().await?;
    let evaluation = evaluate_codex_usage(previous.as_ref(), &subscription, now_ms);
    if let Some(notification) = evaluation.notification() {
        deliver_notification(state, config, &notification, now_ms).await;
    }
    repository.store(&evaluation.state).await
}

async fn deliver_notification(
    state: &AppState,
    config: &AppConfig,
    notification: &PushNotification,
    now_ms: i64,
) {
    let bark = send_bark(state, config, notification);
    let dingtalk = send_dingtalk(state, config, notification, now_ms);
    let (bark_result, dingtalk_result) = tokio::join!(bark, dingtalk);
    if let Err(error) = bark_result {
        log_failure("scheduled_bark_push", &error);
    }
    if let Err(error) = dingtalk_result {
        log_failure("scheduled_dingtalk_push", &error);
    }
}

async fn send_bark(
    state: &AppState,
    config: &AppConfig,
    notification: &PushNotification,
) -> AppResult<()> {
    let Some(endpoint) = config.notifications.bark_push_url.as_deref() else {
        return Ok(());
    };
    let endpoint = parse_bark_push_url(endpoint)?;
    let response = state
        .client
        .post(endpoint)
        .timeout(Duration::from_millis(BARK_PUSH_REQUEST_TIMEOUT_MS))
        .json(&bark_push_payload(
            &notification.title,
            &notification.body,
            notification.url.as_deref(),
        ))
        .send()
        .await
        .map_err(|_| bark_push_unavailable())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(bark_push_unavailable())
    }
}

async fn send_dingtalk(
    state: &AppState,
    config: &AppConfig,
    notification: &PushNotification,
    now_ms: i64,
) -> AppResult<()> {
    let (Some(webhook), Some(secret)) = (
        config.notifications.dingtalk_webhook_url.as_deref(),
        config.notifications.dingtalk_secret.as_deref(),
    ) else {
        return Ok(());
    };
    let target = signed_dingtalk_webhook(webhook, secret, now_ms)?;
    let payload = match notification.url.as_deref() {
        Some(url) => serde_json::to_value(dingtalk_markdown_notification_payload(
            &notification.title,
            &notification.body,
            url,
        )),
        None => serde_json::to_value(dingtalk_notification_payload(&notification.body)),
    }
    .map_err(|_| dingtalk_unavailable())?;
    let response = state
        .client
        .post(target)
        .timeout(Duration::from_millis(DINGTALK_REQUEST_TIMEOUT_MS))
        .json(&payload)
        .send()
        .await
        .map_err(|_| dingtalk_unavailable())?;
    if !response.status().is_success() {
        return Err(dingtalk_unavailable());
    }
    let body = bounded_bytes(response, DINGTALK_MAX_RESPONSE_BYTES)
        .await
        .ok_or_else(dingtalk_unavailable)?;
    let result: DingTalkResponse =
        serde_json::from_slice(&body).map_err(|_| dingtalk_unavailable())?;
    if result.is_success() {
        Ok(())
    } else {
        Err(dingtalk_unavailable())
    }
}

async fn bounded_bytes(response: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
    let declared_length = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok());
    let mut collector = LimitedBodyCollector::new(limit, declared_length).ok()?;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.ok()?;
        collector.push_chunk(&chunk).ok()?;
    }
    Some(collector.finish())
}

fn log_failure(event: &str, error: &ApiError) {
    tracing::error!(
        event,
        status = "failed",
        code = error.code.as_deref().unwrap_or("scheduled_task_failed")
    );
}
