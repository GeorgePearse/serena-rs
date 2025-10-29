use anyhow::Result;
use clap::Parser;
use log::{error, info};
use simplelog::{ConfigBuilder, LevelFilter, SimpleLogger};

use serena_mcp::{
    cli::{Cli, Transport},
    rpc, tools,
};

fn main() {
    if let Err(err) = run() {
        error!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging();

    info!(
        "Starting Serena MCP prototype | context={} transport={:?} project={:?}",
        cli.context, cli.transport, cli.project
    );

    if cli.transport != Transport::Stdio {
        anyhow::bail!("Only stdio transport is implemented in the Rust prototype");
    }

    let registry = tools::build_registry();
    rpc::run_stdio_server(&registry)
}

fn init_logging() {
    let config = ConfigBuilder::new()
        .set_time_level(LevelFilter::Off)
        .set_location_level(LevelFilter::Off)
        .build();
    let _ = SimpleLogger::init(LevelFilter::Info, config);
}
