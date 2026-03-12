# agentcat

A universal stream formatter for AI coding agent output. Pipe NDJSON streams from multiple AI agent CLIs through agentcat to get rich, human-friendly terminal output with colors, emojis, spinners, and structured formatting.

## Supported agents

- **Claude Code** — `claude -p ... --output-format stream-json --verbose --include-partial-messages`
- **pi.dev** — `pi --mode json ...`
- **Gemini CLI** — `gemini -o stream-json -p ...`
- **Codex CLI** — `codex exec --json ...`

The input format is auto-detected from the first JSON line, or can be forced with `--format`.

## Usage

```bash
# Claude Code
claude -p "analyze src/main.rs" --output-format stream-json --verbose --include-partial-messages | agentcat

# pi.dev
pi --mode json "refactor the code" | agentcat

# Gemini CLI
gemini -o stream-json -p "explain this function" | agentcat

# Codex CLI
codex exec --json "write tests" | agentcat

# Show extended thinking blocks
claude -p "complex task" --output-format stream-json --verbose --include-partial-messages | agentcat --show-thinking
```

## Options

```
agentcat [OPTIONS]

  --show-thinking     Show extended thinking blocks
  --no-emoji          Disable emoji output
  --no-color          Disable ANSI color output
  --format <FORMAT>   Force input format (claude|pi|gemini|codex)
  --version           Print version
  --help              Print help
```

The `NO_COLOR` environment variable is also respected.

## What it shows

- Session info (agent type, model, session ID)
- Streaming text responses with live updates
- Tool execution with animated per-tool spinners and elapsed time
- Extended thinking blocks (with `--show-thinking`)
- Token usage, cost, and timing statistics
- Error context and warnings

## Building

Requires Rust toolchain.

```bash
cargo build --release
```

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Agent reported failure |
| 2 | Parse error, empty stdin, or unrecognized format |
