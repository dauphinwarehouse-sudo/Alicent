use alicent_provider_wire::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn frame(event: &str, value: Value) -> String { format!("event: {event}\ndata: {value}\n\n") }
fn start() -> String { frame("message_start", json!({"type":"message_start","message":{"id":"msg_fixture","type":"message","role":"assistant","model":"claude-fixture","usage":{"input_tokens":11,"output_tokens":0}}})) }
fn decoder() -> AnthropicDecoder { AnthropicDecoder::new(BTreeSet::from(["read_scene".into()])).unwrap() }
fn fixture() -> String {
    [
        start(),
        frame("content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})),
        frame("content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Привет 雪"}})),
        frame("content_block_stop", json!({"type":"content_block_stop","index":0})),
        frame("content_block_start", json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_scene","input":{}}})),
        frame("content_block_delta", json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"id\":"}})),
        frame("content_block_delta", json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"scene-1\"}"}})),
        frame("content_block_stop", json!({"type":"content_block_stop","index":1})),
        frame("message_delta", json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}})),
        frame("message_stop", json!({"type":"message_stop"})),
    ].concat()
}

#[test]
fn normalizes_text_tool_use_and_usage_at_every_chunk_size() {
    let input = fixture();
    for size in 1..=input.len() {
        let mut decoder = decoder();
        let mut text = String::new();
        for chunk in input.as_bytes().chunks(size) {
            for event in decoder.push(chunk).unwrap() {
                match event { ChatEvent::TextDelta(delta) => text.push_str(&delta) }
            }
        }
        let result = decoder.finish().unwrap();
        assert_eq!(text, "Привет 雪");
        assert_eq!(result.reason, ChatStopReason::ToolCalls);
        assert_eq!(result.tools[0].arguments, json!({"id":"scene-1"}));
        assert_eq!(result.usage, Some(TokenUsage { input_tokens: 11, output_tokens: 7, total_tokens: 18 }));
    }
}

#[test]
fn every_aborted_prefix_is_truncated_and_releases_no_tools() {
    let input = fixture();
    for end in 0..input.len() {
        let mut decoder = decoder();
        decoder.push(&input.as_bytes()[..end]).unwrap();
        assert_eq!(decoder.finish(), Err(WireError::TruncatedStream));
        assert_eq!(decoder.finish(), Err(WireError::Closed));
    }
}

#[test]
fn malformed_ambiguous_and_unapproved_events_fail_closed() {
    let cases = [
        format!("{}event: ping\ndata: {{\"type\":\"message_stop\"}}\n\n", start()),
        format!("{}event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"index\":1,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n", start()),
        [start(), frame("content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"delete_all","input":{}}}))].concat(),
    ];
    for input in cases {
        let mut decoder = decoder();
        assert!(decoder.push(input.as_bytes()).is_err());
        assert_eq!(decoder.finish(), Err(WireError::Closed));
    }
    let mut decoder = decoder();
    decoder.push(start().as_bytes()).unwrap();
    decoder.cancel();
    assert_eq!(decoder.push(b""), Err(WireError::Closed));
}
