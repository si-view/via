mod cli;
mod codec;
mod forward;
mod log;
mod proto;
mod send;
mod serve;

use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() {
    let result = run().await;
    if let Err(e) = result {
        // `send` runs standalone (not under Virtuoso) — stderr is fine.
        // `serve` / `forward` should have logged to file before propagating.
        eprintln!("via: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Serve(args) => serve::run(args).await,
        cli::Command::Forward(args) => forward::run(args).await,
        cli::Command::Send(args) => send::run(args).await,
    }
}
