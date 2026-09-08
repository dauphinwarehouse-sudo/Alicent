use alicent_provider_wire::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn decoder() -> ChatDecoder {
    ChatDecoder::new(["read_scene".into()].into_iter().collect()).unwrap()
}

fn chunk(delta: Value, reason: Value) -> Value {
    json!({
        "id":"chatcmpl_fixture",
        "object":"chat.completion.chunk",
        "model":"fixture-model",
        "choices":[{"index":0,"delta":delta,"finish_reason":reason}],
        "usage":null
    })
}

fn frame(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn stop(reason: &str) -> String {
    frame(chunk(json!({}), json!(reason)))
}

fn usage_chunk(input: Value, output: Value, total: Value) -> Value {
    let mut value = chunk(json!({}), Value::Null);
    value["choices"] = json!([]);
    value["usage"] = json!({
        "prompt_tokens":input,"completion_tokens":output,"total_tokens":total
    });
    value
}

fn call(index: u32, id: &str, name: &str, arguments: &str) -> Value {
    json!({
        "index":index,"id":id,"type":"function",
        "function":{"name":name,"arguments":arguments}
    })
}

fn continuation(index: u32, arguments: &str) -> Value {
    json!({"index":index,"function":{"arguments":arguments}})
}

fn text(events: Vec<ChatEvent>) -> String {
    events
        .into_iter()
        .map(|event| match event {
            ChatEvent::TextDelta(value) => value,
        })
        .collect()
}

fn rejected(value: Value, expected: WireError) {
    let mut stream = decoder();
    assert_eq!(stream.push(frame(value).as_bytes()), Err(expected));
    assert_eq!(stream.push(b""), Err(WireError::Closed));
    assert_eq!(stream.finish(), Err(WireError::Closed));
}

#[test]
fn text_usage_and_terminal_markers_survive_every_transport_chunk_size() {
    let input = [
        "\u{feff}: keepalive\r\n".into(),
        frame(chunk(json!({"role":"assistant","content":""}), Value::Null)),
        frame(chunk(json!({"content":"Элина ❄ 雪"}), Value::Null)),
        stop("stop"),
        frame(usage_chunk(json!(12), json!(3), json!(15))),
        "data: [DONE]\n\n".into(),
    ]
    .concat();
    for size in 1..=input.len() {
        let mut stream = decoder();
        let mut events = Vec::new();
        for bytes in input.as_bytes().chunks(size) {
            events.extend(stream.push(bytes).unwrap());
        }
        let result = stream.finish().unwrap();
        assert_eq!(text(events), "Элина ❄ 雪");
        assert_eq!(result.reason, ChatStopReason::Stop);
        assert!(result.tools.is_empty());
        assert_eq!(
            result.usage,
            Some(TokenUsage {
                input_tokens: 12,
                output_tokens: 3,
                total_tokens: 15,
            })
        );
        assert_eq!(stream.finish(), Err(WireError::Closed));
    }
}

#[test]
fn interleaved_calls_are_only_returned_by_finish_after_done() {
    let mut stream = decoder();
    let initial = json!({"tool_calls":[
        call(1, "call_b", "read_scene", "{\"id\":"),
        call(0, "call_a", "read_scene", "{")
    ]});
    assert!(stream
        .push(frame(chunk(initial, Value::Null)).as_bytes())
        .unwrap()
        .is_empty());
    let more = json!({"tool_calls":[
        continuation(0, "\"id\":\"Сцена 雪\"}"),
        continuation(1, "\"second\"}")
    ]});
    assert!(stream
        .push(frame(chunk(more, Value::Null)).as_bytes())
        .unwrap()
        .is_empty());
    assert!(stream
        .push(stop("tool_calls").as_bytes())
        .unwrap()
        .is_empty());
    assert!(stream.push(b"data: [DONE]\n\n").unwrap().is_empty());
    let result = stream.finish().unwrap();
    assert_eq!(result.reason, ChatStopReason::ToolCalls);
    assert_eq!(result.usage, None);
    assert_eq!(result.tools.len(), 2);
    assert_eq!(result.tools[0].id, "call_a");
    assert_eq!(result.tools[0].arguments, json!({"id":"Сцена 雪"}));
    assert_eq!(result.tools[1].id, "call_b");
    assert_eq!(result.tools[1].arguments, json!({"id":"second"}));
}

#[test]
fn every_truncated_prefix_fails_without_releasing_tools() {
    let start = json!({"tool_calls":[call(0, "call", "read_scene", "{}") ]});
    let input = [
        frame(chunk(start, Value::Null)),
        stop("tool_calls"),
        "data: [DONE]\n\n".into(),
    ]
    .concat();
    for end in 0..input.len() {
        let mut stream = decoder();
        assert!(stream.push(&input.as_bytes()[..end]).unwrap().is_empty());
        assert_eq!(stream.finish(), Err(WireError::TruncatedStream));
        assert_eq!(stream.finish(), Err(WireError::Closed));
    }
}

#[test]
fn done_without_finish_reason_is_not_success() {
    let mut stream = decoder();
    assert_eq!(
        stream.push(b"data: [DONE]\n\n"),
        Err(WireError::TruncatedStream)
    );
    assert_eq!(stream.finish(), Err(WireError::Closed));
}

#[test]
fn a_terminal_marker_cannot_hide_a_later_or_partial_event() {
    let complete = [stop("stop"), "data: [DONE]\n\n".into()].concat();
    let extra = frame(chunk(json!({"content":"late"}), Value::Null));
    let mut stream = decoder();
    assert_eq!(
        stream.push(format!("{complete}{extra}").as_bytes()),
        Err(WireError::InvalidResponse)
    );
    let mut stream = decoder();
    stream.push(complete.as_bytes()).unwrap();
    assert_eq!(
        stream.push(extra.as_bytes()),
        Err(WireError::InvalidResponse)
    );
    let mut stream = decoder();
    stream.push(complete.as_bytes()).unwrap();
    stream.push(b"data: partial").unwrap();
    assert_eq!(stream.finish(), Err(WireError::TruncatedStream));
}

#[test]
fn length_filter_and_refusal_fail_closed() {
    for reason in ["length", "content_filter"] {
        let mut stream = decoder();
        let tools = json!({"tool_calls":[call(0, "call", "read_scene", "{}") ]});
        stream
            .push(frame(chunk(tools, Value::Null)).as_bytes())
            .unwrap();
        assert_eq!(
            stream.push(stop(reason).as_bytes()),
            Err(WireError::IncompleteResponse)
        );
        assert_eq!(stream.finish(), Err(WireError::Closed));
    }
    rejected(
        chunk(json!({"refusal":"private refusal details"}), Value::Null),
        WireError::IncompleteResponse,
    );
}

#[test]
fn stop_reason_must_match_whether_tools_exist() {
    rejected(
        chunk(json!({}), json!("tool_calls")),
        WireError::InvalidResponse,
    );
    let mut stream = decoder();
    let tools = json!({"tool_calls":[call(0, "call", "read_scene", "{}") ]});
    stream
        .push(frame(chunk(tools, Value::Null)).as_bytes())
        .unwrap();
    assert_eq!(
        stream.push(stop("stop").as_bytes()),
        Err(WireError::InvalidResponse)
    );
    rejected(
        chunk(json!({}), json!("unknown")),
        WireError::InvalidResponse,
    );
}

#[test]
fn malformed_or_ambiguous_tool_json_rejects_the_entire_completion() {
    for arguments in [
        "{",
        "[]",
        "null",
        "{} {}",
        "{\"id\":\"first\",\"id\":\"second\"}",
        "{\"nested\":[{\"x\":1,\"x\":2}]}",
    ] {
        let mut stream = decoder();
        let tools = json!({"tool_calls":[
            call(0, "ok", "read_scene", "{}"),
            call(1, "bad", "read_scene", arguments)
        ]});
        let input = [
            frame(chunk(tools, Value::Null)),
            stop("tool_calls"),
            "data: [DONE]\n\n".into(),
        ]
        .concat();
        assert!(stream.push(input.as_bytes()).unwrap().is_empty());
        assert_eq!(stream.finish(), Err(WireError::InvalidArguments));
        assert_eq!(stream.finish(), Err(WireError::Closed));
    }
}

#[test]
fn tool_metadata_cannot_mutate_mid_stream() {
    for replacement in [
        call(0, "different", "read_scene", "{}"),
        call(0, "call", "other_name", "{}"),
        json!({"index":0,"type":"custom","function":{"arguments":"{}"}}),
    ] {
        let mut stream = decoder();
        let initial = json!({"tool_calls":[call(0, "call", "read_scene", "")]});
        stream
            .push(frame(chunk(initial, Value::Null)).as_bytes())
            .unwrap();
        let more = chunk(json!({"tool_calls":[replacement]}), Value::Null);
        assert!(stream.push(frame(more).as_bytes()).is_err());
        assert_eq!(stream.finish(), Err(WireError::Closed));
    }
}

#[test]
fn unknown_tool_duplicate_identity_and_missing_metadata_are_rejected() {
    let unknown = chunk(
        json!({"tool_calls":[call(0, "call", "delete_all", "{}")] }),
        Value::Null,
    );
    rejected(unknown, WireError::InvalidArguments);
    let duplicates = chunk(
        json!({"tool_calls":[
            call(0, "same", "read_scene", "{}"),
            call(1, "same", "read_scene", "{}")
        ]}),
        Value::Null,
    );
    rejected(duplicates, WireError::InvalidArguments);
    rejected(
        chunk(json!({"tool_calls":[continuation(0, "{}")] }), Value::Null),
        WireError::InvalidResponse,
    );
    rejected(
        chunk(
            json!({"tool_calls":[
                call(0, "a", "read_scene", "{}"),
                call(0, "b", "read_scene", "{}")
            ]}),
            Value::Null,
        ),
        WireError::InvalidResponse,
    );
}

#[test]
fn a_single_response_identity_and_model_are_required() {
    for field in ["id", "model"] {
        let mut stream = decoder();
        stream
            .push(frame(chunk(json!({}), Value::Null)).as_bytes())
            .unwrap();
        let mut changed = chunk(json!({"content":"different"}), Value::Null);
        changed[field] = json!("different");
        assert_eq!(
            stream.push(frame(changed).as_bytes()),
            Err(WireError::InvalidResponse)
        );
        assert_eq!(stream.finish(), Err(WireError::Closed));
    }
    let mut invalid = chunk(json!({}), Value::Null);
    invalid["object"] = json!("chat.completion");
    rejected(invalid, WireError::InvalidResponse);
}

#[test]
fn malformed_shapes_and_unexpected_roles_are_not_treated_as_text() {
    for value in [
        json!([]),
        json!({}),
        chunk(json!({"role":"system"}), Value::Null),
        chunk(json!({"content":42}), Value::Null),
        chunk(json!({"refusal":true}), Value::Null),
        chunk(json!({"tool_calls":{}}), Value::Null),
        chunk(Value::Null, Value::Null),
    ] {
        rejected(value, WireError::InvalidResponse);
    }
}

#[test]
fn multiple_choices_and_unsupported_modalities_are_explicit_errors() {
    let mut multiple = chunk(json!({}), Value::Null);
    let second = multiple["choices"][0].clone();
    multiple["choices"].as_array_mut().unwrap().push(second);
    rejected(multiple, WireError::UnsupportedResponse);
    let mut other_choice = chunk(json!({}), Value::Null);
    other_choice["choices"][0]["index"] = json!(1);
    rejected(other_choice, WireError::UnsupportedResponse);
    for delta in [
        json!({"reasoning_content":"opaque reasoning"}),
        json!({"audio":{"data":"unsupported"}}),
        json!({"function_call":{"name":"legacy"}}),
    ] {
        rejected(chunk(delta, Value::Null), WireError::UnsupportedResponse);
    }
}

#[test]
fn usage_requires_nonnegative_integers_consistent_totals_and_successful_stop() {
    rejected(
        usage_chunk(json!(1), json!(2), json!(3)),
        WireError::InvalidResponse,
    );
    for (input, output, total) in [
        (json!(-1), json!(2), json!(1)),
        (json!(1.5), json!(2), json!(3.5)),
        (Value::Null, json!(2), json!(2)),
        (json!(1), json!(2), json!(4)),
        (json!(u64::MAX), json!(1), json!(0)),
    ] {
        let mut stream = decoder();
        stream.push(stop("stop").as_bytes()).unwrap();
        let value = frame(usage_chunk(input, output, total));
        assert_eq!(
            stream.push(value.as_bytes()),
            Err(WireError::InvalidResponse)
        );
    }
}

#[test]
fn duplicate_usage_and_repeated_finish_reason_are_invalid() {
    let mut stream = decoder();
    stream.push(stop("stop").as_bytes()).unwrap();
    let value = frame(usage_chunk(json!(0), json!(0), json!(0)));
    stream.push(value.as_bytes()).unwrap();
    assert_eq!(
        stream.push(value.as_bytes()),
        Err(WireError::InvalidResponse)
    );
    let mut stream = decoder();
    stream.push(stop("stop").as_bytes()).unwrap();
    assert_eq!(
        stream.push(stop("stop").as_bytes()),
        Err(WireError::InvalidResponse)
    );
}

#[test]
fn usage_can_be_on_the_final_choice_and_missing_usage_stays_unknown() {
    let mut terminal = chunk(json!({}), json!("stop"));
    terminal["usage"] = json!({"prompt_tokens":1,"completion_tokens":0,"total_tokens":1});
    let mut stream = decoder();
    stream.push(frame(terminal).as_bytes()).unwrap();
    stream.push(b"data: [DONE]\n\n").unwrap();
    assert_eq!(stream.finish().unwrap().usage.unwrap().input_tokens, 1);
    let mut stream = decoder();
    stream.push(stop("stop").as_bytes()).unwrap();
    stream.push(b"data: [DONE]\n\n").unwrap();
    assert_eq!(stream.finish().unwrap().usage, None);
}

#[test]
fn cancel_and_invalid_utf8_permanently_close_the_response() {
    let mut stream = decoder();
    let tools = json!({"tool_calls":[call(0, "call", "read_scene", "{}")] });
    stream
        .push(frame(chunk(tools, Value::Null)).as_bytes())
        .unwrap();
    stream.cancel();
    assert_eq!(stream.finish(), Err(WireError::Closed));
    assert_eq!(stream.push(b"data: [DONE]\n\n"), Err(WireError::Closed));
    let mut stream = decoder();
    assert_eq!(stream.push(b"data: \xff\n\n"), Err(WireError::InvalidUtf8));
    assert_eq!(stream.finish(), Err(WireError::Closed));
}

#[test]
fn provider_errors_and_debug_output_do_not_include_payloads() {
    let secret = "fixture-private-manuscript-and-key";
    let mut stream = decoder();
    let error = stream
        .push(frame(json!({"error":{"message":secret}})).as_bytes())
        .unwrap_err();
    assert_eq!(error, WireError::ProviderFailure);
    assert!(!format!("{error:?} {error}").contains(secret));
    let event = ChatEvent::TextDelta(secret.into());
    assert!(!format!("{event:?}").contains(secret));
    let result = ChatCompletion {
        reason: ChatStopReason::ToolCalls,
        tools: vec![ToolProposal {
            id: "id".into(),
            name: "read_scene".into(),
            arguments: json!({"text":secret}),
        }],
        usage: None,
    };
    assert!(!format!("{result:?}").contains(secret));
    let mut stream = decoder();
    assert_eq!(
        stream.push(format!("event: error\ndata: {secret}\n\n").as_bytes()),
        Err(WireError::ProviderFailure)
    );
}

#[test]
fn duplicate_json_fields_in_provider_envelopes_are_rejected() {
    let valid = chunk(json!({"content":"first"}), Value::Null).to_string();
    let ambiguous = valid.replace(
        "\"content\":\"first\"",
        "\"content\":\"first\",\"content\":\"second\"",
    );
    assert_ne!(valid, ambiguous);
    let mut stream = decoder();
    assert_eq!(
        stream.push(format!("data: {ambiguous}\n\n").as_bytes()),
        Err(WireError::InvalidResponse)
    );
}

#[test]
fn per_tool_bytes_and_transport_limits_also_apply_to_chat() {
    let mut stream = decoder();
    let initial = json!({"tool_calls":[call(0, "call", "read_scene", "")]});
    stream
        .push(frame(chunk(initial, Value::Null)).as_bytes())
        .unwrap();
    let delta = json!({"tool_calls":[continuation(0, &"x".repeat(400_000))]});
    let bytes = frame(chunk(delta, Value::Null));
    stream.push(bytes.as_bytes()).unwrap();
    stream.push(bytes.as_bytes()).unwrap();
    assert_eq!(stream.push(bytes.as_bytes()), Err(WireError::LimitExceeded));
    assert_eq!(stream.finish(), Err(WireError::Closed));
    let mut stream = decoder();
    assert_eq!(
        stream.push(&vec![b'x'; SseDecoder::MAX_CHUNK + 1]),
        Err(WireError::LimitExceeded)
    );
}

#[test]
fn invalid_allowlists_are_rejected_before_decoding() {
    assert!(matches!(
        ChatDecoder::new(["bad name".into()].into_iter().collect()),
        Err(WireError::InvalidRequest)
    ));
    let names = (0..65).map(|index| format!("tool_{index}")).collect();
    assert!(matches!(
        ChatDecoder::new(names),
        Err(WireError::InvalidRequest)
    ));
    let mut empty = ChatDecoder::new(BTreeSet::new()).unwrap();
    empty.push(stop("stop").as_bytes()).unwrap();
    empty.push(b"data: [DONE]\n\n").unwrap();
    assert!(empty.finish().unwrap().tools.is_empty());
}
