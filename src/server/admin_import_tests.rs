use std::path::PathBuf;

use axum::{Router, body::to_bytes};
use tokio::fs;
use tower::ServiceExt;

use super::*;
use crate::server::{config::ConfigStore, router};

const IMPORT_PATH: &str = "/test/admin/codex-accounts/import";
const ORIGIN: &str = "http://router.example";

struct ImportApp {
    app: Router,
    state: AppState,
    directory: PathBuf,
    cookie: String,
}

impl ImportApp {
    async fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("codex-import-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).await.unwrap();
        let path = directory.join("config.toml");
        let mut config: AppConfig =
            toml::from_str(include_str!("../../config.example.toml")).unwrap();
        config.server.public_origin = ORIGIN.into();
        config.admin.path = "test".into();
        let session = create_admin_session(&config.admin.secret, current_time_ms()).unwrap();
        let cookie = admin_session_cookie_header(&session)
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        fs::write(&path, toml::to_string_pretty(&config).unwrap())
            .await
            .unwrap();
        let state = AppState::new(ConfigStore::load(path).await.unwrap())
            .await
            .unwrap();
        let app = router::build(state.clone());
        Self {
            app,
            state,
            directory,
            cookie,
        }
    }

    async fn request(
        &self,
        method: Method,
        body: String,
        authenticated: bool,
        origin: Option<&str>,
    ) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri(IMPORT_PATH)
            .header(header::CONTENT_TYPE, "application/json");
        if authenticated {
            request = request.header(header::COOKIE, &self.cookie);
        }
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        self.app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    async fn import(&self, input: Value) -> Response {
        self.request(Method::POST, input.to_string(), true, Some(ORIGIN))
            .await
    }
}

impl Drop for ImportApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn credentials(account: &str) -> Value {
    json!({
        "name": "导入账户",
        "credentials": { "tokens": {
            "access_token": format!("private-access-{account}"),
            "refresh_token": format!("private-refresh-{account}"),
            "account_id": account,
            "expires_in": 3600,
        }},
    })
}

#[tokio::test]
async fn import_endpoint_requires_session_origin_valid_json_and_bounded_body() {
    let test = ImportApp::new().await;
    for (method, authenticated, origin, input, status) in [
        (
            Method::POST,
            false,
            Some(ORIGIN),
            credentials("test").to_string(),
            401,
        ),
        (
            Method::POST,
            true,
            None,
            credentials("test").to_string(),
            403,
        ),
        (
            Method::POST,
            true,
            Some("http://other.example"),
            credentials("test").to_string(),
            403,
        ),
        (Method::GET, true, Some(ORIGIN), String::new(), 404),
        (
            Method::POST,
            true,
            Some(ORIGIN),
            "{invalid-json".into(),
            400,
        ),
        (Method::POST, true, Some(ORIGIN), "{}".into(), 400),
        (
            Method::POST,
            true,
            Some(ORIGIN),
            " ".repeat(MAX_CREDENTIAL_IMPORT_BYTES + 1),
            413,
        ),
    ] {
        let response = test.request(method, input, authenticated, origin).await;
        assert_eq!(response.status().as_u16(), status);
        if status != 404 {
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    let snapshot = test.state.config.snapshot().await;
    assert!(snapshot.state.account_routing.accounts.is_empty());
    assert!(snapshot.state.codex_account_oauth.is_empty());
}

#[tokio::test]
async fn import_endpoint_persists_credentials_returns_metadata_and_rejects_duplicates() {
    let test = ImportApp::new().await;
    let response = test.import(credentials("import-account")).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("private-"));
    let value: Value = serde_json::from_slice(&body).unwrap();
    let id = value["account"]["id"].as_str().unwrap();
    assert!(valid_record_id(id));
    assert_eq!(value["account"]["name"], "导入账户");
    assert_eq!(value["account"]["enabled"], true);
    assert_eq!(value["account"]["oauth"]["accountId"], "import-account");
    let reloaded = ConfigStore::load(test.directory.join("config.toml"))
        .await
        .unwrap();
    let stored = OAuthRepository::new(reloaded.as_ref(), id)
        .require_valid(current_time_ms())
        .await
        .unwrap();
    assert_eq!(stored.access_token, "private-access-import-account");
    assert_eq!(
        reloaded
            .snapshot()
            .await
            .state
            .account_routing
            .accounts
            .len(),
        1
    );

    // Refresh-only duplicates must be caught before making any provider request.
    let response = test
        .import(json!({ "refresh_token": stored.refresh_token }))
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = test.import(credentials("import-account")).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        test.state
            .config
            .snapshot()
            .await
            .state
            .account_routing
            .accounts
            .len(),
        1
    );
}

#[tokio::test]
async fn simultaneous_imports_keep_all_accounts_and_reject_same_identity() {
    let test = ImportApp::new().await;
    let (first, second) = tokio::join!(
        test.import(credentials("first")),
        test.import(credentials("second"))
    );
    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(second.status(), StatusCode::CREATED);
    let (first, second) = tokio::join!(
        test.import(credentials("same")),
        test.import(credentials("same"))
    );
    let mut statuses = [first.status().as_u16(), second.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [201, 409]);
    let snapshot = test.state.config.snapshot().await;
    assert_eq!(snapshot.state.account_routing.accounts.len(), 3);
    assert_eq!(snapshot.state.codex_account_oauth.len(), 3);
}

#[tokio::test]
async fn failed_import_write_leaves_no_account_or_credentials_in_memory() {
    let test = ImportApp::new().await;
    fs::remove_file(test.directory.join("config.toml"))
        .await
        .unwrap();
    // A directory at the destination makes the atomic rename fail.
    fs::create_dir(test.directory.join("config.toml"))
        .await
        .unwrap();
    let response = test.import(credentials("write-failure")).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let snapshot = test.state.config.snapshot().await;
    assert!(snapshot.state.account_routing.accounts.is_empty());
    assert!(snapshot.state.codex_account_oauth.is_empty());
}
