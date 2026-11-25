//! Which relayed Codex requests run on a pooled account, and proof that the
//! caller owns the ChatGPT account it claims before that happens.
//!
//! Users who point `openai_base_url` at the router send every request of the
//! built-in OpenAI provider here, signed with their own ChatGPT login. Only
//! inference moves to a pooled account. Everything else — Notes and History
//! (`alpha/*`), web search, realtime calls, memories — stays on the user's own
//! credentials, since it reads or writes that user's data or is tied to it by
//! later requests the router never sees.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use axum::http::{HeaderMap, Method};
use serde_json::Value;

use crate::{auth::sha256, upstream::codex::resolve_chatgpt_url};

use super::{chatgpt_proxy::ChatgptTransport, oauth::current_time_ms};

const CODEX_ROOT: &str = "/backend-api/codex/";
/// Where the caller's token is asked which accounts it may act for.
const ACCOUNTS_CHECK_PATH: &str = "/backend-api/wham/accounts/check";
const VERIFIED_TTL_MS: i64 = 30 * 60_000;
const REJECTED_TTL_MS: i64 = 60_000;
/// A verified owner stays trusted this long when ChatGPT cannot be reached.
const STALE_TRUST_MS: i64 = 6 * 60 * 60_000;
const MAX_VERDICTS: usize = 4_096;
/// Request headers copied onto the ownership check, so it looks like the client.
const FORWARDED_CHECK_HEADERS: [&str; 3] = ["user-agent", "originator", "version"];

/// A relayed request that runs on a pooled account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PooledRequest {
    /// Generates a model response (and so may probe a recovering account).
    pub generative: bool,
}

/// Classifies a relayed request; `None` keeps the caller's own credentials.
pub fn pooled_request(method: &Method, pathname: &str, websocket: bool) -> Option<PooledRequest> {
    let relative = pathname.strip_prefix(CODEX_ROOT)?.trim_end_matches('/');
    let generative = match (relative, method.as_str()) {
        // HTTP responses and the Responses WebSocket share a path.
        ("responses", "POST") => true,
        ("responses", "GET") if websocket => true,
        ("responses/compact" | "images/generations" | "images/edits", "POST") => true,
        // The model picker should list what the pooled accounts can serve.
        ("models", "GET") if !websocket => false,
        _ => return None,
    };
    Some(PooledRequest { generative })
}

/// Outcome of checking that a token may act for a ChatGPT account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Verified,
    Rejected,
    /// ChatGPT could not answer; nothing is known.
    Unknown,
}

#[derive(Clone, Default)]
pub struct AccountOwnership {
    verdicts: Arc<Mutex<HashMap<[u8; 32], Verdict>>>,
}

#[derive(Debug, Clone, Copy)]
struct Verdict {
    verified: bool,
    checked_at_ms: i64,
}

impl AccountOwnership {
    /// Whether the bearer token in `headers` belongs to `account_id`.
    pub async fn verify(
        &self,
        transport: &ChatgptTransport,
        headers: &HeaderMap,
        account_id: &str,
    ) -> Ownership {
        let Some(authorization) = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() > "Bearer ".len())
        else {
            return Ownership::Rejected;
        };
        let key = sha256(&format!("{account_id}\n{authorization}"));
        let now = current_time_ms();
        let cached = self.cached(&key);
        if let Some(verdict) = cached {
            let ttl = if verdict.verified {
                VERIFIED_TTL_MS
            } else {
                REJECTED_TTL_MS
            };
            if now - verdict.checked_at_ms < ttl {
                return ownership(verdict.verified);
            }
        }
        match check_accounts(transport, headers, authorization).await {
            Some(accounts) => {
                let verified = accounts.iter().any(|id| id == account_id);
                self.remember(key, verified, now);
                if !verified {
                    tracing::warn!(event = "relay_identity", status = "rejected", account_id);
                }
                ownership(verified)
            }
            // ChatGPT is unreachable: keep trusting a recent verification.
            None if cached.is_some_and(|verdict| {
                verdict.verified && now - verdict.checked_at_ms < STALE_TRUST_MS
            }) =>
            {
                Ownership::Verified
            }
            None => Ownership::Unknown,
        }
    }

    fn cached(&self, key: &[u8; 32]) -> Option<Verdict> {
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    fn remember(&self, key: [u8; 32], verified: bool, now_ms: i64) {
        let mut verdicts = self.verdicts.lock().unwrap_or_else(PoisonError::into_inner);
        if verdicts.len() >= MAX_VERDICTS && !verdicts.contains_key(&key) {
            verdicts.retain(|_, verdict| {
                let ttl = if verdict.verified {
                    STALE_TRUST_MS
                } else {
                    REJECTED_TTL_MS
                };
                now_ms - verdict.checked_at_ms < ttl
            });
            if verdicts.len() >= MAX_VERDICTS {
                verdicts.clear();
            }
        }
        verdicts.insert(
            key,
            Verdict {
                verified,
                checked_at_ms: now_ms,
            },
        );
    }
}

fn ownership(verified: bool) -> Ownership {
    if verified {
        Ownership::Verified
    } else {
        Ownership::Rejected
    }
}

/// Active account IDs the token may act for, or `None` when ChatGPT did not
/// answer conclusively. A rejected token yields an empty list.
async fn check_accounts(
    transport: &ChatgptTransport,
    headers: &HeaderMap,
    authorization: &str,
) -> Option<Vec<String>> {
    let target = resolve_chatgpt_url(ACCOUNTS_CHECK_PATH, None);
    let mut request = transport
        .client()
        .get(target.as_str())
        .header("authorization", authorization)
        .header("accept", "application/json")
        .timeout(std::time::Duration::from_secs(10));
    for name in FORWARDED_CHECK_HEADERS {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!(event = "relay_identity", status = "unavailable");
            return None;
        }
    };
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Some(Vec::new());
    }
    if !status.is_success() {
        tracing::warn!(
            event = "relay_identity",
            status = "unavailable",
            http_status = status.as_u16()
        );
        return None;
    }
    let body = response.json::<Value>().await.ok()?;
    Some(active_account_ids(&body))
}

/// Account IDs in an accounts-check response, in either of its two shapes:
/// a list of `{id}` entries, or a map of `{account: {account_id}}` entries.
fn active_account_ids(body: &Value) -> Vec<String> {
    let entries: Vec<&Value> = match body.get("accounts") {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let account = entry.get("account").unwrap_or(entry);
            let deactivated = account
                .get("is_deactivated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let id = account
                .get("account_id")
                .or_else(|| entry.get("id"))
                .and_then(Value::as_str)?;
            (!deactivated).then(|| id.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_inference_and_the_model_catalog_use_pooled_accounts() {
        let post = Method::POST;
        let get = Method::GET;
        for (method, path, websocket, expected) in [
            (&post, "/backend-api/codex/responses", false, Some(true)),
            (&get, "/backend-api/codex/responses", true, Some(true)),
            (&post, "/backend-api/codex/responses/", false, Some(true)),
            (
                &post,
                "/backend-api/codex/responses/compact",
                false,
                Some(true),
            ),
            (
                &post,
                "/backend-api/codex/images/generations",
                false,
                Some(true),
            ),
            (&post, "/backend-api/codex/images/edits", false, Some(true)),
            (&get, "/backend-api/codex/models", false, Some(false)),
            // The user's own data and features tied to the user's identity.
            (
                &post,
                "/backend-api/codex/alpha/notes/v2/thread_hint",
                false,
                None,
            ),
            (
                &post,
                "/backend-api/codex/alpha/notes/v2/write_file",
                false,
                None,
            ),
            (
                &post,
                "/backend-api/codex/alpha/history/v2/list_items",
                false,
                None,
            ),
            (&post, "/backend-api/codex/alpha/search", false, None),
            (&post, "/backend-api/codex/realtime/calls", false, None),
            (
                &post,
                "/backend-api/codex/memories/trace_summarize",
                false,
                None,
            ),
            (
                &post,
                "/backend-api/codex/analytics-events/events",
                false,
                None,
            ),
            // A plain GET of responses is not the WebSocket.
            (&get, "/backend-api/codex/responses", false, None),
            (&get, "/backend-api/wham/usage", false, None),
            (&post, "/backend-api/codex/responsesx", false, None),
        ] {
            assert_eq!(
                pooled_request(method, path, websocket).map(|request| request.generative),
                expected,
                "{method} {path} websocket={websocket}"
            );
        }
    }

    #[test]
    fn reads_both_accounts_check_shapes_and_skips_deactivated_accounts() {
        let list = json!({"accounts": [
            {"id": "a"},
            {"id": "b", "is_deactivated": true},
        ]});
        assert_eq!(active_account_ids(&list), ["a"]);
        let map = json!({
            "accounts": {
                "x": {"account": {"account_id": "team", "is_deactivated": false}},
                "y": {"account": {"account_id": "gone", "is_deactivated": true}},
            },
            "account_ordering": ["x", "y"],
        });
        assert_eq!(active_account_ids(&map), ["team"]);
        assert!(active_account_ids(&json!({})).is_empty());
    }

    #[test]
    fn verdicts_stay_bounded() {
        let ownership = AccountOwnership::default();
        for i in 0..MAX_VERDICTS + 10 {
            let key = sha256(&i.to_string());
            ownership.remember(key, true, 0);
        }
        assert!(ownership.verdicts.lock().unwrap().len() <= MAX_VERDICTS);
    }
}
