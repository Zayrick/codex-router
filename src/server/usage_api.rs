//! HTTP handlers for analytics, the request log and its CSV export, shared by
//! the management API and the signed-in user views.

use axum::{
    body::Body,
    http::{HeaderValue, header},
    response::Response,
};
use url::Url;

use crate::core::{ApiError, AppResult};

use super::{
    config::AppConfig,
    oauth::current_time_ms,
    response,
    state::AppState,
    usage::{UsageEventRow, UsageFilters},
    usage_analytics::{AnalyticsQueryError, AnalyticsRequest, AnalyticsScope, EventPageRequest},
};

pub async fn analytics(
    url: &Url,
    scope: AnalyticsScope,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let now = current_time_ms();
    let earliest = earliest(url, &scope, state).await?;
    let request = AnalyticsRequest::from_url(url, &scope, earliest, now).map_err(query_error)?;
    let analytics = state
        .usage
        .analytics(
            request,
            &config.usage_tracking.model_prices,
            config.usage_tracking.fast_cost_multiplier(),
            now,
        )
        .await
        .map_err(|error| {
            tracing::warn!(event = "usage_analytics", status = "failed", error = %error);
            unavailable()
        })?;
    response::json(&analytics, 200)
}

pub async fn events(
    url: &Url,
    scope: AnalyticsScope,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let now = current_time_ms();
    let earliest = earliest(url, &scope, state).await?;
    let request = EventPageRequest::from_url(url, &scope, earliest, now).map_err(query_error)?;
    let prices = &config.usage_tracking.model_prices;
    let multiplier = config.usage_tracking.fast_cost_multiplier();
    let failed = |error: anyhow::Error| {
        tracing::warn!(event = "usage_events", status = "failed", error = %error);
        unavailable()
    };
    if url
        .query_pairs()
        .any(|(key, value)| key == "format" && value == "csv")
    {
        let events = state
            .usage
            .export_events(request, prices, multiplier)
            .await
            .map_err(failed)?;
        return Ok(csv_response(&events));
    }
    let page = state
        .usage
        .event_page(request, prices, multiplier, now)
        .await
        .map_err(failed)?;
    response::json(&page, 200)
}

/// Earliest event visible to the scope; only needed for the `all` range.
async fn earliest(url: &Url, scope: &AnalyticsScope, state: &AppState) -> AppResult<Option<i64>> {
    if !url
        .query_pairs()
        .any(|(key, value)| key == "range" && value == "all")
    {
        return Ok(None);
    }
    let filters = match scope {
        AnalyticsScope::Admin => UsageFilters::default(),
        AnalyticsScope::Identity(identity) => UsageFilters::new(Some(identity.clone()), None),
    };
    state
        .usage
        .earliest_event_at(filters)
        .await
        .map_err(|error| {
            tracing::warn!(event = "usage_earliest", status = "failed", error = %error);
            unavailable()
        })
}

const CSV_COLUMNS: &str = "recorded_at,request_id,response_id,identity_type,identity_name,\
codex_account,account_group,requested_model,model,status,status_code,error_code,\
reasoning_effort,service_tier,response_service_tier,transport,stream,endpoint,session_id,\
parent_session_id,session_kind,client_ip,x_forwarded_for,user_agent,ttft_ms,latency_ms,\
input_tokens,cached_input_tokens,cache_creation_input_tokens,output_tokens,\
reasoning_output_tokens,total_tokens,cost_usd";

fn csv_response(events: &[UsageEventRow]) -> Response {
    let mut output = String::from(CSV_COLUMNS);
    output.push('\n');
    for event in events {
        let fields = [
            event.recorded_at.to_string(),
            event.request_id.clone(),
            event.response_id.clone(),
            event.identity_type.clone(),
            event.identity_name.clone(),
            event.codex_account_name.clone(),
            event.account_group_name.clone(),
            event.requested_model.clone(),
            event.model.clone(),
            event.status.clone(),
            event
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_default(),
            event.error_code.clone(),
            event.reasoning_effort.clone(),
            event.service_tier.clone(),
            event.response_service_tier.clone(),
            event.transport.clone(),
            event.stream.to_string(),
            event.endpoint.clone(),
            event.session_id.clone(),
            event.parent_session_id.clone(),
            event.session_kind.clone(),
            event.client_ip.clone(),
            event.x_forwarded_for.clone(),
            event.user_agent.clone(),
            event
                .ttft_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event.latency_ms.to_string(),
            event.input_tokens.to_string(),
            event.cached_input_tokens.to_string(),
            event.cache_creation_input_tokens.to_string(),
            event.output_tokens.to_string(),
            event.reasoning_output_tokens.to_string(),
            event.total_tokens.to_string(),
            format!("{:.6}", event.cost_usd),
        ];
        let line = fields
            .iter()
            .map(|field| csv_field(field))
            .collect::<Vec<_>>()
            .join(",");
        output.push_str(&line);
        output.push('\n');
    }
    let mut response = Response::new(Body::from(output));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"codex-router-requests.csv\""),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Quotes a CSV field and neutralizes spreadsheet formulas.
fn csv_field(value: &str) -> String {
    let guarded = if value.starts_with(['=', '+', '-', '@']) && value.parse::<f64>().is_err() {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn query_error(error: AnalyticsQueryError) -> ApiError {
    ApiError::new(400, "The usage query is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_usage_query")
        .with_param(error.param)
}

fn unavailable() -> ApiError {
    ApiError::new(503, "Usage data is temporarily unavailable.")
        .with_kind("server_error")
        .with_code("usage_query_failed")
}

#[cfg(test)]
mod tests {
    use super::csv_field;

    #[test]
    fn quotes_and_guards_csv_fields() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_field("-1.5"), "-1.5");
    }
}
