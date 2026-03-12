use crate::event::{AgentEvent, AgentKind};
use crate::parse::{tool_summary, EventParser};
use serde_json::Value;

enum BlockState {
    None,
    Text,
    ToolUse { name: String, json_buf: String },
    Thinking,
}

pub struct ClaudeParser {
    block_state: BlockState,
    saw_stream_events: bool,
    debug: bool,
}

impl ClaudeParser {
    pub fn new(debug: bool) -> Self {
        Self {
            block_state: BlockState::None,
            saw_stream_events: false,
            debug,
        }
    }

    fn parse_stream_event(&mut self, v: &Value) -> Vec<AgentEvent> {
        self.saw_stream_events = true;
        let event = match v.get("event") {
            Some(e) => e,
            None => return vec![],
        };
        let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match event_type {
            "content_block_start" => {
                let cb = event.get("content_block").unwrap_or(&Value::Null);
                let block_type = cb.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        self.block_state = BlockState::Text;
                        vec![]
                    }
                    "thinking" => {
                        self.block_state = BlockState::Thinking;
                        vec![AgentEvent::ThinkingStart]
                    }
                    "tool_use" => {
                        let name = cb
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let events = vec![AgentEvent::ToolStart {
                            tool_name: name.clone(),
                        }];
                        self.block_state = BlockState::ToolUse {
                            name,
                            json_buf: String::new(),
                        };
                        events
                    }
                    other => {
                        if self.debug {
                            eprintln!("debug: unknown claude content_block_start type: {}", other);
                        }
                        vec![]
                    }
                }
            }
            "content_block_delta" => {
                let delta = event.get("delta").unwrap_or(&Value::Null);
                let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match delta_type {
                    "text_delta" => {
                        let text = delta.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        vec![AgentEvent::TextDelta(text.to_string())]
                    }
                    "thinking_delta" => {
                        let thinking = delta
                            .get("thinking")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        vec![AgentEvent::ThinkingDelta(thinking.to_string())]
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(|p| p.as_str())
                            .unwrap_or("");
                        if let BlockState::ToolUse { json_buf, .. } = &mut self.block_state {
                            json_buf.push_str(partial);
                        }
                        vec![]
                    }
                    "signature_delta" => vec![],
                    other => {
                        if self.debug {
                            eprintln!("debug: unknown claude content_block_delta type: {}", other);
                        }
                        vec![]
                    },
                }
            }
            "content_block_stop" => {
                let old_state = std::mem::replace(&mut self.block_state, BlockState::None);
                match old_state {
                    BlockState::Thinking => vec![AgentEvent::ThinkingEnd],
                    BlockState::ToolUse { name, json_buf } => {
                        let input: Value =
                            serde_json::from_str(&json_buf).unwrap_or(Value::Null);
                        let summary = tool_summary(&name, &input);
                        vec![AgentEvent::ToolReady {
                            tool_name: name,
                            input_summary: summary,
                        }]
                    }
                    _ => vec![],
                }
            }
            "message_start" | "message_delta" | "message_stop" | "ping" | "error" => vec![],
            other => {
                if self.debug {
                    eprintln!("debug: unknown claude stream event type: {}", other);
                }
                vec![]
            }
        }
    }

    fn parse_assistant(&mut self, v: &Value) -> Vec<AgentEvent> {
        if self.saw_stream_events {
            // Stream events already emitted everything; reset flag for next turn
            self.saw_stream_events = false;
            return vec![];
        }

        let mut events = vec![];
        let content = match v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            Some(c) => c,
            None => return events,
        };

        for block in content {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match block_type {
                "text" => {
                    let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    events.push(AgentEvent::TextComplete(text.to_string()));
                }
                "tool_use" => {
                    let name = block
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let input = block.get("input").unwrap_or(&Value::Null);
                    let summary = tool_summary(&name, input);
                    events.push(AgentEvent::ToolStart {
                        tool_name: name.clone(),
                    });
                    events.push(AgentEvent::ToolReady {
                        tool_name: name,
                        input_summary: summary,
                    });
                }
                "thinking" => {
                    let thinking = block
                        .get("thinking")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    events.push(AgentEvent::ThinkingStart);
                    events.push(AgentEvent::ThinkingDelta(thinking.to_string()));
                    events.push(AgentEvent::ThinkingEnd);
                }
                "tool_result" => {}
                other => {
                    if self.debug {
                        eprintln!("debug: unknown claude assistant block type: {}", other);
                    }
                }
            }
        }
        events
    }

    fn parse_user(&self, v: &Value) -> Vec<AgentEvent> {
        let mut events = vec![];
        // content may be at top level or nested under "message"
        let content = v
            .get("content")
            .or_else(|| v.get("message").and_then(|m| m.get("content")));

        // content can be a string or array
        let blocks = match content {
            Some(Value::Array(arr)) => arr.clone(),
            _ => return events,
        };

        for block in &blocks {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if block_type == "tool_result" {
                let is_error = block
                    .get("is_error")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                let content_val = block.get("content");
                let content_str = match content_val {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                };
                events.push(AgentEvent::ToolResult {
                    is_error,
                    content: content_str,
                });
            }
        }
        events
    }

    fn parse_result(&self, v: &Value) -> Vec<AgentEvent> {
        let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
        let success = subtype == "success";
        let is_error = v
            .get("is_error")
            .and_then(|e| e.as_bool())
            .unwrap_or(!success);

        let error_type = if !success {
            Some(subtype.to_string())
        } else {
            None
        };

        let usage = v.get("usage");
        vec![AgentEvent::SessionEnd {
            success: !is_error,
            error_type,
            error_message: None,
            num_turns: v.get("num_turns").and_then(|n| n.as_u64()).map(|n| n as u32),
            duration_ms: v.get("duration_ms").and_then(|d| d.as_u64()),
            api_duration_ms: v.get("duration_api_ms").and_then(|d| d.as_u64()),
            cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()),
            input_tokens: usage.and_then(|u| u.get("input_tokens")).and_then(|t| t.as_u64()),
            output_tokens: usage.and_then(|u| u.get("output_tokens")).and_then(|t| t.as_u64()),
            cached_tokens: usage
                .and_then(|u| u.get("cache_read_input_tokens"))
                .and_then(|t| t.as_u64()),
        }]
    }
}

impl EventParser for ClaudeParser {
    fn parse(&mut self, line: &str) -> Result<Vec<AgentEvent>, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "system" => {
                let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
                match subtype {
                    "init" => {
                        let session_id = v
                            .get("session_id")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok(vec![AgentEvent::SessionStart {
                            session_id,
                            agent: AgentKind::Claude,
                            model: None,
                        }])
                    }
                    "compact_boundary" => Ok(vec![AgentEvent::Compaction]),
                    "status" => Ok(vec![]),
                    other => {
                        if self.debug {
                            eprintln!("debug: unknown claude system subtype: {}", other);
                        }
                        Ok(vec![])
                    }
                }
            }
            "stream_event" => Ok(self.parse_stream_event(&v)),
            "assistant" => Ok(self.parse_assistant(&v)),
            "user" => Ok(self.parse_user(&v)),
            "result" => Ok(self.parse_result(&v)),
            "hook_started" | "hook_progress" | "hook_response"
            | "tool_progress" | "tool_use_summary"
            | "auth_status"
            | "task_started" | "task_progress" | "task_notification"
            | "files_persisted" | "rate_limit_event" | "prompt_suggestion" => Ok(vec![]),
            other => {
                if self.debug {
                    eprintln!("debug: unknown claude event type: {}", other);
                }
                Ok(vec![])
            }
        }
    }

    fn finish(&mut self) -> Vec<AgentEvent> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::EventParser;

    fn parse(parser: &mut ClaudeParser, line: &str) -> Vec<AgentEvent> {
        parser.parse(line).unwrap()
    }

    #[test]
    fn system_init() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"system","subtype":"init","session_id":"abc-123"}"#);
        assert_eq!(events, vec![AgentEvent::SessionStart {
            session_id: "abc-123".into(),
            agent: AgentKind::Claude,
            model: None,
        }]);
    }

    #[test]
    fn system_compact_boundary() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"system","subtype":"compact_boundary"}"#);
        assert_eq!(events, vec![AgentEvent::Compaction]);
    }

    #[test]
    fn stream_event_text_delta() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}}"#);
        assert_eq!(events, vec![AgentEvent::TextDelta("Hello".into())]);
    }

    #[test]
    fn stream_event_tool_use_flow() {
        let mut p = ClaudeParser::new(false);

        // content_block_start tool_use
        let e1 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_abc","name":"Bash"}}}"#);
        assert_eq!(e1, vec![AgentEvent::ToolStart { tool_name: "Bash".into() }]);

        // input_json_delta #1
        let e2 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"ls"}}}"#);
        assert!(e2.is_empty());

        // input_json_delta #2
        let e3 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" -la\"}"}}}"#);
        assert!(e3.is_empty());

        // content_block_stop → ToolReady
        let e4 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1}}"#);
        assert_eq!(e4.len(), 1);
        match &e4[0] {
            AgentEvent::ToolReady { tool_name, input_summary } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(input_summary, "ls -la");
            }
            other => panic!("expected ToolReady, got {:?}", other),
        }
    }

    #[test]
    fn stream_event_thinking_flow() {
        let mut p = ClaudeParser::new(false);

        let e1 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}}"#);
        assert_eq!(e1, vec![AgentEvent::ThinkingStart]);

        let e2 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}}"#);
        assert_eq!(e2, vec![AgentEvent::ThinkingDelta("Let me think...".into())]);

        let e3 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#);
        assert_eq!(e3, vec![AgentEvent::ThinkingEnd]);
    }

    #[test]
    fn assistant_message_no_prior_stream_events() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}},{"type":"thinking","thinking":"hmm"}]}}"#);
        assert_eq!(events, vec![
            AgentEvent::TextComplete("Hello".into()),
            AgentEvent::ToolStart { tool_name: "Bash".into() },
            AgentEvent::ToolReady { tool_name: "Bash".into(), input_summary: "ls".into() },
            AgentEvent::ThinkingStart,
            AgentEvent::ThinkingDelta("hmm".into()),
            AgentEvent::ThinkingEnd,
        ]);
    }

    #[test]
    fn assistant_message_after_stream_events_suppressed() {
        let mut p = ClaudeParser::new(false);
        // Trigger saw_stream_events by parsing a stream_event first
        parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#);
        // Now assistant should be suppressed
        let events = parse(&mut p, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi"}]}}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn user_tool_result() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","content":[{"type":"tool_result","tool_use_id":"toolu_abc","content":"file1.rs\nfile2.rs","is_error":false}]}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: false,
            content: "file1.rs\nfile2.rs".into(),
        }]);
    }

    #[test]
    fn user_tool_result_error() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","content":[{"type":"tool_result","tool_use_id":"toolu_abc","content":"permission denied","is_error":true}]}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: true,
            content: "permission denied".into(),
        }]);
    }

    #[test]
    fn user_tool_result_nested_under_message() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_abc","content":"hello_world","is_error":false}]}}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: false,
            content: "hello_world".into(),
        }]);
    }

    #[test]
    fn result_success() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0042,"num_turns":3,"duration_ms":12345,"duration_api_ms":9800,"usage":{"input_tokens":1500,"output_tokens":800,"cache_read_input_tokens":500}}"#);
        assert_eq!(events, vec![AgentEvent::SessionEnd {
            success: true,
            error_type: None,
            error_message: None,
            num_turns: Some(3),
            duration_ms: Some(12345),
            api_duration_ms: Some(9800),
            cost_usd: Some(0.0042),
            input_tokens: Some(1500),
            output_tokens: Some(800),
            cached_tokens: Some(500),
        }]);
    }

    #[test]
    fn result_error_max_turns() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#);
        match &events[0] {
            AgentEvent::SessionEnd { success, error_type, .. } => {
                assert!(!success);
                assert_eq!(error_type.as_deref(), Some("error_max_turns"));
            }
            other => panic!("expected SessionEnd, got {:?}", other),
        }
    }

    #[test]
    fn result_error_during_execution() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#);
        match &events[0] {
            AgentEvent::SessionEnd { success, error_type, .. } => {
                assert!(!success);
                assert_eq!(error_type.as_deref(), Some("error_during_execution"));
            }
            other => panic!("expected SessionEnd, got {:?}", other),
        }
    }

    #[test]
    fn unknown_type_returns_empty() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"unknown_future_type","data":{}}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn invalid_json_returns_error() {
        let mut p = ClaudeParser::new(false);
        assert!(p.parse("not json").is_err());
    }
}
