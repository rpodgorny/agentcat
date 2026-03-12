pub mod claude;
pub mod codex;
pub mod gemini;
pub mod pi;

use crate::event::AgentEvent;

pub trait EventParser {
    fn parse(&mut self, line: &str) -> Result<Vec<AgentEvent>, String>;
    fn finish(&mut self) -> Vec<AgentEvent>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Claude,
    Pi,
    Gemini,
    Codex,
}

pub fn detect_format(first_line: &str) -> Result<Format, String> {
    let v: serde_json::Value =
        serde_json::from_str(first_line).map_err(|e| format!("invalid JSON: {e}"))?;
    match v.get("type").and_then(|t| t.as_str()) {
        Some("system") => Ok(Format::Claude),
        Some("session") => Ok(Format::Pi),
        Some("init") => Ok(Format::Gemini),
        Some("thread.started") => Ok(Format::Codex),
        _ => Err("Error: unrecognized stream format".to_string()),
    }
}

pub fn create_parser(format: Format, debug: bool) -> Box<dyn EventParser> {
    match format {
        Format::Claude => Box::new(claude::ClaudeParser::new(debug)),
        Format::Pi => Box::new(pi::PiParser::new(debug)),
        Format::Gemini => Box::new(gemini::GeminiParser::new(debug)),
        Format::Codex => Box::new(codex::CodexParser::new(debug)),
    }
}

pub fn tool_summary(tool_name: &str, input: &serde_json::Value) -> String {
    let name_lower = tool_name.to_lowercase();
    match name_lower.as_str() {
        "bash" | "command_execution" => {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                truncate(cmd, 80)
            } else {
                fallback_summary(tool_name, input)
            }
        }
        "read" | "read_file" => extract_field(input, "file_path", tool_name),
        "write" | "write_file" => extract_field(input, "file_path", tool_name),
        "edit" | "edit_file" => extract_field(input, "file_path", tool_name),
        "glob" => extract_field(input, "pattern", tool_name),
        "grep" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let path = input.get("path").and_then(|v| v.as_str());
            if let Some(p) = path {
                format!("\"{}\" in {}", pattern, p)
            } else {
                format!("\"{}\"", pattern)
            }
        }
        "websearch" | "web_search" => extract_field(input, "query", tool_name),
        "webfetch" => extract_field(input, "url", tool_name),
        "filechange" | "file_change" => {
            if let Some(changes) = input.get("changes").and_then(|v| v.as_array()) {
                let paths: Vec<&str> = changes
                    .iter()
                    .filter_map(|c| c.get("path").and_then(|p| p.as_str()))
                    .collect();
                if paths.is_empty() {
                    fallback_summary(tool_name, input)
                } else {
                    paths.join(", ")
                }
            } else {
                fallback_summary(tool_name, input)
            }
        }
        _ => fallback_summary(tool_name, input),
    }
}

fn extract_field(input: &serde_json::Value, field: &str, tool_name: &str) -> String {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| fallback_summary(tool_name, input))
}

fn fallback_summary(_tool_name: &str, input: &serde_json::Value) -> String {
    let json_str = input.to_string();
    truncate(&json_str, 60)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- detect_format ---

    #[test]
    fn detect_format_claude() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123"}"#;
        assert_eq!(detect_format(line).unwrap(), Format::Claude);
    }

    #[test]
    fn detect_format_pi() {
        let line = r#"{"type":"session","version":3,"id":"550e8400"}"#;
        assert_eq!(detect_format(line).unwrap(), Format::Pi);
    }

    #[test]
    fn detect_format_gemini() {
        let line = r#"{"type":"init","session_id":"test-123","model":"gemini-2.5-pro"}"#;
        assert_eq!(detect_format(line).unwrap(), Format::Gemini);
    }

    #[test]
    fn detect_format_codex() {
        let line = r#"{"type":"thread.started","thread_id":"0199a213"}"#;
        assert_eq!(detect_format(line).unwrap(), Format::Codex);
    }

    #[test]
    fn detect_format_unknown_type() {
        let line = r#"{"type":"foobar"}"#;
        assert!(detect_format(line).is_err());
    }

    #[test]
    fn detect_format_invalid_json() {
        assert!(detect_format("not json at all").is_err());
    }

    // --- tool_summary ---

    #[test]
    fn tool_summary_bash_command() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(tool_summary("Bash", &input), "ls -la");
    }

    #[test]
    fn tool_summary_bash_truncates_at_80() {
        let long_cmd = "a".repeat(100);
        let input = serde_json::json!({"command": long_cmd});
        let result = tool_summary("Bash", &input);
        assert_eq!(result.len(), 83); // 80 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn tool_summary_command_execution() {
        let input = serde_json::json!({"command": "echo hello"});
        assert_eq!(tool_summary("command_execution", &input), "echo hello");
    }

    #[test]
    fn tool_summary_read_file_path() {
        let input = serde_json::json!({"file_path": "/src/main.rs"});
        assert_eq!(tool_summary("Read", &input), "/src/main.rs");
    }

    #[test]
    fn tool_summary_write_file_path() {
        let input = serde_json::json!({"file_path": "/tmp/out.txt"});
        assert_eq!(tool_summary("Write", &input), "/tmp/out.txt");
    }

    #[test]
    fn tool_summary_edit_file_path() {
        let input = serde_json::json!({"file_path": "/src/lib.rs"});
        assert_eq!(tool_summary("Edit", &input), "/src/lib.rs");
    }

    #[test]
    fn tool_summary_glob_pattern() {
        let input = serde_json::json!({"pattern": "**/*.rs"});
        assert_eq!(tool_summary("Glob", &input), "**/*.rs");
    }

    #[test]
    fn tool_summary_grep_pattern_only() {
        let input = serde_json::json!({"pattern": "TODO"});
        assert_eq!(tool_summary("Grep", &input), r#""TODO""#);
    }

    #[test]
    fn tool_summary_grep_pattern_with_path() {
        let input = serde_json::json!({"pattern": "TODO", "path": "src/"});
        assert_eq!(tool_summary("Grep", &input), r#""TODO" in src/"#);
    }

    #[test]
    fn tool_summary_websearch_query() {
        let input = serde_json::json!({"query": "rust async"});
        assert_eq!(tool_summary("WebSearch", &input), "rust async");
    }

    #[test]
    fn tool_summary_webfetch_url() {
        let input = serde_json::json!({"url": "https://example.com"});
        assert_eq!(tool_summary("WebFetch", &input), "https://example.com");
    }

    #[test]
    fn tool_summary_filechange_paths() {
        let input = serde_json::json!({
            "changes": [
                {"path": "a.rs", "kind": "edit"},
                {"path": "b.rs", "kind": "create"}
            ]
        });
        assert_eq!(tool_summary("FileChange", &input), "a.rs, b.rs");
    }

    #[test]
    fn tool_summary_filechange_empty_changes() {
        let input = serde_json::json!({"changes": []});
        // Empty paths → fallback
        let result = tool_summary("FileChange", &input);
        assert!(result.contains("changes"));
    }

    #[test]
    fn tool_summary_mcp_fallback() {
        let input = serde_json::json!({"query": "hello"});
        let result = tool_summary("my-server/search", &input);
        // Should be the first 60 chars of the JSON string (or less)
        assert!(result.contains("query"));
    }

    #[test]
    fn tool_summary_missing_field_fallback() {
        let input = serde_json::json!({"something": "else"});
        let result = tool_summary("Bash", &input);
        // No "command" field → fallback summary
        assert!(result.contains("something"));
    }
}
