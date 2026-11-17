//! Client-side request facts attached to usage events: network origin, user
//! agent, and the Codex session hierarchy announced through request headers.

use axum::http::{Extensions, HeaderMap};
use serde_json::{Map, Value};

use super::{config::AppConfig, console_security::client_address};

/// Header values are clipped so a misbehaving client cannot bloat the usage database.
const MAX_CLIENT_FIELD_CHARS: usize = 512;

/// Role of a request within its Codex session tree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionKind {
    /// No session information was sent.
    #[default]
    Unknown,
    Main,
    Subagent,
    Fork,
    Compaction,
}

impl SessionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "",
            Self::Main => "main",
            Self::Subagent => "subagent",
            Self::Fork => "fork",
            Self::Compaction => "compaction",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageSession {
    pub id: String,
    pub parent_id: String,
    pub kind: SessionKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageClient {
    /// Client address after honoring trusted reverse proxies.
    pub ip: String,
    /// Raw `X-Forwarded-For` chain as received.
    pub forwarded_for: String,
    pub user_agent: String,
    pub session: UsageSession,
}

impl UsageClient {
    pub fn from_request(extensions: &Extensions, headers: &HeaderMap, config: &AppConfig) -> Self {
        Self {
            ip: client_address(extensions, headers, config)
                .map(|ip| ip.to_string())
                .unwrap_or_default(),
            forwarded_for: joined_header(headers, "x-forwarded-for"),
            user_agent: header(headers, "user-agent"),
            session: parse_session(headers),
        }
    }
}

/// Mirrors how the Codex CLI announces its thread tree: `session_id`/`thread_id`
/// headers, the `x-codex-turn-metadata` JSON, an explicit parent thread header,
/// and `x-openai-subagent` for review/compaction helpers.
fn parse_session(headers: &HeaderMap) -> UsageSession {
    let metadata = headers
        .get("x-codex-turn-metadata")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .and_then(|value| match value {
            Value::Object(object) => Some(object),
            _ => None,
        })
        .unwrap_or_default();
    let session = first_header(headers, &["session_id", "session-id", "x-session-id"])
        .or_else(|| metadata_field(&metadata, "session_id"))
        .or_else(|| first_header(headers, &["x-claude-code-session-id"]))
        .unwrap_or_default();
    let thread = first_header(headers, &["thread_id", "thread-id"])
        .or_else(|| metadata_field(&metadata, "thread_id"))
        .unwrap_or_default();
    let parent_thread = first_header(headers, &["x-codex-parent-thread-id"])
        .or_else(|| metadata_field(&metadata, "parent_thread_id"))
        .unwrap_or_default();
    let forked_from = metadata_field(&metadata, "forked_from_thread_id")
        .or_else(|| metadata_field(&metadata, "forked_from_id"))
        .unwrap_or_default();
    let subagent = header(headers, "x-openai-subagent").to_ascii_lowercase();

    // The thread identifies the conversation node; fall back to the session.
    let id = if thread.is_empty() {
        session.clone()
    } else {
        thread.clone()
    };
    if id.is_empty() {
        return UsageSession::default();
    }
    let distinct =
        |candidate: &str| (!candidate.is_empty() && candidate != id).then(|| candidate.to_owned());
    if subagent == "compact" {
        return UsageSession {
            parent_id: distinct(&parent_thread).unwrap_or_default(),
            id,
            kind: SessionKind::Compaction,
        };
    }
    if let Some(parent_id) = distinct(&forked_from) {
        return UsageSession {
            id,
            parent_id,
            kind: SessionKind::Fork,
        };
    }
    let subagent_signal = !matches!(subagent.as_str(), "" | "false" | "0")
        || metadata_field(&metadata, "subagent_kind").is_some();
    let parent_id = distinct(&parent_thread).or_else(|| distinct(&session));
    if subagent_signal || parent_id.is_some() {
        return UsageSession {
            id,
            parent_id: parent_id.unwrap_or_default(),
            kind: SessionKind::Subagent,
        };
    }
    UsageSession {
        id,
        parent_id: String::new(),
        kind: SessionKind::Main,
    }
}

fn metadata_field(metadata: &Map<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(clip)
        .filter(|value| !value.is_empty())
}

fn first_header(headers: &HeaderMap, names: &[&str]) -> Option<String> {
    names
        .iter()
        .map(|name| header(headers, name))
        .find(|value| !value.is_empty())
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(clip)
        .unwrap_or_default()
}

fn joined_header(headers: &HeaderMap, name: &str) -> String {
    let values = headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    clip(&values.join(", "))
}

fn clip(value: &str) -> String {
    value.trim().chars().take(MAX_CLIENT_FIELD_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn reads_a_main_codex_session() {
        let session = parse_session(&headers(&[("session_id", "s-1")]));
        assert_eq!(session.id, "s-1");
        assert_eq!(session.parent_id, "");
        assert_eq!(session.kind, SessionKind::Main);
    }

    #[test]
    fn detects_subagents_from_thread_and_parent_headers() {
        let session = parse_session(&headers(&[("session_id", "s-1"), ("thread_id", "t-2")]));
        assert_eq!(
            (session.id.as_str(), session.parent_id.as_str()),
            ("t-2", "s-1")
        );
        assert_eq!(session.kind, SessionKind::Subagent);

        let session = parse_session(&headers(&[
            ("session_id", "s-1"),
            ("x-codex-parent-thread-id", "p-0"),
        ]));
        assert_eq!(session.parent_id, "p-0");
        assert_eq!(session.kind, SessionKind::Subagent);

        let session = parse_session(&headers(&[
            ("session_id", "s-1"),
            ("x-openai-subagent", "review"),
        ]));
        assert_eq!(session.kind, SessionKind::Subagent);
        assert_eq!(session.parent_id, "");
    }

    #[test]
    fn detects_forks_and_compactions() {
        let session = parse_session(&headers(&[(
            "x-codex-turn-metadata",
            r#"{"session_id":"s-1","thread_id":"t-9","forked_from_thread_id":"t-1"}"#,
        )]));
        assert_eq!(
            (session.id.as_str(), session.parent_id.as_str()),
            ("t-9", "t-1")
        );
        assert_eq!(session.kind, SessionKind::Fork);

        let session = parse_session(&headers(&[
            ("session_id", "s-1"),
            ("x-openai-subagent", "compact"),
        ]));
        assert_eq!(session.kind, SessionKind::Compaction);
    }

    #[test]
    fn requests_without_session_headers_are_unknown() {
        assert_eq!(parse_session(&HeaderMap::new()), UsageSession::default());
    }

    #[test]
    fn joins_and_clips_client_headers() {
        let source = headers(&[
            ("x-forwarded-for", "1.1.1.1"),
            ("x-forwarded-for", "2.2.2.2, 3.3.3.3"),
            ("user-agent", &"a".repeat(600)),
        ]);
        assert_eq!(
            joined_header(&source, "x-forwarded-for"),
            "1.1.1.1, 2.2.2.2, 3.3.3.3"
        );
        assert_eq!(header(&source, "user-agent").len(), MAX_CLIENT_FIELD_CHARS);
    }
}
