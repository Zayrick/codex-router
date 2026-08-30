use axum::response::Response;
use serde::Serialize;

use crate::{application::MonitoredQuotaWindow, core::ApiError};

use super::{response, state::AppState, usage_store::CodexUsageStateRepository};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicUsageSnapshot<'a> {
    sampled_at: i64,
    plan_type: Option<&'a str>,
    windows: Vec<PublicQuotaWindow<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicQuotaWindow<'a> {
    id: &'a str,
    category: crate::upstream::codex::CodexQuotaCategory,
    name: &'a str,
    kind: crate::upstream::codex::CodexQuotaWindowKind,
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    limit_window_seconds: Option<f64>,
    reset_at: Option<i64>,
}

impl<'a> From<&'a MonitoredQuotaWindow> for PublicQuotaWindow<'a> {
    fn from(window: &'a MonitoredQuotaWindow) -> Self {
        Self {
            id: &window.id,
            category: window.category,
            name: &window.name,
            kind: window.kind,
            used_percent: window.used_percent,
            remaining_percent: window.remaining_percent,
            limit_window_seconds: window.limit_window_seconds,
            reset_at: window.reset_at,
        }
    }
}

pub async fn usage_snapshot(state: &AppState) -> Response {
    let repository = CodexUsageStateRepository::new(state.config.as_ref());
    match repository.read().await {
        Ok(Some(snapshot)) => response::json(
            &PublicUsageSnapshot {
                sampled_at: snapshot.sampled_at,
                plan_type: snapshot.plan_type.as_deref(),
                windows: snapshot
                    .windows
                    .iter()
                    .map(PublicQuotaWindow::from)
                    .collect(),
            },
            200,
        )
        .unwrap_or_else(|_| response::empty(500)),
        Ok(None) => response::json(&serde_json::json!({ "snapshot": null }), 200)
            .unwrap_or_else(|_| response::empty(500)),
        Err(_) => response::api_error(&status_unavailable()),
    }
}

fn status_unavailable() -> ApiError {
    ApiError::new(503, "Usage status is temporarily unavailable.")
        .with_kind("server_error")
        .with_code("usage_status_unavailable")
}
