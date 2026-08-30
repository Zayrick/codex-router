use std::io::Read;

use ruzstd::decoding::StreamingDecoder;
use serde_json::Value;

use crate::core::{ApiError, AppResult, JsonObject};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedJsonBody {
    pub body: JsonObject,
    /// Original, possibly compressed bytes used when forwarding the request.
    pub encoded_body: Vec<u8>,
}

/// Parses request bytes and returns only the JSON object.
pub fn parse_json_body(
    encoded_body: &[u8],
    content_encoding: Option<&str>,
) -> AppResult<JsonObject> {
    parse_object(encoded_body, content_encoding)
}

/// Parses an owned body while retaining its original wire representation.
pub fn parse_json_body_with_source(
    encoded_body: Vec<u8>,
    content_encoding: Option<&str>,
) -> AppResult<ParsedJsonBody> {
    let body = parse_object(&encoded_body, content_encoding)?;
    Ok(ParsedJsonBody { body, encoded_body })
}

fn parse_object(encoded: &[u8], content_encoding: Option<&str>) -> AppResult<JsonObject> {
    let decoded;
    let bytes = match content_encoding
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => encoded,
        Some(encoding) if encoding.eq_ignore_ascii_case("zstd") => {
            decoded = decode_zstd(encoded)?;
            decoded.as_slice()
        }
        Some(_) => return Err(invalid_json()),
    };

    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(invalid_json()),
    }
}

fn decode_zstd(encoded: &[u8]) -> AppResult<Vec<u8>> {
    let mut decoder = StreamingDecoder::new(encoded).map_err(|_| invalid_json())?;
    let mut decoded = Vec::with_capacity(encoded.len());
    decoder
        .read_to_end(&mut decoded)
        .map_err(|_| invalid_json())?;
    Ok(decoded)
}

fn invalid_json() -> ApiError {
    ApiError::new(400, "The request body is not valid JSON.")
        .with_kind("invalid_request_error")
        .with_code("invalid_json")
}

#[cfg(test)]
mod tests {
    use ruzstd::encoding::{CompressionLevel, compress_to_vec};
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_an_object_and_retains_encoded_bytes() {
        let encoded = br#"{"model":"gpt-5","stream":true}"#.to_vec();
        let parsed = parse_json_body_with_source(encoded.clone(), None).expect("valid JSON object");
        assert_eq!(parsed.body["model"], "gpt-5");
        assert_eq!(parsed.encoded_body, encoded);
    }

    #[test]
    fn rejects_empty_non_object_and_invalid_utf8_json() {
        for encoded in [
            b"".as_slice(),
            b"[]".as_slice(),
            b"{\"x\":\"\xff\"}".as_slice(),
        ] {
            let error = parse_json_body(encoded, None).expect_err("body must be rejected");
            assert_eq!(error.status, 400);
            assert_eq!(error.code.as_deref(), Some("invalid_json"));
        }
    }

    #[test]
    fn decodes_valid_zstd_and_rejects_invalid_or_unsupported_encoding() {
        let plain = br#"{"input":"hello"}"#;
        let encoded = compress_to_vec(plain.as_slice(), CompressionLevel::Fastest);
        let parsed =
            parse_json_body_with_source(encoded.clone(), Some(" ZSTD ")).expect("valid zstd JSON");
        assert_eq!(
            parsed.body,
            json!({"input": "hello"}).as_object().unwrap().clone()
        );
        assert_eq!(parsed.encoded_body, encoded);
        assert!(parse_json_body(plain, Some("zstd")).is_err());
        assert!(parse_json_body(plain, Some("gzip")).is_err());
    }
}
