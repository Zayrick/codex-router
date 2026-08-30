use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error("invalid signing key")]
    InvalidKey,
    #[error("invalid signed value")]
    InvalidValue,
}

/// Signs JSON without encrypting it.
///
/// The first token segment is readable base64url JSON. The second segment is
/// an HMAC-SHA256 tag bound to `purpose`, so callers cannot alter the payload.
pub fn sign_json(value: &Value, key: &str, purpose: &str) -> Result<String, SignatureError> {
    if key.is_empty() || purpose.is_empty() {
        return Err(SignatureError::InvalidKey);
    }
    let payload = serde_json::to_vec(value).map_err(|_| SignatureError::InvalidValue)?;
    let signature = signature(&payload, key, purpose)?;
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

pub fn verify_json(token: &str, key: &str, purpose: &str) -> Result<Value, SignatureError> {
    let (payload, supplied_signature) =
        token.split_once('.').ok_or(SignatureError::InvalidValue)?;
    if supplied_signature.contains('.') {
        return Err(SignatureError::InvalidValue);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| SignatureError::InvalidValue)?;
    let supplied_signature = URL_SAFE_NO_PAD
        .decode(supplied_signature)
        .map_err(|_| SignatureError::InvalidValue)?;
    let expected_signature = signature(&payload, key, purpose)?;
    if supplied_signature.len() != expected_signature.len()
        || !bool::from(supplied_signature.ct_eq(expected_signature.as_slice()))
    {
        return Err(SignatureError::InvalidValue);
    }
    serde_json::from_slice(&payload).map_err(|_| SignatureError::InvalidValue)
}

fn signature(payload: &[u8], key: &str, purpose: &str) -> Result<Vec<u8>, SignatureError> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| SignatureError::InvalidKey)?;
    mac.update(purpose.as_bytes());
    mac.update(&[0]);
    mac.update(payload);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[must_use]
pub fn sha256(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

#[must_use]
pub fn constant_time_equal(left: &str, right: &str) -> bool {
    bool::from(sha256(left).ct_eq(&sha256(right)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn signs_plaintext_json_and_rejects_tampering() {
        let value = json!({"accessToken": "plain-token"});
        let signed = sign_json(&value, "signing-key", "test/session").unwrap();
        let encoded_payload = signed.split_once('.').unwrap().0;
        let decoded = URL_SAFE_NO_PAD.decode(encoded_payload).unwrap();
        assert!(String::from_utf8(decoded).unwrap().contains("plain-token"));
        assert_eq!(
            verify_json(&signed, "signing-key", "test/session").unwrap(),
            value
        );
        assert!(verify_json(&signed, "other-key", "test/session").is_err());
        assert!(verify_json(&signed, "signing-key", "other-purpose").is_err());
    }
}
