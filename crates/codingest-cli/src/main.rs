//! Standalone `codingest` CLI — build/status a `.kgl` code graph from a
//! checkout or git revisions. The `CodeTreeCommand` subcommands (Build /
//! Status) are exposed directly as the binary's top-level commands.

use anyhow::Result;
use clap::Parser;

mod code_tree_cli;

#[derive(Parser, Debug)]
#[command(name = "codingest", version, about)]
struct Cli {
    #[command(subcommand)]
    command: code_tree_cli::CodeTreeCommand,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    code_tree_cli::run(&cli.command)
}
