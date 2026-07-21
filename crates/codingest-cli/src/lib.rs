//! Shared implementation of the `codingest` CLI.
//!
//! `codingest` builds/checks a `.kgl` code graph from a checkout or one or more
//! git revisions and installs Codingest's code-review Agent Skill. The
//! `CodeTreeCommand` variants are the binary's top-level commands.
//!
//! Pure-Rust over the sibling `codingest` builder + `kglite::api::io` (no
//! libpython link in the standalone binary). The `pip install codingest` wheel
//! links this same library through `_run_cli`, so command parsing and behavior
//! cannot drift between the cargo binary and the console script.

mod code_tree_cli;
mod skill;
mod skill_assets;

use std::ffi::OsString;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "codingest", version, about)]
struct Cli {
    #[command(subcommand)]
    command: code_tree_cli::CodeTreeCommand,
}

/// Run the CLI over an explicit argument vector, including the program name.
///
/// The standalone binary and the `pip install codingest` wheel shim both call
/// this entry point, so command parsing and behavior cannot drift.
pub fn run<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    code_tree_cli::run(&cli.command)
}
