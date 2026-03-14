# agentcat

A universal stream formatter for AI coding agent output. Pipe NDJSON streams from multiple AI agent CLIs through agentcat to get rich, human-friendly terminal output with colors, emojis, spinners, and structured formatting.

## Supported agents

- **Claude Code** — `claude -p ... --output-format stream-json --verbose --include-partial-messages`
- **pi.dev** — `pi --mode json ...`
- **Gemini CLI** — `gemini -o stream-json -p ...`
- **Codex CLI** — `codex exec --json ...`
- **OpenCode** — `opencode run --format json ...`

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

# OpenCode
opencode run --format json "say hello" | agentcat

# Show extended thinking blocks
claude -p "complex task" --output-format stream-json --verbose --include-partial-messages | agentcat --show-thinking
```

## Options

```
agentcat [OPTIONS]

  --show-thinking     Show extended thinking blocks
  --no-emoji          Disable emoji output
  --no-color          Disable ANSI color output
  --format <FORMAT>   Force input format (claude|pi|gemini|codex|opencode)
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

## Installation

### From crates.io

```bash
cargo install agentcat
```

### From source

```bash
git clone https://github.com/rpodgorny/agentcat.git
cd agentcat
cargo build --release
# binary is at target/release/agentcat
```

### Arch Linux (AUR)

```bash
yay -S agentcat-git
```

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Agent reported failure |
| 2 | Parse error, empty stdin, or unrecognized format |

## Similar projects

All known similar tools are Claude Code specific:

- [claude-stream-format](https://github.com/jemmyw/claude-stream-format) — Rust CLI for formatting Claude Code JSON streams
- [claude-clean](https://github.com/ariel-frischer/claude-clean) — Go CLI with multiple output styles (boxed, compact, minimal, plain)
- [format-claude-stream](https://github.com/Khan/format-claude-stream) — TypeScript CLI by Khan Academy, also usable as a library
- [claude-stream-json-parser](https://github.com/shibuido/claude-stream-json-parser) — Rust library and CLI for parsing Claude Code stream-json output

agentcat is the only tool we know of that supports multiple agent formats (Claude Code, pi.dev, Gemini CLI, Codex, OpenCode) under a single unified interface.

