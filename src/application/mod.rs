//! Application routing, request adaptation, tokenization, and usage monitoring.

mod adapters;
mod reset_watch;
mod routes;
mod tokenizer;
mod usage_monitor;

pub use adapters::{AdaptedUpstreamRequest, RequestAdapter, ResponseAdapter};
pub use reset_watch::reset_watch_notification;
pub use routes::{
    AdminRoute, ApiRoute, CONSOLE_BASE_PATH, ConsoleRoute, ProtocolFamily, is_console_path,
    is_known_api_path, is_legacy_admin_path_family, legacy_admin_base_path, match_api_route,
    match_console_route,
};
pub use tokenizer::Cl100kTokenCounter;
pub use usage_monitor::{
    CodexUsageMonitorEvaluation, CodexUsageMonitorState, MonitoredQuotaWindow,
    evaluate_codex_usage, validate_codex_usage_monitor_state,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushNotification {
    pub title: String,
    pub body: String,
    pub url: Option<String>,
}
