use std::fmt;

use clap::{Parser, ValueEnum};

/// Command line interface for the Serena MCP server prototype.
#[derive(Debug, Parser)]
#[command(name = "serena-mcp", about = "Minimal Serena MCP server prototype")]
pub struct Cli {
    /// Optional project path or identifier to activate at startup.
    #[arg(long)]
    pub project: Option<String>,

    /// Context name to mirror upstream Serena behaviour.
    #[arg(long, default_value = "desktop-app")]
    pub context: String,

    /// One or more operational modes.
    #[arg(long = "mode", value_enum, default_values_t = vec![Mode::Planning])]
    pub modes: Vec<Mode>,

    /// Transport selection. For now only `stdio` is implemented but the flag helps keep CLI parity.
    #[arg(long, value_enum, default_value_t = Transport::Stdio)]
    pub transport: Transport,
}

/// Stub representation of available modes.
#[derive(Debug, Clone, ValueEnum)]
pub enum Mode {
    Planning,
    Editing,
    Interactive,
}

/// Supported transports for the server. Only `stdio` is currently wired up.
#[derive(Debug, Clone, ValueEnum, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    #[allow(dead_code)]
    Sse,
    #[allow(dead_code)]
    StreamableHttp,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Mode::Planning => "planning",
            Mode::Editing => "editing",
            Mode::Interactive => "interactive",
        };
        write!(f, "{value}")
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Transport::Stdio => "stdio",
            Transport::Sse => "sse",
            Transport::StreamableHttp => "streamable-http",
        };
        write!(f, "{value}")
    }
}
