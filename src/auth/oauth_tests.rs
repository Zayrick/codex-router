use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use url::form_urlencoded;

use crate::core::AppResult;

use super::*;

const STATE_SIGNING_KEY: &str = "test-state-signing-key";
const NOW_MS: i64 = 1_800_000_000_000;

#[derive(Default)]
struct FakeHttp {
    responses: Mutex<VecDeque<Result<OAuthHttpResponse, OAuthHttpFailure>>>,
    requests: Mutex<Vec<OAuthHttpRequest>>,
}

impl FakeHttp {
    fn push(&self, response: Result<OAuthHttpResponse, OAuthHttpFailure>) {
        self.responses.lock().unwrap().push_back(response);
    }

    fn requests(&self) -> Vec<OAuthHttpRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn is_empty(&self) -> bool {
        self.responses.lock().unwrap().is_empty()
    }
}

#[async_trait]
impl OAuthHttpClient for FakeHttp {
    async fn execute(
        &self,
        request: OAuthHttpRequest,
    ) -> Result<OAuthHttpResponse, OAuthHttpFailure> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("missing fake OAuth response")
    }
}

struct FakeClock {
    now_ms: Mutex<i64>,
    sleeps: Mutex<Vec<u64>>,
}

impl FakeClock {
    fn new(now_ms: i64) -> Self {
        Self {
            now_ms: Mutex::new(now_ms),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    fn set(&self, now_ms: i64) {
        *self.now_ms.lock().unwrap() = now_ms;
    }
}

#[async_trait]
impl OAuthClock for FakeClock {
    async fn now_ms(&self) -> i64 {
        *self.now_ms.lock().unwrap()
    }

    async fn sleep_ms(&self, delay_ms: u64) {
        self.sleeps.lock().unwrap().push(delay_ms);
    }
}

#[derive(Default)]
struct MemoryStateStore {
    values: Mutex<HashMap<String, String>>,
}

impl MemoryStateStore {
    fn raw(&self, key: &str) -> Option<String> {
        self.values.lock().unwrap().get(key).cloned()
    }
}

#[async_trait]
impl StateStore for MemoryStateStore {
    async fn get(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self.raw(key))
    }

    async fn put(&self, key: &str, value: &str) -> AppResult<()> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

fn response(status: u16, value: Value) -> Result<OAuthHttpResponse, OAuthHttpFailure> {
    Ok(OAuthHttpResponse {
        status,
        body: serde_json::to_vec(&value).unwrap(),
    })
}

fn empty_response(status: u16) -> Result<OAuthHttpResponse, OAuthHttpFailure> {
    Ok(OAuthHttpResponse {
        status,
        body: Vec::new(),
    })
}

fn credentials(expires_at: i64) -> StoredOAuthCredentials {
    StoredOAuthCredentials {
        version: 1,
        access_token: "access-original".into(),
        refresh_token: "refresh-original".into(),
        id_token: None,
        account_id: Some("account-original".into()),
        email: None,
        expires_at,
        updated_at: "2027-01-15T08:00:00.000Z".into(),
    }
}

fn form(request: &OAuthHttpRequest) -> HashMap<String, String> {
    form_urlencoded::parse(request.body.as_bytes())
        .into_owned()
        .collect()
}

#[tokio::test]
async fn device_start_pending_and_completion_use_repository() {
    let http = FakeHttp::default();
    http.push(response(
        200,
        json!({
            "device_auth_id": "device-auth-sensitive",
            "user_code": "ABCD-EFGH",
            "interval": "1"
        }),
    ));
    http.push(empty_response(403));
    http.push(response(
        200,
        json!({
            "authorization_code": "authorization-code",
            "code_verifier": "code-verifier",
            "code_challenge": "code-challenge"
        }),
    ));
    http.push(response(
        200,
        json!({
            "access_token": "device-access-sensitive",
            "refresh_token": "device-refresh-sensitive",
            "expires_in": 3600
        }),
    ));
    let clock = FakeClock::new(NOW_MS);
    let provider = OAuthProvider::new(&http, &clock);
    let state_store = MemoryStateStore::default();
    let repository = OAuthRepository::new(&state_store);
    let service =
        DeviceAuthorizationService::new(&repository, &provider, &clock, STATE_SIGNING_KEY);

    let authorization = service.start().await.unwrap();
    assert_eq!(authorization.verification_uri, DEVICE_VERIFICATION_URL);
    assert_eq!(authorization.user_code, "ABCD-EFGH");
    assert_eq!(authorization.expires_in, 900);
    assert_eq!(authorization.interval, 1);
    assert!(!authorization.state.contains("device-auth-sensitive"));

    assert_eq!(
        service.poll(&authorization.state).await.unwrap(),
        DevicePollResult::Pending { retry_after: 1 }
    );
    let stored = service.poll(&authorization.state).await.unwrap();
    let DevicePollResult::Stored { credentials } = stored else {
        panic!("expected stored device credentials");
    };
    assert_eq!(credentials.access_token, "device-access-sensitive");
    assert_eq!(credentials.refresh_token, "device-refresh-sensitive");

    assert_eq!(
        repository.read().await.unwrap().unwrap().access_token,
        "device-access-sensitive"
    );

    let requests = http.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests[0].url,
        "https://auth.openai.com/api/accounts/deviceauth/usercode"
    );
    assert_eq!(requests[0].timeout_ms, 10_000);
    assert_eq!(requests[0].max_response_bytes, 64 * 1024);
    assert_eq!(
        form(&requests[3]).get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(
        form(&requests[3]).get("code_verifier").map(String::as_str),
        Some("code-verifier")
    );
    assert!(http.is_empty());
}

#[tokio::test]
async fn device_flow_rejects_expired_invalid_and_aborted_sessions() {
    let http = FakeHttp::default();
    http.push(response(
        200,
        json!({
            "device_auth_id": "device",
            "user_code": "CODE",
            "interval": 1
        }),
    ));
    let clock = FakeClock::new(NOW_MS);
    let provider = OAuthProvider::new(&http, &clock);
    let state_store = MemoryStateStore::default();
    let repository = OAuthRepository::new(&state_store);
    let service =
        DeviceAuthorizationService::new(&repository, &provider, &clock, STATE_SIGNING_KEY);
    let authorization = service.start().await.unwrap();

    clock.set(NOW_MS + DEVICE_LIFETIME_MS);
    let expired = service.poll(&authorization.state).await.unwrap_err();
    assert_eq!(expired.status, 410);
    assert_eq!(expired.code.as_deref(), Some("device_session_expired"));

    let invalid = service.poll("not-an-envelope").await.unwrap_err();
    assert_eq!(invalid.status, 400);
    assert_eq!(invalid.code.as_deref(), Some("invalid_device_session"));

    let aborting_http = FakeHttp::default();
    aborting_http.push(Err(OAuthHttpFailure::ClientAborted));
    let aborting_provider = OAuthProvider::new(&aborting_http, &clock);
    let aborting_service =
        DeviceAuthorizationService::new(&repository, &aborting_provider, &clock, STATE_SIGNING_KEY);
    let aborted = aborting_service.start().await.unwrap_err();
    assert_eq!(aborted.status, 408);
    assert_eq!(aborted.code.as_deref(), Some("request_aborted"));
}

#[tokio::test]
async fn device_state_is_bound_to_the_proxy_record_id() {
    let http = FakeHttp::default();
    http.push(response(
        200,
        json!({
            "device_auth_id": "device",
            "user_code": "CODE",
            "interval": 1
        }),
    ));
    let clock = FakeClock::new(NOW_MS);
    let provider = OAuthProvider::new(&http, &clock);
    let state_store = MemoryStateStore::default();
    let first = OAuthRepository::for_auth_proxy_account(
        &state_store,
        "00000000-0000-4000-8000-000000000001",
    );
    let second = OAuthRepository::for_auth_proxy_account(
        &state_store,
        "00000000-0000-4000-8000-000000000002",
    );
    let first_service = DeviceAuthorizationService::scoped(
        &first,
        &provider,
        &clock,
        STATE_SIGNING_KEY,
        "00000000-0000-4000-8000-000000000001",
    );
    let second_service = DeviceAuthorizationService::scoped(
        &second,
        &provider,
        &clock,
        STATE_SIGNING_KEY,
        "00000000-0000-4000-8000-000000000002",
    );
    let authorization = first_service.start().await.unwrap();
    let error = second_service.poll(&authorization.state).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("invalid_device_session"));
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn refresh_retries_transient_failures_and_rotates_credentials() {
    let http = FakeHttp::default();
    http.push(Err(OAuthHttpFailure::Network));
    http.push(empty_response(503));
    http.push(response(
        200,
        json!({
            "access_token": "access-refreshed-sensitive",
            "refresh_token": "refresh-refreshed-sensitive",
            "expires_in": 3600
        }),
    ));
    let clock = FakeClock::new(NOW_MS + 500);
    let provider = OAuthProvider::new(&http, &clock);
    let state_store = MemoryStateStore::default();
    let repository = OAuthRepository::new(&state_store);
    repository
        .store(&credentials(NOW_MS + 60_000))
        .await
        .unwrap();
    let service = OAuthRefreshService::new(&repository, &provider, &clock);

    assert_eq!(
        service.refresh(Some(NOW_MS)).await.unwrap(),
        OAuthRefreshResult::Refreshed
    );
    assert_eq!(*clock.sleeps.lock().unwrap(), vec![1_000, 2_000]);
    let stored = repository.read().await.unwrap().unwrap();
    assert_eq!(stored.access_token, "access-refreshed-sensitive");
    assert_eq!(stored.refresh_token, "refresh-refreshed-sensitive");
    assert_eq!(stored.expires_at, NOW_MS + 500 + 3_600_000);
    for request in http.requests() {
        let values = form(&request);
        assert_eq!(
            values.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            values.get("refresh_token").map(String::as_str),
            Some("refresh-original")
        );
        assert!(!format!("{request:?}").contains("refresh-original"));
    }
    assert!(http.is_empty());
}

#[tokio::test]
async fn proxy_credentials_are_uuid_scoped_and_fall_back_to_primary() {
    let state_store = MemoryStateStore::default();
    let primary = OAuthRepository::new(&state_store);
    let proxy_id = "00000000-0000-4000-8000-000000000001";
    let proxy = OAuthRepository::for_auth_proxy_account(&state_store, proxy_id);
    primary.store(&credentials(NOW_MS + 60_000)).await.unwrap();

    let selected = auth_proxy_credentials_or_primary(&proxy, &primary, NOW_MS)
        .await
        .unwrap();
    assert_eq!(selected.access_token, "access-original");

    let mut proxy_credentials = credentials(NOW_MS + 60_000);
    proxy_credentials.access_token = "access-proxy-sensitive".into();
    proxy_credentials.refresh_token = "refresh-proxy-sensitive".into();
    proxy_credentials.account_id = Some("account-proxy".into());
    proxy.store(&proxy_credentials).await.unwrap();
    let selected = auth_proxy_credentials_or_primary(&proxy, &primary, NOW_MS)
        .await
        .unwrap();
    assert_eq!(selected.access_token, "access-proxy-sensitive");
    assert_eq!(selected.account_id.as_deref(), Some("account-proxy"));

    proxy_credentials.account_id = None;
    proxy.store(&proxy_credentials).await.unwrap();
    let selected = auth_proxy_credentials_or_primary(&proxy, &primary, NOW_MS)
        .await
        .unwrap();
    assert_eq!(selected.access_token, "access-original");

    proxy_credentials.account_id = Some("account-proxy".into());
    proxy_credentials.expires_at = NOW_MS;
    proxy.store(&proxy_credentials).await.unwrap();
    let selected = auth_proxy_credentials_or_primary(&proxy, &primary, NOW_MS)
        .await
        .unwrap();
    assert_eq!(selected.access_token, "access-original");
}

#[tokio::test]
async fn refresh_distinguishes_missing_not_due_and_safe_provider_errors() {
    let clock = FakeClock::new(NOW_MS);

    let missing_http = FakeHttp::default();
    let missing_provider = OAuthProvider::new(&missing_http, &clock);
    let missing_store = MemoryStateStore::default();
    let missing_repository = OAuthRepository::new(&missing_store);
    let missing_service = OAuthRefreshService::new(&missing_repository, &missing_provider, &clock);
    assert_eq!(
        missing_service.refresh(Some(NOW_MS)).await.unwrap(),
        OAuthRefreshResult::Missing
    );
    assert!(missing_http.requests().is_empty());

    let future_http = FakeHttp::default();
    let future_provider = OAuthProvider::new(&future_http, &clock);
    let future_store = MemoryStateStore::default();
    let future_repository = OAuthRepository::new(&future_store);
    future_repository
        .store(&credentials(NOW_MS + REFRESH_WINDOW_MS + 1))
        .await
        .unwrap();
    let future_service = OAuthRefreshService::new(&future_repository, &future_provider, &clock);
    assert_eq!(
        future_service.refresh(Some(NOW_MS)).await.unwrap(),
        OAuthRefreshResult::NotDue
    );
    assert!(future_http.requests().is_empty());

    let failing_http = FakeHttp::default();
    failing_http.push(Ok(OAuthHttpResponse {
        status: 400,
        body: b"access-original refresh-original should-never-surface".to_vec(),
    }));
    let failing_provider = OAuthProvider::new(&failing_http, &clock);
    let failing_store = MemoryStateStore::default();
    let failing_repository = OAuthRepository::new(&failing_store);
    failing_repository
        .store(&credentials(NOW_MS))
        .await
        .unwrap();
    let failing_service = OAuthRefreshService::new(&failing_repository, &failing_provider, &clock);
    let error = failing_service.refresh(Some(NOW_MS)).await.unwrap_err();
    assert_eq!(error.status, 502);
    assert_eq!(error.code.as_deref(), Some("oauth_provider_error"));
    assert!(!error.message.contains("access-original"));
    assert!(!error.message.contains("refresh-original"));
    assert!(clock.sleeps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn refresh_preserves_rate_limit_error_semantics() {
    let clock = FakeClock::new(NOW_MS);
    let limited_http = FakeHttp::default();
    limited_http.push(empty_response(429));
    limited_http.push(empty_response(429));
    limited_http.push(empty_response(429));
    let limited_provider = OAuthProvider::new(&limited_http, &clock);
    let error = limited_provider
        .refresh_provider_token("secret")
        .await
        .unwrap_err();
    assert_eq!(error.status, 502);
    assert_eq!(error.code.as_deref(), Some("oauth_rate_limited"));
    assert_eq!(*clock.sleeps.lock().unwrap(), vec![1_000, 2_000]);
}
