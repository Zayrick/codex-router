//! Copy-on-write policies for backend request bodies.

use std::borrow::Cow;

use serde_json::Value;

use crate::core::JsonObject;

pub(crate) struct BodyEditor<'a> {
    original: &'a JsonObject,
    owned: Option<JsonObject>,
}

impl<'a> BodyEditor<'a> {
    pub(crate) fn new(original: &'a JsonObject) -> Self {
        Self {
            original,
            owned: None,
        }
    }

    pub(crate) fn current(&self) -> &JsonObject {
        self.owned.as_ref().unwrap_or(self.original)
    }

    pub(crate) fn make_mut(&mut self) -> &mut JsonObject {
        self.owned.get_or_insert_with(|| self.original.clone())
    }

    pub(crate) fn finish(self) -> Cow<'a, JsonObject> {
        match self.owned {
            Some(body) => Cow::Owned(body),
            None => Cow::Borrowed(self.original),
        }
    }
}

const REMOVED_RESPONSE_FIELDS: [&str; 12] = [
    "max_completion_tokens",
    "max_output_tokens",
    "maxOutputTokens",
    "max_tokens",
    "context_management",
    "temperature",
    "top_p",
    "truncation",
    "user",
    "prompt_cache_options",
    "prompt_cache_retention",
    "safety_identifier",
];

const REMOVED_HTTP_RESPONSE_FIELDS: [&str; 3] =
    ["previous_response_id", "generate", "stream_options"];

fn apply_base_policy(editor: &mut BodyEditor<'_>) {
    remove_fields(editor, &REMOVED_RESPONSE_FIELDS);
    remove_non_priority_service_tier(editor);
    set_field(editor, "store", Value::Bool(false));
}

pub(crate) fn apply_http_response_create_egress_policy_to(editor: &mut BodyEditor<'_>) {
    apply_base_policy(editor);
    remove_fields(editor, &REMOVED_HTTP_RESPONSE_FIELDS);
}

pub(crate) fn apply_websocket_response_create_egress_policy_to(editor: &mut BodyEditor<'_>) {
    apply_base_policy(editor);
}

fn remove_fields(editor: &mut BodyEditor<'_>, fields: &[&str]) {
    for field in fields {
        if editor.current().contains_key(*field) {
            editor.make_mut().remove(*field);
        }
    }
}

fn set_field(editor: &mut BodyEditor<'_>, field: &str, value: Value) {
    if editor.current().get(field) != Some(&value) {
        editor.make_mut().insert(field.into(), value);
    }
}

/// Responses rejects `max_output_tokens` below this value.
pub(crate) const MIN_MAX_OUTPUT_TOKENS: u64 = 16;

/// Maps a downstream output limit onto `max_output_tokens`, raising limits
/// below the upstream minimum instead of letting the request fail.
pub(crate) fn insert_max_output_tokens(body: &mut JsonObject, limit: Option<&Value>) {
    if let Some(limit) = limit.and_then(Value::as_u64) {
        body.insert(
            "max_output_tokens".into(),
            Value::from(limit.max(MIN_MAX_OUTPUT_TOKENS)),
        );
    }
}

/// Fields the plan-backed Responses endpoint rejects. `max_output_tokens`,
/// `context_management`, `prompt_cache_key` and `instructions` are accepted
/// and therefore kept.
const REMOVED_CONVERTED_RESPONSE_FIELDS: [&str; 14] = [
    "background",
    "generate",
    "max_completion_tokens",
    "max_tokens",
    "maxOutputTokens",
    "metadata",
    "previous_response_id",
    "prompt_cache_options",
    "prompt_cache_retention",
    "safety_identifier",
    "temperature",
    "top_p",
    "truncation",
    "user",
];
const REQUIRED_INCLUDE: &str = "reasoning.encrypted_content";
const UNSUPPORTED_INCLUDES: [&str; 1] = ["message.output_text.logprobs"];
const UNSUPPORTED_STREAM_OPTIONS: [&str; 1] = ["include_usage"];
const UNSUPPORTED_INPUT_ITEM_TYPES: [&str; 1] = ["configuration_update"];
const UNSUPPORTED_CONTENT_FIELDS: [&str; 1] = ["prompt_cache_breakpoint"];

/// Egress policy for bodies converted from Chat/Completions/Messages/Gemini.
/// Only rejected values are removed; `store: false` and `stream: true` must be
/// explicit, and encrypted reasoning is requested for signature round-trips.
#[must_use]
pub fn apply_converted_response_egress_policy<'a>(body: &'a JsonObject) -> Cow<'a, JsonObject> {
    let mut editor = BodyEditor::new(body);
    remove_fields(&mut editor, &REMOVED_CONVERTED_RESPONSE_FIELDS);
    remove_non_priority_service_tier(&mut editor);
    set_field(&mut editor, "store", Value::Bool(false));
    set_field(&mut editor, "stream", Value::Bool(true));
    remove_unsupported_stream_options(&mut editor);
    normalize_include(&mut editor);
    remove_unsupported_input_parts(&mut editor);
    editor.finish()
}

fn remove_non_priority_service_tier(editor: &mut BodyEditor<'_>) {
    if editor
        .current()
        .get("service_tier")
        .is_some_and(|value| value != "priority")
    {
        editor.make_mut().remove("service_tier");
    }
}

fn remove_unsupported_stream_options(editor: &mut BodyEditor<'_>) {
    let Some(options) = editor.current().get("stream_options") else {
        return;
    };
    let Some(options) = options.as_object() else {
        editor.make_mut().remove("stream_options");
        return;
    };
    if !UNSUPPORTED_STREAM_OPTIONS
        .iter()
        .any(|field| options.contains_key(*field))
    {
        return;
    }
    let mut options = options.clone();
    for field in UNSUPPORTED_STREAM_OPTIONS {
        options.remove(field);
    }
    let body = editor.make_mut();
    if options.is_empty() {
        body.remove("stream_options");
    } else {
        body.insert("stream_options".into(), Value::Object(options));
    }
}

fn normalize_include(editor: &mut BodyEditor<'_>) {
    let mut include = editor
        .current()
        .get("include")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    include.retain(|value| {
        value
            .as_str()
            .is_some_and(|value| !UNSUPPORTED_INCLUDES.contains(&value))
    });
    if !include.iter().any(|value| value == REQUIRED_INCLUDE) {
        include.push(Value::String(REQUIRED_INCLUDE.into()));
    }
    set_field(editor, "include", Value::Array(include));
}

fn remove_unsupported_input_parts(editor: &mut BodyEditor<'_>) {
    let unsupported_item = |item: &Value| {
        item.get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| UNSUPPORTED_INPUT_ITEM_TYPES.contains(&kind))
    };
    let unsupported_part = |part: &Value| {
        part.as_object().is_some_and(|part| {
            UNSUPPORTED_CONTENT_FIELDS
                .iter()
                .any(|field| part.contains_key(*field))
        })
    };
    let needs_edit = editor
        .current()
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|input| {
            input.iter().any(|item| {
                unsupported_item(item)
                    || item
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|content| content.iter().any(unsupported_part))
            })
        });
    if !needs_edit {
        return;
    }
    let Some(input) = editor
        .make_mut()
        .get_mut("input")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    input.retain(|item| !unsupported_item(item));
    for content in input
        .iter_mut()
        .filter_map(|item| item.get_mut("content").and_then(Value::as_array_mut))
    {
        for part in content.iter_mut().filter_map(Value::as_object_mut) {
            for field in UNSUPPORTED_CONTENT_FIELDS {
                part.remove(field);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(value: Value) -> JsonObject {
        value.as_object().cloned().unwrap_or_default()
    }

    fn apply_http_policy(body: &JsonObject) -> Cow<'_, JsonObject> {
        let mut editor = BodyEditor::new(body);
        apply_http_response_create_egress_policy_to(&mut editor);
        editor.finish()
    }

    fn apply_websocket_policy(body: &JsonObject) -> Cow<'_, JsonObject> {
        let mut editor = BodyEditor::new(body);
        apply_websocket_response_create_egress_policy_to(&mut editor);
        editor.finish()
    }

    #[test]
    fn response_policy_is_copy_on_write_and_preserves_extensions() {
        let satisfied = object(json!({"model":"gpt-5.6-luna","store":false}));
        assert!(matches!(apply_http_policy(&satisfied), Cow::Borrowed(_)));

        let input = object(json!({
            "model":"gpt-5.6-luna",
            "store":true,
            "service_tier":"flex",
            "max_tokens":1024,
            "previous_response_id":"resp_1",
            "generate":true,
            "prompt_cache_options":{"mode":"implicit"},
            "prompt_cache_retention":"24h",
            "safety_identifier":"safety_1",
            "stream_options":{"include_usage":true},
            "unknown_extension":{"keep":true}
        }));
        let output = apply_http_policy(&input);
        assert_eq!(
            output.as_ref(),
            &object(json!({
                "model":"gpt-5.6-luna",
                "store":false,
                "unknown_extension":{"keep":true}
            }))
        );
        assert_eq!(input.get("store"), Some(&Value::Bool(true)));
    }

    #[test]
    fn websocket_policy_only_removes_websocket_unsupported_fields() {
        let input = object(json!({
            "model":"gpt-5.6-luna",
            "store":false,
            "previous_response_id":"resp_1",
            "generate":true,
            "prompt_cache_options":{"mode":"explicit"},
            "prompt_cache_retention":"24h",
            "safety_identifier":"safety_1",
            "stream_options":{"include_usage":true}
        }));
        assert_eq!(
            apply_websocket_policy(&input).as_ref(),
            &object(json!({
                "model":"gpt-5.6-luna",
                "store":false,
                "previous_response_id":"resp_1",
                "generate":true,
                "stream_options":{"include_usage":true}
            }))
        );
    }

    #[test]
    fn converted_policy_removes_only_rejected_values() {
        let input = object(json!({
            "instructions":"downstream",
            "store":true,
            "stream":false,
            "include":["message.input_image.image_url","message.output_text.logprobs"],
            "max_output_tokens":1024,
            "context_management":[{"type":"compaction"}],
            "prompt_cache_key":"cache",
            "service_tier":"flex",
            "max_tokens":1024,
            "metadata":{"k":"v"},
            "background":false,
            "previous_response_id":"resp_1",
            "generate":true,
            "prompt_cache_options":{"mode":"implicit"},
            "prompt_cache_retention":"24h",
            "safety_identifier":"safety_1",
            "stream_options":{"include_usage":true,"include_obfuscation":false},
            "input":[
                {"type":"configuration_update","reasoning":{"effort":"low"}},
                {"type":"message","role":"user","content":[
                    {"type":"input_text","text":"hi","prompt_cache_breakpoint":{"mode":"explicit"}}
                ]}
            ]
        }));
        assert_eq!(
            apply_converted_response_egress_policy(&input).as_ref(),
            &object(json!({
                "instructions":"downstream",
                "store":false,
                "stream":true,
                "include":["message.input_image.image_url","reasoning.encrypted_content"],
                "max_output_tokens":1024,
                "context_management":[{"type":"compaction"}],
                "prompt_cache_key":"cache",
                "stream_options":{"include_obfuscation":false},
                "input":[
                    {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}
                ]
            }))
        );
    }

    #[test]
    fn converted_policy_adds_required_fields_and_drops_empty_stream_options() {
        let input = object(json!({"stream_options":{"include_usage":true}}));
        assert_eq!(
            apply_converted_response_egress_policy(&input).as_ref(),
            &object(json!({
                "store":false,
                "stream":true,
                "include":["reasoning.encrypted_content"]
            }))
        );
        let satisfied = object(json!({
            "store":false,
            "stream":true,
            "include":["reasoning.encrypted_content"]
        }));
        assert!(matches!(
            apply_converted_response_egress_policy(&satisfied),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn output_limit_is_raised_to_upstream_minimum() {
        let mut body = JsonObject::new();
        insert_max_output_tokens(&mut body, Some(&json!(1)));
        assert_eq!(body["max_output_tokens"], MIN_MAX_OUTPUT_TOKENS);
        insert_max_output_tokens(&mut body, Some(&json!(64000)));
        assert_eq!(body["max_output_tokens"], 64000);
        insert_max_output_tokens(&mut body, Some(&json!("many")));
        assert_eq!(body["max_output_tokens"], 64000);
    }
}
