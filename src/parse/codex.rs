use crate::event::{AgentEvent, AgentKind};
use crate::parse::{tool_summary, EventParser};
use serde_json::Value;

pub struct CodexParser {
    accumulated_input_tokens: u64,
    accumulated_output_tokens: u64,
    accumulated_cached_tokens: u64,
    had_error: bool,
    error_message: Option<String>,
    session_id: String,
    debug: bool,
}

impl CodexParser {
    pub fn new(debug: bool) -> Self {
        Self {
            accumulated_input_tokens: 0,
            accumulated_output_tokens: 0,
            accumulated_cached_tokens: 0,
            had_error: false,
            error_message: None,
            session_id: String::new(),
            debug,
        }
    }

    fn parse_item_started(&self, item: &Value) -> Vec<AgentEvent> {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match item_type {
            "command_execution" => {
                let command = item
                    .get("command")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = serde_json::json!({"command": command});
                let summary = tool_summary("Bash", &input);
                vec![
                    AgentEvent::ToolStart {
                        tool_name: "Bash".to_string(),
                    },
                    AgentEvent::ToolReady {
                        tool_name: "Bash".to_string(),
                        input_summary: summary,
                    },
                ]
            }
            "reasoning" => vec![AgentEvent::ThinkingStart],
            "file_change" => vec![AgentEvent::ToolStart {
                tool_name: "FileChange".to_string(),
            }],
            "mcp_tool_call" => {
                let server = item
                    .get("server")
                    .and_then(|s| s.as_str())
                    .unwrap_or("mcp");
                let tool = item
                    .get("tool")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let name = format!("{}/{}", server, tool);
                vec![AgentEvent::ToolStart { tool_name: name }]
            }
            "agent_message" | "web_search" | "context_compaction"
            | "collab_tool_call" | "todo_list" | "error" => vec![],
            other => {
                if self.debug {
                    eprintln!("debug: unknown codex item.started type: {}", other);
                }
                vec![]
            }
        }
    }

    fn parse_item_completed(&self, item: &Value) -> Vec<AgentEvent> {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match item_type {
            "command_execution" => {
                let exit_code = item
                    .get("exitCode")
                    .and_then(|c| c.as_i64())
                    .unwrap_or(0);
                let output = item
                    .get("aggregatedOutput")
                    .and_then(|o| o.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_error = exit_code != 0;
                let content = if is_error {
                    format!("exit {}: {}", exit_code, output.lines().next().unwrap_or(""))
                } else {
                    output
                };
                vec![AgentEvent::ToolResult { is_error, content }]
            }
            "agent_message" => {
                let text = item
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                vec![AgentEvent::TextComplete(text)]
            }
            "reasoning" => {
                let mut events = vec![];
                if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
                    for entry in summary {
                        if let Some(text) = entry.get("text").and_then(|t| t.as_str()) {
                            events.push(AgentEvent::ThinkingDelta(text.to_string()));
                        }
                    }
                }
                events.push(AgentEvent::ThinkingEnd);
                events
            }
            "file_change" => {
                let mut events = vec![];
                let changes = item.get("changes").unwrap_or(&Value::Null);
                let summary = tool_summary("FileChange", &serde_json::json!({"changes": changes}));
                events.push(AgentEvent::ToolReady {
                    tool_name: "FileChange".to_string(),
                    input_summary: summary,
                });
                let status = item
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("completed");
                let is_error = status == "failed";
                events.push(AgentEvent::ToolResult {
                    is_error,
                    content: String::new(),
                });
                events
            }
            "mcp_tool_call" => {
                let server = item
                    .get("server")
                    .and_then(|s| s.as_str())
                    .unwrap_or("mcp");
                let tool = item
                    .get("tool")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let name = format!("{}/{}", server, tool);
                let args = item.get("arguments").unwrap_or(&Value::Null);
                let summary = tool_summary(&name, args);
                let status = item
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("completed");
                let is_error = status == "failed";
                let result = item
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                vec![
                    AgentEvent::ToolReady {
                        tool_name: name,
                        input_summary: summary,
                    },
                    AgentEvent::ToolResult {
                        is_error,
                        content: result,
                    },
                ]
            }
            "web_search" => {
                let query = item
                    .get("query")
                    .and_then(|q| q.as_str())
                    .unwrap_or("")
                    .to_string();
                vec![
                    AgentEvent::ToolStart {
                        tool_name: "WebSearch".to_string(),
                    },
                    AgentEvent::ToolReady {
                        tool_name: "WebSearch".to_string(),
                        input_summary: query,
                    },
                    AgentEvent::ToolResult {
                        is_error: false,
                        content: String::new(),
                    },
                ]
            }
            "context_compaction" => vec![AgentEvent::Compaction],
            "collab_tool_call" | "todo_list" | "error" => vec![],
            other => {
                if self.debug {
                    eprintln!("debug: unknown codex item.completed type: {}", other);
                }
                vec![]
            }
        }
    }
}

impl EventParser for CodexParser {
    fn parse(&mut self, line: &str) -> Result<Vec<AgentEvent>, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "thread.started" => {
                let thread_id = v
                    .get("thread_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                self.session_id = thread_id.clone();
                Ok(vec![AgentEvent::SessionStart {
                    session_id: thread_id,
                    agent: AgentKind::Codex,
                    model: None,
                }])
            }
            "item.started" => {
                let item = v.get("item").unwrap_or(&Value::Null);
                Ok(self.parse_item_started(item))
            }
            "item.completed" => {
                let item = v.get("item").unwrap_or(&Value::Null);
                Ok(self.parse_item_completed(item))
            }
            "turn.completed" => {
                if let Some(usage) = v.get("usage") {
                    self.accumulated_input_tokens += usage
                        .get("input_tokens")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    self.accumulated_output_tokens += usage
                        .get("output_tokens")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    self.accumulated_cached_tokens += usage
                        .get("cached_input_tokens")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                }
                Ok(vec![])
            }
            "turn.failed" => {
                self.had_error = true;
                self.error_message = v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
                Ok(vec![])
            }
            "error" => {
                self.had_error = true;
                self.error_message = v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
                Ok(vec![])
            }
            "turn.started" | "item.updated" => Ok(vec![]),
            other => {
                if self.debug {
                    eprintln!("debug: unknown codex event type: {}", other);
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
            cost_usd: None,
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

    fn parse(parser: &mut CodexParser, line: &str) -> Vec<AgentEvent> {
        parser.parse(line).unwrap()
    }

    #[test]
    fn thread_started() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"thread.started","thread_id":"0199a213-81c0-7800"}"#);
        assert_eq!(events, vec![AgentEvent::SessionStart {
            session_id: "0199a213-81c0-7800".into(),
            agent: AgentKind::Codex,
            model: None,
        }]);
    }

    #[test]
    fn item_started_command_execution() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"bash -lc ls","cwd":"/project","status":"in_progress"}}"#);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], AgentEvent::ToolStart { tool_name: "Bash".into() });
        match &events[1] {
            AgentEvent::ToolReady { tool_name, input_summary } => {
                assert_eq!(tool_name, "Bash");
                assert!(input_summary.contains("bash -lc ls"));
            }
            other => panic!("expected ToolReady, got {:?}", other),
        }
    }

    #[test]
    fn item_completed_command_execution_success() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"bash -lc ls","status":"completed","exitCode":0,"aggregatedOutput":"docs\nsdk\n"}}"#);
        assert_eq!(events, vec![AgentEvent::ToolResult {
            is_error: false,
            content: "docs\nsdk\n".into(),
        }]);
    }

    #[test]
    fn item_completed_command_execution_failure() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"bash -lc bad","status":"completed","exitCode":1,"aggregatedOutput":"error: not found\ndetails"}}"#);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolResult { is_error, content } => {
                assert!(is_error);
                assert!(content.starts_with("exit 1:"));
            }
            other => panic!("expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn item_completed_agent_message() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"Repo contains docs, sdk, and examples directories."}}"#);
        assert_eq!(events, vec![AgentEvent::TextComplete("Repo contains docs, sdk, and examples directories.".into())]);
    }

    #[test]
    fn item_started_reasoning() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.started","item":{"id":"item_4","type":"reasoning","summary":[],"content":[]}}"#);
        assert_eq!(events, vec![AgentEvent::ThinkingStart]);
    }

    #[test]
    fn item_completed_reasoning() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_4","type":"reasoning","summary":[{"type":"summaryText","text":"Analyzing the repository structure..."}],"content":[]}}"#);
        assert_eq!(events, vec![
            AgentEvent::ThinkingDelta("Analyzing the repository structure...".into()),
            AgentEvent::ThinkingEnd,
        ]);
    }

    #[test]
    fn item_started_file_change() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.started","item":{"id":"item_2","type":"file_change"}}"#);
        assert_eq!(events, vec![AgentEvent::ToolStart { tool_name: "FileChange".into() }]);
    }

    #[test]
    fn item_completed_file_change() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_2","type":"file_change","status":"completed","changes":[{"path":"src/main.rs","kind":"edit","diff":"..."}]}}"#);
        assert_eq!(events.len(), 2);
        match &events[0] {
            AgentEvent::ToolReady { tool_name, input_summary } => {
                assert_eq!(tool_name, "FileChange");
                assert!(input_summary.contains("src/main.rs"));
            }
            other => panic!("expected ToolReady, got {:?}", other),
        }
        assert_eq!(events[1], AgentEvent::ToolResult { is_error: false, content: String::new() });
    }

    #[test]
    fn item_completed_mcp_tool_call() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_5","type":"mcp_tool_call","server":"my-server","tool":"search","status":"completed","arguments":{"query":"hello"},"result":"Search results..."}}"#);
        assert_eq!(events.len(), 2);
        match &events[0] {
            AgentEvent::ToolReady { tool_name, .. } => {
                assert_eq!(tool_name, "my-server/search");
            }
            other => panic!("expected ToolReady, got {:?}", other),
        }
        assert_eq!(events[1], AgentEvent::ToolResult { is_error: false, content: "Search results...".into() });
    }

    #[test]
    fn item_completed_web_search() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_6","type":"web_search","query":"rust async stdin","action":"search"}}"#);
        assert_eq!(events, vec![
            AgentEvent::ToolStart { tool_name: "WebSearch".into() },
            AgentEvent::ToolReady { tool_name: "WebSearch".into(), input_summary: "rust async stdin".into() },
            AgentEvent::ToolResult { is_error: false, content: String::new() },
        ]);
    }

    #[test]
    fn item_completed_context_compaction() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"item.completed","item":{"id":"item_7","type":"context_compaction"}}"#);
        assert_eq!(events, vec![AgentEvent::Compaction]);
    }

    #[test]
    fn turn_completed_accumulates_tokens() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"turn.completed","usage":{"input_tokens":24763,"cached_input_tokens":24448,"output_tokens":122}}"#);
        assert!(events.is_empty());
        assert_eq!(p.accumulated_input_tokens, 24763);
        assert_eq!(p.accumulated_output_tokens, 122);
        assert_eq!(p.accumulated_cached_tokens, 24448);
    }

    #[test]
    fn turn_failed_sets_error() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"turn.failed","error":{"message":"Context window exceeded","codexErrorInfo":"ContextWindowExceeded"}}"#);
        assert!(events.is_empty());
        assert!(p.had_error);
        assert_eq!(p.error_message.as_deref(), Some("Context window exceeded"));
    }

    #[test]
    fn error_event_sets_error() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"error","error":{"message":"Context window exceeded"}}"#);
        assert!(events.is_empty());
        assert!(p.had_error);
    }

    #[test]
    fn finish_after_success() {
        let mut p = CodexParser::new(false);
        parse(&mut p, r#"{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":500,"output_tokens":200}}"#);
        let events = p.finish();
        assert_eq!(events, vec![AgentEvent::SessionEnd {
            success: true,
            error_type: None,
            error_message: None,
            num_turns: None,
            duration_ms: None,
            api_duration_ms: None,
            cost_usd: None,
            input_tokens: Some(1000),
            output_tokens: Some(200),
            cached_tokens: Some(500),
        }]);
    }

    #[test]
    fn finish_after_turn_failed() {
        let mut p = CodexParser::new(false);
        parse(&mut p, r#"{"type":"turn.failed","error":{"message":"Exceeded"}}"#);
        let events = p.finish();
        match &events[0] {
            AgentEvent::SessionEnd { success, error_type, error_message, .. } => {
                assert!(!success);
                assert_eq!(error_type.as_deref(), Some("error"));
                assert_eq!(error_message.as_deref(), Some("Exceeded"));
            }
            other => panic!("expected SessionEnd, got {:?}", other),
        }
    }

    #[test]
    fn multi_turn_accumulation() {
        let mut p = CodexParser::new(false);
        parse(&mut p, r#"{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":500,"output_tokens":100}}"#);
        parse(&mut p, r#"{"type":"turn.completed","usage":{"input_tokens":2000,"cached_input_tokens":1500,"output_tokens":200}}"#);
        let events = p.finish();
        assert_eq!(events, vec![AgentEvent::SessionEnd {
            success: true,
            error_type: None,
            error_message: None,
            num_turns: None,
            duration_ms: None,
            api_duration_ms: None,
            cost_usd: None,
            input_tokens: Some(3000),
            output_tokens: Some(300),
            cached_tokens: Some(2000),
        }]);
    }

    #[test]
    fn unknown_type_returns_empty() {
        let mut p = CodexParser::new(false);
        let events = parse(&mut p, r#"{"type":"future_event","data":{}}"#);
        assert!(events.is_empty());
    }
}
