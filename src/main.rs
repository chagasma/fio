mod cli;
mod context;
mod hooks;
mod index;
mod model;
mod store;
mod tui;
mod view;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    cli::run(cli::Cli::parse().command)
}
