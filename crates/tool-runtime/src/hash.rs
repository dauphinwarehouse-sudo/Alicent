use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) fn canonical_json(value: &Value) -> Vec<u8> {
    let mut output = Vec::new();
    write_canonical(value, &mut output);
    output
}

fn write_canonical(value: &Value, output: &mut Vec<u8>) {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
        Value::String(value) => output.extend_from_slice(
            serde_json::to_string(value)
                .expect("JSON strings are serializable")
                .as_bytes(),
        ),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical(value, output);
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    serde_json::to_string(key)
                        .expect("JSON keys are serializable")
                        .as_bytes(),
                );
                output.push(b':');
                write_canonical(value, output);
            }
            output.push(b'}');
        }
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The single definition of the authorization payload hash.
///
/// The approval engine and the executor must hash exactly the same bytes: if
/// the two constructions ever diverged, a payload swapped between
/// authorization and execution would stop being detected.
pub(crate) fn payload_hash(tool: &str, version: u32, arguments: &Value) -> String {
    sha256_hex(&canonical_json(&json!({
        "tool": tool,
        "version": version,
        "arguments": arguments,
    })))
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn implements_sha256_test_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn canonicalizes_object_key_order() {
        assert_eq!(
            canonical_json(&json!({"z": 1, "a": {"b": 2, "a": 1}})),
            br#"{"a":{"a":1,"b":2},"z":1}"#
        );
    }

    #[test]
    fn payload_hash_ignores_key_order_but_not_version() {
        assert_eq!(
            payload_hash("tool", 1, &json!({"a": 1, "b": 2})),
            payload_hash("tool", 1, &json!({"b": 2, "a": 1}))
        );
        assert_ne!(
            payload_hash("tool", 1, &json!({"a": 1})),
            payload_hash("tool", 2, &json!({"a": 1}))
        );
    }

    #[test]
    fn constant_time_eq_compares_content_and_length() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
