use crate::event::{AgentEvent, AgentKind};
use crate::parse::{tool_summary, EventParser};
use serde_json::Value;

pub struct GeminiParser {
    debug: bool,
}

impl GeminiParser {
    pub fn new(debug: bool) -> Self {
        Self { debug }
    }
}

impl EventParser for GeminiParser {
    fn parse(&mut self, line: &str) -> Result<Vec<AgentEvent>, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "init" => {
                let session_id = v
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let model = v
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
                Ok(vec![AgentEvent::SessionStart {
                    session_id,
                    agent: AgentKind::Gemini,
                    model,
                }])
            }
            "message" => {
                let content = v
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "user" {
                    if content.is_empty() {
                        return Ok(vec![]);
                    }
                    return Ok(vec![AgentEvent::UserMessage(content)]);
                }
                let delta = v.get("delta").and_then(|d| d.as_bool()).unwrap_or(false);
                if delta {
                    Ok(vec![AgentEvent::TextDelta(content)])
                } else {
                    Ok(vec![AgentEvent::TextComplete(content)])
                }
            }
            "tool_use" => {
                let tool_name = v
                    .get("tool_name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let id = v.get("tool_id").and_then(|i| i.as_str()).map(|s| s.to_string());
                let params = v.get("parameters").unwrap_or(&Value::Null);
                let summary = tool_summary(&tool_name, params);
                Ok(vec![
                    AgentEvent::ToolStart {
                        tool_name: tool_name.clone(),
                        id: id.clone(),
                    },
                    AgentEvent::ToolReady {
                        tool_name,
                        input_summary: summary,
                        id,
                    },
                ])
            }
            "tool_result" => {
                let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let is_error = status == "error";
                let id = v.get("tool_id").and_then(|i| i.as_str()).map(|s| s.to_string());
                let content = if is_error {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("error")
                        .to_string()
                } else {
                    v.get("output")
                        .and_then(|o| o.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                Ok(vec![AgentEvent::ToolResult { is_error, content, id }])
            }
            "error" => {
                let message = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(vec![AgentEvent::Warning(message)])
            }
            "result" => {
                let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let success = status == "success";
                let stats = v.get("stats");

                let error_type = if !success {
                    v.get("error")
                        .and_then(|e| e.get("type"))
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                };
                let error_message = if !success {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                };

                Ok(vec![AgentEvent::SessionEnd {
                    success,
                    error_type,
                    error_message,
                    num_turns: None,
                    duration_ms: stats
                        .and_then(|s| s.get("duration_ms"))
                        .and_then(|d| d.as_u64()),
                    api_duration_ms: None,
                    cost_usd: None,
                    input_tokens: stats
                        .and_then(|s| s.get("input_tokens"))
                        .and_then(|t| t.as_u64()),
                    output_tokens: stats
                        .and_then(|s| s.get("output_tokens"))
                        .and_then(|t| t.as_u64()),
                    cached_tokens: stats
                        .and_then(|s| s.get("cached"))
                        .and_then(|t| t.as_u64()),
                }])
            }
            other => {
                if self.debug {
                    eprintln!("debug: unknown gemini event type: {}", other);
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

    fn parse(parser: &mut GeminiParser, line: &str) -> Vec<AgentEvent> {
        parser.parse(line).unwrap()
    }

    #[test]
    fn init_event() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"init","timestamp":"2025-10-10T12:00:00.000Z","session_id":"test-session-123","model":"gemini-2.5-pro"}"#);
        assert_eq!(events, vec![AgentEvent::SessionStart {
            session_id: "test-session-123".into(),
            agent: AgentKind::Gemini,
            model: Some("gemini-2.5-pro".into()),
        }]);
    }

    #[test]
    fn message_assistant_delta() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message","role":"assistant","content":"The answer","delta":true}"#);
        assert_eq!(events, vec![AgentEvent::TextDelta("The answer".into())]);
    }

    #[test]
    fn message_assistant_complete() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message","role":"assistant","content":"Full response","delta":false}"#);
        assert_eq!(events, vec![AgentEvent::TextComplete("Full response".into())]);
    }

    #[test]
    fn message_user_rendered() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message","role":"user","content":"my prompt"}"#);
        assert_eq!(events, vec![AgentEvent::UserMessage("my prompt".into())]);
    }

    #[test]
    fn message_user_empty_ignored() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"message","role":"user","content":""}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn tool_use_event() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"tool_use","tool_name":"Read","tool_id":"read-123","parameters":{"file_path":"/path/to/file.txt"}}"#);
        assert_eq!(events, vec![
            AgentEvent::ToolStart { tool_name: "Read".into(), id: Some("read-123".into()) },
            AgentEvent::ToolReady {
                tool_name: "Read".into(),
                input_summary: "/path/to/file.txt".into(),
                id: Some("read-123".into()),
            },
        ]);
    }

    #[test]
    fn tool_result_success() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"tool_result","tool_id":"read-123","status":"success","output":"file contents here"}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: false,
            content: "file contents here".into(),
            id: Some("read-123".into()),
        }]);
    }

    #[test]
    fn tool_result_error() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"tool_result","tool_id":"read-123","status":"error","error":{"type":"FILE_NOT_FOUND","message":"File not found"}}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: true,
            content: "File not found".into(),
            id: Some("read-123".into()),
        }]);
    }

    #[test]
    fn error_event_warning() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"error","severity":"warning","message":"Loop detected, stopping execution"}"#);
        assert_eq!(events, vec![AgentEvent::Warning("Loop detected, stopping execution".into())]);
    }

    #[test]
    fn result_success() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"result","status":"success","stats":{"duration_ms":3200,"input_tokens":150,"output_tokens":100,"cached":50}}"#);
        assert_eq!(events, vec![AgentEvent::SessionEnd {
            success: true,
            error_type: None,
            error_message: None,
            num_turns: None,
            duration_ms: Some(3200),
            api_duration_ms: None,
            cost_usd: None,
            input_tokens: Some(150),
            output_tokens: Some(100),
            cached_tokens: Some(50),
        }]);
    }

    #[test]
    fn result_error() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"result","status":"error","error":{"type":"MaxSessionTurnsError","message":"Maximum session turns exceeded"},"stats":{}}"#);
        match &events[0] {
            AgentEvent::SessionEnd { success, error_type, error_message, .. } => {
                assert!(!success);
                assert_eq!(error_type.as_deref(), Some("MaxSessionTurnsError"));
                assert_eq!(error_message.as_deref(), Some("Maximum session turns exceeded"));
            }
            other => panic!("expected SessionEnd, got {:?}", other),
        }
    }

    #[test]
    fn unknown_type_returns_empty() {
        let mut p = GeminiParser::new(false);
        let events = parse(&mut p, r#"{"type":"future_event","data":{}}"#);
        assert!(events.is_empty());
    }
}
