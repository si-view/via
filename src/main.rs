mod cli;
mod codec;
mod forward;
mod kill;
mod list;
mod log;
mod process;
mod proto;
mod send;
mod serve;
mod start;

use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() {
    let result = run().await;
    if let Err(e) = result {
        // `send` / `start` / `list` / `kill` run standalone — stderr is fine.
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
        cli::Command::Start(args) => tokio::task::spawn_blocking(|| start::run(args))
            .await
            .unwrap(),
        cli::Command::List(args) => tokio::task::spawn_blocking(|| list::run(args))
            .await
            .unwrap(),
        cli::Command::Kill(args) => kill::run(args).await,
    }
}
