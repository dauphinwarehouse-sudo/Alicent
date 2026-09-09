use alicent_provider_wire::*;
use serde_json::json;
use std::{collections::BTreeSet, time::Duration};

fn allowed() -> BTreeSet<String> {
    BTreeSet::from(["read_scene".to_owned()])
}

fn request() -> Request {
    Request {
        model: "claude-fixture".into(),
        max_output_tokens: 1024,
        messages: vec![
            Message::System("You are a careful editor.".into()),
            Message::User("Read the scene.".into()),
        ],
        tools: vec![ToolSpec {
            name: "read_scene".into(),
            description: "Read one scene".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        }],
    }
}

#[test]
fn anthropic_request_supports_regular_and_streaming_modes() {
    let regular = build_anthropic_request(&request(), false).unwrap();
    assert_eq!(regular["model"], "claude-fixture");
    assert_eq!(regular["stream"], false);
    assert_eq!(regular["max_tokens"], 1024);
    assert_eq!(regular["system"][0]["type"], "text");
    assert_eq!(regular["messages"][0]["role"], "user");
    assert_eq!(regular["tools"][0]["input_schema"]["type"], "object");
    assert!(regular.get("store").is_none());

    let streaming = build_anthropic_request(&request(), true).unwrap();
    assert_eq!(streaming["stream"], true);
}

#[test]
fn anthropic_endpoint_and_timeout_policy_are_fail_closed() {
    let config = ProviderConfig::anthropic("anthropic", "claude-fixture");
    assert_eq!(
        config.anthropic_endpoint().unwrap().as_str(),
        "https://api.anthropic.com/v1/messages"
    );

    let mut invalid = config;
    invalid.timeouts.idle = Duration::from_millis(99);
    assert_eq!(
        invalid.anthropic_endpoint(),
        Err(ProviderConfigError::InvalidTimeout)
    );
}

#[test]
fn regular_message_normalizes_text_tool_use_and_usage() {
    let response =
        decode_anthropic_message(include_bytes!("fixtures/anthropic_message.json"), allowed())
            .unwrap();
    assert_eq!(response.text, "Привет ❄");
    assert_eq!(response.completion.reason, ChatStopReason::ToolCalls);
    assert_eq!(response.completion.tools.len(), 1);
    assert_eq!(response.completion.tools[0].id, "toolu_1");
    assert_eq!(response.completion.tools[0].name, "read_scene");
    assert_eq!(
        response.completion.tools[0].arguments,
        json!({"id": "scene-1"})
    );
    assert_eq!(
        response.completion.usage,
        Some(TokenUsage {
            input_tokens: 11,
            output_tokens: 7,
            total_tokens: 18,
        })
    );
}

#[test]
fn streaming_message_handles_chunk_boundaries_and_tool_use_events() {
    let fixture = include_bytes!("fixtures/anthropic_stream.sse");
    for chunk_size in [1, 7, 64, fixture.len()] {
        let mut decoder = AnthropicDecoder::new(allowed()).unwrap();
        let mut events = Vec::new();
        for chunk in fixture.chunks(chunk_size) {
            events.extend(decoder.push(chunk).unwrap());
        }
        assert_eq!(
            events,
            vec![
                ChatEvent::TextDelta("Привет ".into()),
                ChatEvent::TextDelta("❄".into()),
            ],
            "chunk size {chunk_size}"
        );
        let completion = decoder.finish().unwrap();
        assert_eq!(completion.reason, ChatStopReason::ToolCalls);
        assert_eq!(completion.tools.len(), 1);
        assert_eq!(completion.tools[0].id, "toolu_2");
        assert_eq!(completion.tools[0].name, "read_scene");
        assert_eq!(completion.tools[0].arguments, json!({"id": "scene-2"}));
        assert_eq!(
            completion.usage,
            Some(TokenUsage {
                input_tokens: 11,
                output_tokens: 7,
                total_tokens: 18,
            })
        );
    }
}

/// A future Anthropic build may add events, block types and cache usage: none
/// of that may turn an otherwise complete response into an error.
#[test]
fn unknown_events_and_blocks_are_ignored_and_cache_tokens_are_billed() {
    let stream = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_fixture_2\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-fixture\",\"content\":[],\"usage\":{\"input_tokens\":5,\"cache_read_input_tokens\":6}}}\n\n",
        "event: ping\n",
        "data: {\"type\":\"ping\"}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hidden\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_future\n",
        "data: {\"type\":\"message_future\",\"detail\":1}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let mut decoder = AnthropicDecoder::new(allowed()).unwrap();
    let events = decoder.push(stream.as_bytes()).unwrap();
    assert_eq!(events, vec![ChatEvent::TextDelta("ok".into())]);
    let completion = decoder.finish().unwrap();
    assert_eq!(completion.reason, ChatStopReason::Stop);
    assert!(completion.tools.is_empty());
    assert_eq!(
        completion.usage,
        Some(TokenUsage {
            input_tokens: 11,
            output_tokens: 3,
            total_tokens: 14,
        })
    );
}

#[test]
fn malformed_and_aborted_stream_fixtures_fail_closed() {
    let mut malformed = AnthropicDecoder::new(allowed()).unwrap();
    let mut malformed_error = None;
    for chunk in include_bytes!("fixtures/anthropic_malformed.sse").chunks(5) {
        match malformed.push(chunk) {
            Ok(_) => {}
            Err(error) => {
                malformed_error = Some(error);
                break;
            }
        }
    }
    assert_eq!(malformed_error, Some(WireError::InvalidResponse));
    assert_eq!(malformed.finish(), Err(WireError::Closed));

    let fixture = include_bytes!("fixtures/anthropic_aborted.sse");
    let mut truncated = AnthropicDecoder::new(allowed()).unwrap();
    for chunk in fixture.chunks(9) {
        truncated.push(chunk).unwrap();
    }
    assert_eq!(truncated.finish(), Err(WireError::TruncatedStream));

    let mut aborted = AnthropicDecoder::new(allowed()).unwrap();
    aborted.push(fixture).unwrap();
    aborted.cancel();
    assert_eq!(aborted.finish(), Err(WireError::Closed));
}

#[test]
fn response_and_allowlist_limits_are_enforced_before_parsing() {
    let oversized = vec![b'x'; MAX_RESPONSE_BYTES + 1];
    assert_eq!(
        decode_anthropic_message(&oversized, BTreeSet::new()),
        Err(WireError::LimitExceeded)
    );
    let names = (0..=ToolArguments::MAX_CALLS)
        .map(|index| format!("tool_{index}"))
        .collect();
    assert!(matches!(
        AnthropicDecoder::new(names),
        Err(WireError::InvalidRequest)
    ));
}
