mod event;
mod parse;
mod render;
mod style;

use clap::Parser;
use crossterm::tty::IsTty;
use parse::{Format, create_parser, detect_format};
use render::Renderer;
use std::process;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(name = "agentcat", version, about = "Rich terminal renderer for AI coding agent streams")]
struct Cli {
    /// Show extended thinking blocks (hidden by default)
    #[arg(long)]
    show_thinking: bool,

    /// Disable emoji output
    #[arg(long)]
    no_emoji: bool,

    /// Disable ANSI color output
    #[arg(long)]
    no_color: bool,

    /// Force input format: claude, pi, gemini, codex (default: auto-detect)
    #[arg(long)]
    format: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let use_color = !cli.no_color
        && std::env::var("NO_COLOR").is_err()
        && std::io::stdout().is_tty();
    let use_emoji = !cli.no_emoji;

    let forced_format = cli.format.as_deref().map(|f| match f {
        "claude" => Format::Claude,
        "pi" => Format::Pi,
        "gemini" => Format::Gemini,
        "codex" => Format::Codex,
        other => {
            eprintln!("Error: unknown format '{}'", other);
            process::exit(2);
        }
    });

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    // Read first line
    let first_line = match lines.next_line().await {
        Ok(Some(line)) => line,
        Ok(None) => {
            eprintln!("Error: empty stdin");
            process::exit(2);
        }
        Err(e) => {
            eprintln!("Error reading stdin: {}", e);
            process::exit(2);
        }
    };

    // Detect or use forced format
    let format = match forced_format {
        Some(f) => f,
        None => match detect_format(&first_line) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{}", e);
                process::exit(2);
            }
        },
    };

    let mut parser = create_parser(format);
    let mut renderer = Renderer::new(cli.show_thinking, use_emoji, use_color);
    let mut last_success = true;

    // Process first line
    match parser.parse(&first_line) {
        Ok(events) => {
            for event in events {
                if let event::AgentEvent::SessionEnd { success, .. } = &event {
                    last_success = *success;
                }
                if let Err(e) = renderer.render(event) {
                    eprintln!("Render error: {}", e);
                    process::exit(2);
                }
            }
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
            process::exit(2);
        }
    }

    // Process remaining lines
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                match parser.parse(&line) {
                    Ok(events) => {
                        for event in events {
                            if let event::AgentEvent::SessionEnd { success, .. } = &event {
                                last_success = *success;
                            }
                            if let Err(e) = renderer.render(event) {
                                eprintln!("Render error: {}", e);
                                process::exit(2);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Parse error: {}", e);
                        // Don't exit on single parse errors, continue
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                eprintln!("Error reading stdin: {}", e);
                process::exit(2);
            }
        }
    }

    // Finish (emit any remaining events, e.g. Codex SessionEnd)
    let final_events = parser.finish();
    for event in final_events {
        if let event::AgentEvent::SessionEnd { success, .. } = &event {
            last_success = *success;
        }
        if let Err(e) = renderer.render(event) {
            eprintln!("Render error: {}", e);
            process::exit(2);
        }
    }

    if last_success {
        process::exit(0);
    } else {
        process::exit(1);
    }
}
