use crate::event::{AgentEvent, AgentKind};
use crate::parse::{tool_summary, EventParser};
use serde_json::Value;

pub struct PiParser {
    saw_text_deltas: bool,
    last_done_reason: Option<String>,
    model_emitted: bool,
    debug: bool,
}

impl PiParser {
    pub fn new(debug: bool) -> Self {
        Self {
            saw_text_deltas: false,
            last_done_reason: None,
            model_emitted: false,
            debug,
        }
    }

    fn parse_message_update(&mut self, v: &Value) -> Vec<AgentEvent> {
        let ame = match v.get("assistantMessageEvent") {
            Some(e) => e,
            None => return vec![],
        };
        let sub_type = ame.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match sub_type {
            "text_start" => {
                self.saw_text_deltas = false;
                vec![]
            }
            "text_delta" => {
                self.saw_text_deltas = true;
                let delta = ame.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                vec![AgentEvent::TextDelta(delta.to_string())]
            }
            "text_end" => {
                if !self.saw_text_deltas {
                    let content = ame.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    vec![AgentEvent::TextComplete(content.to_string())]
                } else {
                    self.saw_text_deltas = false;
                    vec![]
                }
            }
            "thinking_start" => vec![AgentEvent::ThinkingStart],
            "thinking_delta" => {
                let delta = ame.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                vec![AgentEvent::ThinkingDelta(delta.to_string())]
            }
            "thinking_end" => vec![AgentEvent::ThinkingEnd],
            "toolcall_start" => {
                vec![AgentEvent::ToolStart {
                    tool_name: String::new(),
                }]
            }
            "toolcall_end" => {
                let tc = ame.get("toolCall").unwrap_or(&Value::Null);
                let name = tc
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let args = tc.get("arguments").unwrap_or(&Value::Null);
                let summary = tool_summary(&name, args);
                vec![AgentEvent::ToolReady {
                    tool_name: name,
                    input_summary: summary,
                }]
            }
            "done" => {
                let reason = ame
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                self.last_done_reason = Some(reason.clone());

                let message = ame.get("message").unwrap_or(&Value::Null);

                let mut events = vec![];

                // Emit model on first sighting
                if !self.model_emitted {
                    if let Some(model) = message
                        .get("model")
                        .and_then(|m| m.as_str())
                    {
                        self.model_emitted = true;
                        events.push(AgentEvent::ModelDetected(model.to_string()));
                    }
                }

                // Only emit SessionEnd on final "stop" reason
                if reason == "stop" {
                    let usage = message.get("usage");

                    events.push(AgentEvent::SessionEnd {
                        success: true,
                        error_type: None,
                        error_message: None,
                        num_turns: None,
                        duration_ms: None,
                        api_duration_ms: None,
                        cost_usd: None,
                        input_tokens: usage
                            .and_then(|u| u.get("inputTokens"))
                            .and_then(|t| t.as_u64()),
                        output_tokens: usage
                            .and_then(|u| u.get("outputTokens"))
                            .and_then(|t| t.as_u64()),
                        cached_tokens: None,
                    });
                }

                events
            }
            "error" => {
                let error_msg = ame
                    .get("error")
                    .and_then(|e| {
                        // Could be string or object
                        if let Some(s) = e.as_str() {
                            Some(s.to_string())
                        } else {
                            e.get("message").and_then(|m| m.as_str()).map(|s| s.to_string())
                        }
                    })
                    .unwrap_or_else(|| "unknown error".to_string());
                let reason = ame
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("error")
                    .to_string();
                vec![AgentEvent::SessionEnd {
                    success: false,
                    error_type: Some(reason),
                    error_message: Some(error_msg),
                    num_turns: None,
                    duration_ms: None,
                    api_duration_ms: None,
                    cost_usd: None,
                    input_tokens: None,
                    output_tokens: None,
                    cached_tokens: None,
                }]
            }
            "start" | "toolcall_delta" => vec![],
            other => {
                if self.debug {
                    eprintln!("debug: unknown pi message_update sub_type: {}", other);
                }
                vec![]
            }
        }
    }
}

impl EventParser for PiParser {
    fn parse(&mut self, line: &str) -> Result<Vec<AgentEvent>, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "session" => {
                let id = v
                    .get("id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(vec![AgentEvent::SessionStart {
                    session_id: id,
                    agent: AgentKind::Pi,
                    model: None,
                }])
            }
            "message_update" => Ok(self.parse_message_update(&v)),
            "tool_execution_end" => {
                let is_error = v
                    .get("isError")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                // pi.dev tool_execution_end doesn't always have result content in the event
                let content = v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(vec![AgentEvent::ToolResult { is_error, content }])
            }
            "auto_compaction_start" => Ok(vec![AgentEvent::Compaction]),
            "agent_start" | "agent_end"
            | "turn_start" | "turn_end"
            | "message_start" | "message_end"
            | "tool_execution_start" | "tool_execution_update"
            | "auto_compaction_end"
            | "auto_retry_start" | "auto_retry_end"
            | "extension_error" => Ok(vec![]),
            other => {
                if self.debug {
                    eprintln!("debug: unknown pi event type: {}", other);
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

    fn parse(parser: &mut PiParser, line: &str) -> Vec<AgentEvent> {
        parser.parse(line).unwrap()
    }

    #[test]
    fn session_header() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"session","version":3,"id":"550e8400","timestamp":"2025-11-30T12:00:00.000Z","cwd":"/home/user/project"}"#);
        assert_eq!(events, vec![AgentEvent::SessionStart {
            session_id: "550e8400".into(),
            agent: AgentKind::Pi,
            model: None,
        }]);
    }

    #[test]
    fn message_update_text_delta() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hello","contentIndex":0}}"#);
        assert_eq!(events, vec![AgentEvent::TextDelta("Hello".into())]);
    }

    #[test]
    fn message_update_text_end_with_prior_deltas() {
        let mut p = PiParser::new(false);
        // First emit a delta to set saw_text_deltas
        parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hi","contentIndex":0}}"#);
        // text_end should be empty (duplicates suppressed)
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","content":"Hi","contentIndex":0}}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn message_update_text_end_no_prior_deltas() {
        let mut p = PiParser::new(false);
        // Reset via text_start
        parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#);
        // text_end with no deltas → TextComplete
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","content":"Full text","contentIndex":0}}"#);
        assert_eq!(events, vec![AgentEvent::TextComplete("Full text".into())]);
    }

    #[test]
    fn message_update_thinking() {
        let mut p = PiParser::new(false);
        let e1 = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_start","contentIndex":0}}"#);
        assert_eq!(e1, vec![AgentEvent::ThinkingStart]);

        let e2 = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"Analyzing...","contentIndex":0}}"#);
        assert_eq!(e2, vec![AgentEvent::ThinkingDelta("Analyzing...".into())]);

        let e3 = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_end","contentIndex":0}}"#);
        assert_eq!(e3, vec![AgentEvent::ThinkingEnd]);
    }

    #[test]
    fn message_update_toolcall_start() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"toolcall_start","contentIndex":1}}"#);
        assert_eq!(events, vec![AgentEvent::ToolStart { tool_name: String::new() }]);
    }

    #[test]
    fn message_update_toolcall_end() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"toolcall_end","toolCall":{"name":"bash","arguments":{"command":"ls -la"}},"contentIndex":1}}"#);
        assert_eq!(events, vec![AgentEvent::ToolReady {
            tool_name: "bash".into(),
            input_summary: "ls -la".into(),
        }]);
    }

    #[test]
    fn tool_execution_end_success() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"tool_execution_end","toolCallId":"call-123","isError":false}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: false,
            content: String::new(),
        }]);
    }

    #[test]
    fn tool_execution_end_error() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"tool_execution_end","toolCallId":"call-123","isError":true,"result":"permission denied"}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: true,
            content: "permission denied".into(),
        }]);
    }

    #[test]
    fn message_update_done_stop() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"done","reason":"stop","message":{"role":"assistant","usage":{"inputTokens":1500,"outputTokens":800}}}}"#);
        // No model in this message, so only SessionEnd
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::SessionEnd { success, input_tokens, output_tokens, .. } => {
                assert!(success);
                assert_eq!(*input_tokens, Some(1500));
                assert_eq!(*output_tokens, Some(800));
            }
            other => panic!("expected SessionEnd, got {:?}", other),
        }
    }

    #[test]
    fn message_update_done_tool_use_no_session_end() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"done","reason":"toolUse","message":{}}}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn message_update_error() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"error","reason":"error","error":"something broke"}}"#);
        match &events[0] {
            AgentEvent::SessionEnd { success, error_type, error_message, .. } => {
                assert!(!success);
                assert_eq!(error_type.as_deref(), Some("error"));
                assert_eq!(error_message.as_deref(), Some("something broke"));
            }
            other => panic!("expected SessionEnd, got {:?}", other),
        }
    }

    #[test]
    fn auto_compaction_start() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"auto_compaction_start"}"#);
        assert_eq!(events, vec![AgentEvent::Compaction]);
    }

    #[test]
    fn message_update_done_tool_use_emits_model() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"done","reason":"toolUse","message":{"role":"assistant","model":"claude-sonnet-4-6-20250514"}}}"#);
        assert_eq!(events, vec![AgentEvent::ModelDetected("claude-sonnet-4-6-20250514".into())]);
    }

    #[test]
    fn message_update_done_stop_emits_model_and_session_end() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"done","reason":"stop","message":{"role":"assistant","model":"claude-sonnet-4-6-20250514","usage":{"inputTokens":100,"outputTokens":50}}}}"#);
        assert_eq!(events, vec![
            AgentEvent::ModelDetected("claude-sonnet-4-6-20250514".into()),
            AgentEvent::SessionEnd {
                success: true,
                error_type: None,
                error_message: None,
                num_turns: None,
                duration_ms: None,
                api_duration_ms: None,
                cost_usd: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                cached_tokens: None,
            },
        ]);
    }

    #[test]
    fn message_update_done_model_emitted_only_once() {
        let mut p = PiParser::new(false);
        // First done emits model
        let e1 = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"done","reason":"toolUse","message":{"model":"claude-sonnet-4-6-20250514"}}}"#);
        assert_eq!(e1, vec![AgentEvent::ModelDetected("claude-sonnet-4-6-20250514".into())]);
        // Second done should not emit model again
        let e2 = parse(&mut p, r#"{"type":"message_update","assistantMessageEvent":{"type":"done","reason":"toolUse","message":{"model":"claude-sonnet-4-6-20250514"}}}"#);
        assert!(e2.is_empty());
    }

    #[test]
    fn unknown_type_returns_empty() {
        let mut p = PiParser::new(false);
        let events = parse(&mut p, r#"{"type":"turn_start"}"#);
        assert!(events.is_empty());
    }
}
