use crate::event::{AgentEvent, AgentKind};
use crate::parse::{tool_summary, EventParser};
use serde_json::Value;

enum BlockState {
    None,
    Text,
    ToolUse { name: String, json_buf: String, id: Option<String> },
    Thinking,
}

pub struct ClaudeParser {
    block_state: BlockState,
    saw_stream_events: bool,
    model_emitted: bool,
    debug: bool,
    // Claude Code emits the final response text in up to three places:
    //   1. stream_event text deltas (real-time streaming)
    //   2. assistant message content[].text (complete message)
    //   3. result event "result" field (session summary)
    //
    // Without deduplication the same text would appear multiple times.
    // We track the last emitted text so we can suppress duplicates from
    // the result event while still showing it when it's the only source
    // (e.g. error results, unusual session types).
    //
    // The existing `saw_stream_events` flag handles the stream→assistant
    // dedup. This `last_text` field handles the assistant/stream→result dedup.
    last_text: String,
}

impl ClaudeParser {
    pub fn new(debug: bool) -> Self {
        Self {
            block_state: BlockState::None,
            saw_stream_events: false,
            model_emitted: false,
            debug,
            last_text: String::new(),
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
                        self.last_text.clear();
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
                        let id = cb.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
                        let events = vec![AgentEvent::ToolStart {
                            tool_name: name.clone(),
                            id: id.clone(),
                        }];
                        self.block_state = BlockState::ToolUse {
                            name,
                            json_buf: String::new(),
                            id,
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
                        self.last_text.push_str(text);
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
                    BlockState::ToolUse { name, json_buf, id } => {
                        let input: Value =
                            serde_json::from_str(&json_buf).unwrap_or(Value::Null);
                        let summary = tool_summary(&name, &input);
                        vec![AgentEvent::ToolReady {
                            tool_name: name,
                            input_summary: summary,
                            id,
                        }]
                    }
                    _ => vec![],
                }
            }
            "message_start" => {
                if !self.model_emitted {
                    if let Some(model) = event
                        .get("message")
                        .and_then(|m| m.get("model"))
                        .and_then(|m| m.as_str())
                    {
                        self.model_emitted = true;
                        return vec![AgentEvent::ModelDetected(model.to_string())];
                    }
                }
                vec![]
            }
            "message_delta" | "message_stop" | "ping" | "error" => vec![],
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

        // Extract model from non-streaming assistant message
        if !self.model_emitted {
            if let Some(model) = v
                .get("message")
                .and_then(|m| m.get("model"))
                .and_then(|m| m.as_str())
            {
                self.model_emitted = true;
                events.push(AgentEvent::ModelDetected(model.to_string()));
            }
        }

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
                    self.last_text = text.to_string();
                    events.push(AgentEvent::TextComplete(text.to_string()));
                }
                "tool_use" => {
                    let name = block
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let id = block.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
                    let input = block.get("input").unwrap_or(&Value::Null);
                    let summary = tool_summary(&name, input);
                    events.push(AgentEvent::ToolStart {
                        tool_name: name.clone(),
                        id: id.clone(),
                    });
                    events.push(AgentEvent::ToolReady {
                        tool_name: name,
                        input_summary: summary,
                        id,
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

        // Determine if this is a sub-agent message based on parent_tool_use_id
        let is_subagent = v
            .get("parent_tool_use_id")
            .map(|p| !p.is_null())
            .unwrap_or(false);

        // content may be at top level or nested under "message"
        let content = v
            .get("content")
            .or_else(|| v.get("message").and_then(|m| m.get("content")));

        // content can be a plain string
        if let Some(Value::String(s)) = content {
            if !s.is_empty() {
                if is_subagent {
                    events.push(AgentEvent::SubAgentMessage(s.clone()));
                } else {
                    events.push(AgentEvent::UserMessage(s.clone()));
                }
            }
            return events;
        }

        let blocks = match content {
            Some(Value::Array(arr)) => arr.clone(),
            _ => return events,
        };

        let mut user_texts = vec![];

        for block in &blocks {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match block_type {
                "tool_result" => {
                    let is_error = block
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);
                    let id = block.get("tool_use_id").and_then(|i| i.as_str()).map(|s| s.to_string());
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
                        id,
                    });
                }
                "text" => {
                    let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if !text.is_empty() {
                        user_texts.push(text.to_string());
                    }
                }
                _ => {}
            }
        }

        if !user_texts.is_empty() {
            let joined = user_texts.join("\n");
            if is_subagent {
                events.push(AgentEvent::SubAgentMessage(joined));
            } else {
                events.push(AgentEvent::UserMessage(joined));
            }
        }

        events
    }

    fn parse_result(&mut self, v: &Value) -> Vec<AgentEvent> {
        let mut events = vec![];

        // Claude Code's result event may contain a "result" field with the final
        // response text. This is typically a duplicate of what was already emitted
        // via stream_event deltas or the assistant message. We suppress it when
        // it matches last_text to avoid showing the same content twice (a known
        // Claude Code quirk). When it differs or no prior text was emitted, we
        // show it — this covers error messages and edge cases where the result
        // is the only source of the final text.
        if let Some(result_text) = v.get("result").and_then(|r| r.as_str()) {
            if !result_text.is_empty() && result_text != self.last_text {
                events.push(AgentEvent::TextComplete(result_text.to_string()));
            }
        }
        self.last_text.clear();

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
        events.push(AgentEvent::SessionEnd {
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
        });
        events
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
        assert_eq!(e1, vec![AgentEvent::ToolStart { tool_name: "Bash".into(), id: Some("toolu_abc".into()) }]);

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
            AgentEvent::ToolReady { tool_name, input_summary, .. } => {
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
            AgentEvent::ToolStart { tool_name: "Bash".into(), id: Some("t1".into()) },
            AgentEvent::ToolReady { tool_name: "Bash".into(), input_summary: "ls".into(), id: Some("t1".into()) },
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
            id: Some("toolu_abc".into()),
        }]);
    }

    #[test]
    fn user_tool_result_error() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","content":[{"type":"tool_result","tool_use_id":"toolu_abc","content":"permission denied","is_error":true}]}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: true,
            content: "permission denied".into(),
            id: Some("toolu_abc".into()),
        }]);
    }

    #[test]
    fn user_tool_result_nested_under_message() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_abc","content":"hello_world","is_error":false}]}}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: false,
            content: "hello_world".into(),
            id: Some("toolu_abc".into()),
        }]);
    }

    #[test]
    fn user_text_message() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","content":[{"type":"text","text":"Please fix the bug"}]}"#);
        assert_eq!(events, vec![AgentEvent::UserMessage("Please fix the bug".into())]);
    }

    #[test]
    fn user_text_and_tool_result_mixed() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","content":[{"type":"text","text":"Here is context"},{"type":"tool_result","tool_use_id":"t1","content":"output","is_error":false}]}"#);
        assert_eq!(events, vec![
            AgentEvent::ToolResult { is_error: false, content: "output".into(), id: Some("t1".into()) },
            AgentEvent::UserMessage("Here is context".into()),
        ]);
    }

    #[test]
    fn user_empty_text_ignored() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","content":[{"type":"text","text":""}]}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn user_plain_string_content() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","content":"Fix the login bug"}"#);
        assert_eq!(events, vec![AgentEvent::UserMessage("Fix the login bug".into())]);
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
    fn user_text_with_parent_tool_use_id_emits_subagent_message() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","parent_tool_use_id":"toolu_abc","content":[{"type":"text","text":"Investigate the error"}]}"#);
        assert_eq!(events, vec![AgentEvent::SubAgentMessage("Investigate the error".into())]);
    }

    #[test]
    fn user_text_with_null_parent_tool_use_id_emits_user_message() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","parent_tool_use_id":null,"content":[{"type":"text","text":"Fix the bug"}]}"#);
        assert_eq!(events, vec![AgentEvent::UserMessage("Fix the bug".into())]);
    }

    #[test]
    fn user_plain_string_with_parent_tool_use_id_emits_subagent_message() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"user","parent_tool_use_id":"toolu_xyz","content":"Search for files"}"#);
        assert_eq!(events, vec![AgentEvent::SubAgentMessage("Search for files".into())]);
    }

    #[test]
    fn stream_event_message_start_emits_model() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_01abc","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6-20250514","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"output_tokens":1}}}}"#);
        assert_eq!(events, vec![AgentEvent::ModelDetected("claude-sonnet-4-6-20250514".into())]);
    }

    #[test]
    fn stream_event_message_start_model_emitted_only_once() {
        let mut p = ClaudeParser::new(false);
        let e1 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-sonnet-4-6-20250514"}}}"#);
        assert_eq!(e1, vec![AgentEvent::ModelDetected("claude-sonnet-4-6-20250514".into())]);
        // Second message_start should not emit again
        let e2 = parse(&mut p, r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-sonnet-4-6-20250514"}}}"#);
        assert!(e2.is_empty());
    }

    #[test]
    fn assistant_message_emits_model_when_no_stream() {
        let mut p = ClaudeParser::new(false);
        let events = parse(&mut p, r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-6","content":[{"type":"text","text":"Hello"}]}}"#);
        assert_eq!(events, vec![
            AgentEvent::ModelDetected("claude-opus-4-6".into()),
            AgentEvent::TextComplete("Hello".into()),
        ]);
    }

    #[test]
    fn assistant_message_no_model_after_stream_model() {
        let mut p = ClaudeParser::new(false);
        // Model already emitted via stream_event
        parse(&mut p, r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-sonnet-4-6-20250514"}}}"#);
        // stream text
        parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#);
        // assistant message (suppressed because saw_stream_events)
        let events = parse(&mut p, r#"{"type":"assistant","message":{"role":"assistant","model":"claude-sonnet-4-6-20250514","content":[{"type":"text","text":"Hi"}]}}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn result_text_suppressed_after_stream_events() {
        let mut p = ClaudeParser::new(false);
        // Stream text deltas
        parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text"}}}"#);
        parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello world"}}}"#);
        parse(&mut p, r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#);
        // Result with same text should NOT emit TextComplete
        let events = parse(&mut p, r#"{"type":"result","subtype":"success","result":"Hello world","total_cost_usd":0.01}"#);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::SessionEnd { success: true, .. }));
    }

    #[test]
    fn result_text_suppressed_after_assistant() {
        let mut p = ClaudeParser::new(false);
        // Non-streaming assistant message
        parse(&mut p, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"The answer is 42"}]}}"#);
        // Result with same text should NOT emit TextComplete
        let events = parse(&mut p, r#"{"type":"result","subtype":"success","result":"The answer is 42","total_cost_usd":0.01}"#);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::SessionEnd { success: true, .. }));
    }

    #[test]
    fn result_text_emitted_when_no_prior_text() {
        let mut p = ClaudeParser::new(false);
        // No prior streaming or assistant text — result text should be emitted
        let events = parse(&mut p, r#"{"type":"result","subtype":"success","result":"Only in result","total_cost_usd":0.01}"#);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], AgentEvent::TextComplete("Only in result".into()));
        assert!(matches!(&events[1], AgentEvent::SessionEnd { success: true, .. }));
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
