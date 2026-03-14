use crate::event::{AgentEvent, AgentKind};
use crate::parse::{tool_summary, EventParser};
use serde_json::Value;

pub struct OpenCodeParser {
    session_started: bool,
    accumulated_input_tokens: u64,
    accumulated_output_tokens: u64,
    accumulated_cached_tokens: u64,
    accumulated_cost: f64,
    had_error: bool,
    error_message: Option<String>,
    debug: bool,
}

impl OpenCodeParser {
    pub fn new(debug: bool) -> Self {
        Self {
            session_started: false,
            accumulated_input_tokens: 0,
            accumulated_output_tokens: 0,
            accumulated_cached_tokens: 0,
            accumulated_cost: 0.0,
            had_error: false,
            error_message: None,
            debug,
        }
    }
}

impl EventParser for OpenCodeParser {
    fn parse(&mut self, line: &str) -> Result<Vec<AgentEvent>, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "step_start" => {
                if self.session_started {
                    return Ok(vec![]);
                }
                self.session_started = true;
                let session_id = v
                    .get("part")
                    .and_then(|p| p.get("sessionID"))
                    .and_then(|s| s.as_str())
                    .or_else(|| v.get("sessionID").and_then(|s| s.as_str()))
                    .unwrap_or("")
                    .to_string();
                Ok(vec![AgentEvent::SessionStart {
                    session_id,
                    agent: AgentKind::OpenCode,
                    model: None,
                }])
            }
            "text" => {
                let text = v
                    .get("part")
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(vec![AgentEvent::TextComplete(text)])
            }
            "reasoning" => {
                let text = v
                    .get("part")
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(vec![
                    AgentEvent::ThinkingStart,
                    AgentEvent::ThinkingDelta(text),
                    AgentEvent::ThinkingEnd,
                ])
            }
            "tool_use" => {
                let part = v.get("part").unwrap_or(&Value::Null);
                let state = part.get("state").and_then(|s| s.as_str()).unwrap_or("");
                match state {
                    "running" => {
                        let tool_name = part
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let input = part.get("input").unwrap_or(&Value::Null);
                        let summary = tool_summary(&tool_name, input);
                        Ok(vec![
                            AgentEvent::ToolStart {
                                tool_name: tool_name.clone(),
                            },
                            AgentEvent::ToolReady {
                                tool_name,
                                input_summary: summary,
                            },
                        ])
                    }
                    "completed" => {
                        let output = part
                            .get("output")
                            .and_then(|o| o.as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok(vec![AgentEvent::ToolResult {
                            is_error: false,
                            content: output,
                        }])
                    }
                    "error" => {
                        let output = part
                            .get("output")
                            .and_then(|o| o.as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok(vec![AgentEvent::ToolResult {
                            is_error: true,
                            content: output,
                        }])
                    }
                    other => {
                        if self.debug {
                            eprintln!("debug: unknown opencode tool_use state: {}", other);
                        }
                        Ok(vec![])
                    }
                }
            }
            "step_finish" => {
                if let Some(part) = v.get("part") {
                    if let Some(tokens) = part.get("tokens") {
                        self.accumulated_input_tokens += tokens
                            .get("input")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0);
                        self.accumulated_output_tokens += tokens
                            .get("output")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0);
                        self.accumulated_cached_tokens += tokens
                            .get("cache")
                            .and_then(|c| c.get("read"))
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0);
                    }
                    if let Some(cost) = part.get("cost").and_then(|c| c.as_f64()) {
                        self.accumulated_cost += cost;
                    }
                }
                Ok(vec![])
            }
            "error" => {
                self.had_error = true;
                self.error_message = v
                    .get("error")
                    .and_then(|e| e.get("data"))
                    .and_then(|d| d.get("message"))
                    .and_then(|m| m.as_str())
                    .or_else(|| {
                        v.get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                    })
                    .map(|s| s.to_string());
                Ok(vec![])
            }
            "message.part.updated" | "session.status" | "session.error" => Ok(vec![]),
            other => {
                if self.debug {
                    eprintln!("debug: unknown opencode event type: {}", other);
                }
                Ok(vec![])
            }
        }
    }

    fn finish(&mut self) -> Vec<AgentEvent> {
        let input = if self.accumulated_input_tokens > 0 {
            Some(self.accumulated_input_tokens)
        } else {
            None
        };
        let output = if self.accumulated_output_tokens > 0 {
            Some(self.accumulated_output_tokens)
        } else {
            None
        };
        let cached = if self.accumulated_cached_tokens > 0 {
            Some(self.accumulated_cached_tokens)
        } else {
            None
        };
        let cost = if self.accumulated_cost > 0.0 {
            Some(self.accumulated_cost)
        } else {
            None
        };

        vec![AgentEvent::SessionEnd {
            success: !self.had_error,
            error_type: if self.had_error {
                Some("error".to_string())
            } else {
                None
            },
            error_message: self.error_message.take(),
            num_turns: None,
            duration_ms: None,
            api_duration_ms: None,
            cost_usd: cost,
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cached,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::EventParser;

    fn parse(parser: &mut OpenCodeParser, line: &str) -> Vec<AgentEvent> {
        parser.parse(line).unwrap()
    }

    #[test]
    fn step_start_first_emits_session_start() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"step_start","timestamp":1700000000,"sessionID":"ses_abc","part":{"sessionID":"ses_abc"}}"#,
        );
        assert_eq!(
            events,
            vec![AgentEvent::SessionStart {
                session_id: "ses_abc".into(),
                agent: AgentKind::OpenCode,
                model: None,
            }]
        );
    }

    #[test]
    fn step_start_second_is_ignored() {
        let mut p = OpenCodeParser::new(false);
        parse(
            &mut p,
            r#"{"type":"step_start","timestamp":1700000000,"sessionID":"ses_abc","part":{"sessionID":"ses_abc"}}"#,
        );
        let events = parse(
            &mut p,
            r#"{"type":"step_start","timestamp":1700000001,"sessionID":"ses_abc","part":{"sessionID":"ses_abc"}}"#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn text_event() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"text","timestamp":1700000000,"part":{"text":"Hello world"}}"#,
        );
        assert_eq!(events, vec![AgentEvent::TextComplete("Hello world".into())]);
    }

    #[test]
    fn reasoning_event() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"reasoning","timestamp":1700000000,"part":{"text":"Let me think..."}}"#,
        );
        assert_eq!(
            events,
            vec![
                AgentEvent::ThinkingStart,
                AgentEvent::ThinkingDelta("Let me think...".into()),
                AgentEvent::ThinkingEnd,
            ]
        );
    }

    #[test]
    fn tool_use_running() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"tool_use","timestamp":1700000000,"part":{"name":"Bash","state":"running","input":{"command":"ls -la"}}}"#,
        );
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            AgentEvent::ToolStart {
                tool_name: "Bash".into()
            }
        );
        match &events[1] {
            AgentEvent::ToolReady {
                tool_name,
                input_summary,
            } => {
                assert_eq!(tool_name, "Bash");
                assert!(input_summary.contains("ls -la"));
            }
            other => panic!("expected ToolReady, got {:?}", other),
        }
    }

    #[test]
    fn tool_use_completed() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"tool_use","timestamp":1700000000,"part":{"name":"Bash","state":"completed","output":"file1.txt\nfile2.txt"}}"#,
        );
        assert_eq!(
            events,
            vec![AgentEvent::ToolResult {
                is_error: false,
                content: "file1.txt\nfile2.txt".into(),
            }]
        );
    }

    #[test]
    fn tool_use_error() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"tool_use","timestamp":1700000000,"part":{"name":"Bash","state":"error","output":"command not found"}}"#,
        );
        assert_eq!(
            events,
            vec![AgentEvent::ToolResult {
                is_error: true,
                content: "command not found".into(),
            }]
        );
    }

    #[test]
    fn step_finish_accumulates_tokens() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"step_finish","timestamp":1700000000,"part":{"tokens":{"input":1000,"output":200,"cache":{"read":500}},"cost":0.05}}"#,
        );
        assert!(events.is_empty());
        assert_eq!(p.accumulated_input_tokens, 1000);
        assert_eq!(p.accumulated_output_tokens, 200);
        assert_eq!(p.accumulated_cached_tokens, 500);
        assert!((p.accumulated_cost - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn step_finish_multi_accumulation() {
        let mut p = OpenCodeParser::new(false);
        parse(
            &mut p,
            r#"{"type":"step_finish","timestamp":1700000000,"part":{"tokens":{"input":1000,"output":100,"cache":{"read":500}},"cost":0.03}}"#,
        );
        parse(
            &mut p,
            r#"{"type":"step_finish","timestamp":1700000001,"part":{"tokens":{"input":2000,"output":200,"cache":{"read":1500}},"cost":0.07}}"#,
        );
        let events = p.finish();
        assert_eq!(
            events,
            vec![AgentEvent::SessionEnd {
                success: true,
                error_type: None,
                error_message: None,
                num_turns: None,
                duration_ms: None,
                api_duration_ms: None,
                cost_usd: Some(0.1),
                input_tokens: Some(3000),
                output_tokens: Some(300),
                cached_tokens: Some(2000),
            }]
        );
    }

    #[test]
    fn error_event() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"error","timestamp":1700000000,"error":{"data":{"message":"Rate limit exceeded"}}}"#,
        );
        assert!(events.is_empty());
        assert!(p.had_error);
        assert_eq!(p.error_message.as_deref(), Some("Rate limit exceeded"));
    }

    #[test]
    fn finish_success() {
        let mut p = OpenCodeParser::new(false);
        parse(
            &mut p,
            r#"{"type":"step_finish","timestamp":1700000000,"part":{"tokens":{"input":500,"output":100,"cache":{"read":200}},"cost":0.02}}"#,
        );
        let events = p.finish();
        assert_eq!(
            events,
            vec![AgentEvent::SessionEnd {
                success: true,
                error_type: None,
                error_message: None,
                num_turns: None,
                duration_ms: None,
                api_duration_ms: None,
                cost_usd: Some(0.02),
                input_tokens: Some(500),
                output_tokens: Some(100),
                cached_tokens: Some(200),
            }]
        );
    }

    #[test]
    fn finish_after_error() {
        let mut p = OpenCodeParser::new(false);
        parse(
            &mut p,
            r#"{"type":"error","timestamp":1700000000,"error":{"data":{"message":"Something broke"}}}"#,
        );
        let events = p.finish();
        match &events[0] {
            AgentEvent::SessionEnd {
                success,
                error_type,
                error_message,
                ..
            } => {
                assert!(!success);
                assert_eq!(error_type.as_deref(), Some("error"));
                assert_eq!(error_message.as_deref(), Some("Something broke"));
            }
            other => panic!("expected SessionEnd, got {:?}", other),
        }
    }

    #[test]
    fn unknown_type_returns_empty() {
        let mut p = OpenCodeParser::new(false);
        let events = parse(
            &mut p,
            r#"{"type":"future_event","timestamp":1700000000,"data":{}}"#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn invalid_json_returns_error() {
        let mut p = OpenCodeParser::new(false);
        assert!(p.parse("not json").is_err());
    }
}
