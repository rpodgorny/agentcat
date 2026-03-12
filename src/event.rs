use std::fmt;

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum AgentKind {
    Claude,
    Pi,
    Gemini,
    Codex,
    Unknown,
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentKind::Claude => write!(f, "claude"),
            AgentKind::Pi => write!(f, "pi"),
            AgentKind::Gemini => write!(f, "gemini"),
            AgentKind::Codex => write!(f, "codex"),
            AgentKind::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum AgentEvent {
    SessionStart {
        session_id: String,
        agent: AgentKind,
        model: Option<String>,
    },
    TextDelta(String),
    TextComplete(String),
    ThinkingStart,
    ThinkingDelta(String),
    ThinkingEnd,
    ToolStart {
        tool_name: String,
    },
    ToolReady {
        tool_name: String,
        input_summary: String,
    },
    ToolResult {
        is_error: bool,
        content: String,
    },
    Compaction,
    Warning(String),
    SessionEnd {
        success: bool,
        error_type: Option<String>,
        error_message: Option<String>,
        num_turns: Option<u32>,
        duration_ms: Option<u64>,
        api_duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cached_tokens: Option<u64>,
    },
}
