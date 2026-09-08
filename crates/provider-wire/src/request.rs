use crate::{
    tools::{valid_id, valid_name},
    Result, WireError,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
}
// Deliberately no Debug on transcript/config: future logs should use metadata, not manuscript.
#[derive(serde::Serialize)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(serde::Serialize)]
pub enum Message {
    System(String),
    User(String),
    Assistant {
        text: String,
        calls: Vec<Call>,
    },
    ToolResult {
        call_id: String,
        output: Value,
        is_error: bool,
    },
}
#[derive(serde::Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
#[derive(serde::Serialize)]
pub struct Request {
    pub model: String,
    pub max_output_tokens: u32,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
}
const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
struct ByteBudget(usize);
impl std::io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::Error::other("request limit"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded(value: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer(ByteBudget(MAX_REQUEST_BYTES), value)
        .map_err(|_| WireError::LimitExceeded)
}
fn validate(request: &Request) -> Result<()> {
    // Count serialized bytes without duplicating a giant manuscript into a temporary string.
    bounded(request)?;
    if request.model.trim().is_empty()
        || request.model.len() > 256
        || request.model.chars().any(char::is_control)
        || !(1..=128_000).contains(&request.max_output_tokens)
        || request.messages.is_empty()
        || request.messages.len() > 4096
        || request.tools.len() > 64
    {
        return Err(WireError::InvalidRequest);
    }
    let mut names = BTreeSet::new();
    for tool in &request.tools {
        if !valid_name(&tool.name)
            || !names.insert(tool.name.as_str())
            || tool.description.len() > 16_384
            || tool.input_schema.get("type").and_then(Value::as_str) != Some("object")
        {
            return Err(WireError::InvalidRequest);
        }
    }
    let mut pending = BTreeSet::new();
    let mut used = BTreeSet::new();
    let mut dialogue = false;
    for message in &request.messages {
        match message {
            Message::System(text) if text.trim().is_empty() => {
                return Err(WireError::InvalidRequest)
            }
            Message::System(_) if !dialogue => {}
            Message::System(_) => return Err(WireError::InvalidTranscript),
            Message::ToolResult { call_id, .. } => {
                if !pending.remove(call_id.as_str()) {
                    return Err(WireError::InvalidTranscript);
                }
            }
            Message::User(text) => {
                if text.trim().is_empty() {
                    return Err(WireError::InvalidRequest);
                }
                if !pending.is_empty() {
                    return Err(WireError::InvalidTranscript);
                }
                dialogue = true;
            }
            Message::Assistant { text, calls } => {
                if text.is_empty() && calls.is_empty() {
                    return Err(WireError::InvalidRequest);
                }
                if !pending.is_empty() || !dialogue || calls.len() > 64 {
                    return Err(WireError::InvalidTranscript);
                }
                for call in calls {
                    if !valid_id(&call.id)
                        || !names.contains(call.name.as_str())
                        || !call.arguments.is_object()
                        || !used.insert(call.id.as_str())
                    {
                        return Err(WireError::InvalidTranscript);
                    }
                    pending.insert(call.id.as_str());
                }
            }
        }
    }
    if !dialogue || !pending.is_empty() {
        return Err(WireError::InvalidTranscript);
    }
    Ok(())
}
/// Builds body only. Transport must allowlist endpoint, retrieve secret natively and apply budgets.
/// Text+client-tools baseline; vision, reasoning blocks and server tools are intentionally absent.
pub fn build_request(protocol: Protocol, request: &Request) -> Result<Value> {
    validate(request)?;
    let mut messages = Vec::new();
    let mut system = Vec::new();
    for message in &request.messages {
        match (protocol,message) {
            (Protocol::AnthropicMessages,Message::System(text))=>system.push(json!({"type":"text","text":text})),
            (_,Message::System(text))=>messages.push(json!({"role":"system","content":text})),
            (_,Message::User(text))=>messages.push(json!({"role":"user","content":text})),
            (Protocol::OpenAiChat,Message::Assistant{text,calls})=>{
                let mut msg=json!({"role":"assistant","content":if text.is_empty(){Value::Null}else{json!(text)}});
                if !calls.is_empty(){msg["tool_calls"]=json!(calls.iter().map(|c|json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments.to_string()}})).collect::<Vec<_>>());} messages.push(msg);
            },
            (Protocol::OpenAiResponses,Message::Assistant{text,calls})=>{
                if !text.is_empty(){messages.push(json!({"role":"assistant","content":text}));}
                for call in calls {messages.push(json!({"type":"function_call","call_id":call.id,"name":call.name,"arguments":call.arguments.to_string()}));}
            },
            (Protocol::AnthropicMessages,Message::Assistant{text,calls})=>{
                let mut blocks=Vec::new();if !text.is_empty(){blocks.push(json!({"type":"text","text":text}));}
                for call in calls {blocks.push(json!({"type":"tool_use","id":call.id,"name":call.name,"input":call.arguments}));}
                if blocks.is_empty(){return Err(WireError::InvalidRequest);}messages.push(json!({"role":"assistant","content":blocks}));
            },
            (Protocol::OpenAiChat,Message::ToolResult{call_id,output,is_error})=>messages.push(json!({"role":"tool","tool_call_id":call_id,"content":json!({"output":output,"is_error":is_error}).to_string()})),
            (Protocol::OpenAiResponses,Message::ToolResult{call_id,output,is_error})=>messages.push(json!({"type":"function_call_output","call_id":call_id,"output":json!({"output":output,"is_error":is_error}).to_string()})),
            (Protocol::AnthropicMessages,Message::ToolResult{call_id,output,is_error})=>{
                let block=json!({"type":"tool_result","tool_use_id":call_id,"content":output.to_string(),"is_error":is_error});
                // Parallel tool results belong to the same following user turn, before any text.
                if let Some(last)=messages.last_mut().filter(|m|m["role"]=="user" && m["content"].is_array()) {last["content"].as_array_mut().unwrap().push(block);}else{messages.push(json!({"role":"user","content":[block]}));}
            },
        }
    }
    let tools:Vec<_>=request.tools.iter().map(|t|match protocol{
        Protocol::OpenAiChat=>json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}}),
        Protocol::OpenAiResponses=>json!({"type":"function","name":t.name,"description":t.description,"parameters":t.input_schema,"strict":false}),
        Protocol::AnthropicMessages=>json!({"name":t.name,"description":t.description,"input_schema":t.input_schema}),
    }).collect();
    let mut result = match protocol {
        Protocol::OpenAiChat => {
            json!({"model":request.model,"messages":messages,"max_completion_tokens":request.max_output_tokens,"stream":true,"stream_options":{"include_usage":true},"store":false})
        }
        Protocol::OpenAiResponses => {
            json!({"model":request.model,"input":messages,"max_output_tokens":request.max_output_tokens,"stream":true,"store":false})
        }
        Protocol::AnthropicMessages => {
            let mut body = json!({"model":request.model,"messages":messages,"max_tokens":request.max_output_tokens,"stream":true});
            if !system.is_empty() {
                body["system"] = json!(system);
            }
            body
        }
    };
    if !tools.is_empty() {
        result["tools"] = json!(tools);
    }
    bounded(&result)?;
    Ok(result)
}
