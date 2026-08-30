use crate::core::AppResult;

use super::sha256;

pub fn new_record_id() -> AppResult<String> {
    Ok(uuid::Uuid::new_v4().to_string())
}

pub fn derived_record_id(namespace: &str, identity: &str) -> String {
    let seed = format!("codex-router/record-id/v1\0{namespace}\0{identity}");
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&sha256(&seed)[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format_uuid(bytes)
}

pub fn valid_record_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || bytes[8] != b'-'
        || bytes[13] != b'-'
        || bytes[18] != b'-'
        || bytes[23] != b'-'
        || !matches!(bytes[14], b'4' | b'8')
        || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
    {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| {
        matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
    })
}

fn format_uuid(bytes: [u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_valid_random_and_stable_derived_ids() {
        let random = new_record_id().unwrap();
        assert!(valid_record_id(&random));
        assert_eq!(random.as_bytes()[14], b'4');

        let first = derived_record_id("api-key", "client");
        let second = derived_record_id("api-key", "client");
        assert_eq!(first, second);
        assert!(valid_record_id(&first));
        assert_eq!(first.as_bytes()[14], b'8');
        assert_ne!(first, derived_record_id("other-api-key", "client"));
    }
}
