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
mod query;
mod skill;
mod skill_assets;

use std::ffi::OsString;

use anyhow::Result;
use clap::Parser;

pub use query::StaleGraph;

/// Process exit code for an error returned by [`run`] — the CI contract.
///
/// `0` success and `2` usage errors are clap's, emitted before [`run`] is
/// reached. `3` is reserved for a `--require-fresh` refusal so a pipeline can
/// tell "the graph is stale" apart from "the query failed"; every other
/// operational failure (missing artifact, bad Cypher, timeout, I/O) is `1`.
///
/// The `pip install codingest` wheel's `_run_cli` maps every error to
/// `PyRuntimeError`, so a stale refusal exits `1` through the console script.
pub fn exit_code_for(error: &anyhow::Error) -> i32 {
    if error.downcast_ref::<StaleGraph>().is_some() {
        3
    } else {
        1
    }
}

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
