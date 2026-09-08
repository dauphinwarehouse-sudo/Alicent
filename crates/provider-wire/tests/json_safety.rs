use alicent_provider_wire::*;
use serde_json::json;
use std::collections::BTreeSet;

fn allow() -> BTreeSet<String> {
    ["read_scene".into()].into_iter().collect()
}

#[test]
fn duplicate_keys_are_rejected_at_every_depth_including_escaped_names() {
    for input in [
        r#"{"id":"first","id":"second"}"#,
        r#"{"nested":{"id":1,"id":2}}"#,
        r#"{"items":[{"id":1,"id":2}]}"#,
        r#"{"id":1,"\u0069d":2}"#,
        r#"{"items":[{"x":{"y":0,"y":1}}]}"#,
    ] {
        let mut arguments = ToolArguments::default();
        arguments.start(0, "call", "read_scene").unwrap();
        arguments.append(0, input).unwrap();
        assert_eq!(arguments.finish(&allow()), Err(WireError::InvalidArguments));
        assert_eq!(arguments.finish(&allow()), Err(WireError::Closed));
    }
}

#[test]
fn repeated_names_in_distinct_objects_and_all_json_value_types_remain_valid() {
    let expected = json!({
        "objects":[{"id":1},{"id":2}],
        "string":"雪 ❄", "null":null, "bool":true,
        "negative":-2, "positive":u64::MAX, "float":1.25
    });
    let mut arguments = ToolArguments::default();
    arguments.start(0, "call", "read_scene").unwrap();
    arguments.append(0, &expected.to_string()).unwrap();
    assert_eq!(arguments.finish(&allow()).unwrap()[0].arguments, expected);
}

#[test]
fn recursion_and_trailing_data_remain_rejected() {
    let deep = format!("{{\"nested\":{}0{}}}", "[".repeat(130), "]".repeat(130));
    for input in [deep.as_str(), "{} {}", "{\"x\":NaN}", "{\"x\":1e999}"] {
        let mut arguments = ToolArguments::default();
        arguments.start(0, "call", "read_scene").unwrap();
        arguments.append(0, input).unwrap();
        assert_eq!(arguments.finish(&allow()), Err(WireError::InvalidArguments));
    }
}

#[test]
fn ambiguous_later_call_discards_a_valid_earlier_call_without_exposing_its_data() {
    let mut arguments = ToolArguments::default();
    arguments.start(0, "valid", "read_scene").unwrap();
    arguments
        .append(0, r#"{"text":"fixture-private-text"}"#)
        .unwrap();
    arguments.start(1, "ambiguous", "read_scene").unwrap();
    arguments
        .append(1, r#"{"scope":"one","scope":"two"}"#)
        .unwrap();
    let error = arguments.finish(&allow()).unwrap_err();
    assert_eq!(error, WireError::InvalidArguments);
    assert!(!format!("{error:?} {error}").contains("fixture-private-text"));
}
