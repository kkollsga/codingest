//! codingest-mcp — MCP server frontend for code-tree graphs.
//!
//! The entire server (tool surface, Cypher pipeline, workspace
//! activation on `set_root_dir`, file watching) is imported from the
//! `kglite-mcp-server` library crate; this binary is only the process
//! shell. The one thing it injects is the workspace-graph *producer*:
//! via [`kglite_mcp_server::WorkspaceGraphHooks`] it points the
//! server's activation and watch-rebuild paths at THIS workspace's
//! `codingest` crate. KGLite deleted its own in-tree `code_tree` on
//! 2026-07-16, so the server REFUSES to build a workspace without an
//! injected producer — these hooks are what keep the MCP surface
//! working, and they make `codingest` the sole builder behind it.
//! Graph correctness is guarded by the frozen golden digests in
//! `crates/codingest/tests/` (see `PARITY.md`), captured while
//! codingest was verified byte-for-byte identical to the last in-sync
//! in-tree authority.
//!
//! Ingestion policy lives HERE, not in the server: the producer decides
//! from [`WorkspaceGraphRequest::mode`] whether repository markdown is
//! ingested as `:Doc` nodes, and owns revision canonicalization for
//! revision-set builds.

use kglite_mcp_server::{
    ServerExtensions, WorkspaceGraphHooks, WorkspaceGraphMode, WorkspaceGraphRequest,
    WorkspaceGraphResult,
};
use std::path::Path;

fn is_graph_source(path: &Path) -> bool {
    codingest::language_for_path(path).is_some()
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("rst")
            })
}

fn main() -> anyhow::Result<()> {
    let hooks = WorkspaceGraphHooks {
        // Unified plain/revision-set build. Call shapes mirror the previous
        // in-tree activation (`build_code_tree(dir, verbose=false,
        // include_tests=true, save_to=None, max_loc=None, include_docs)`).
        build: Box::new(|request: WorkspaceGraphRequest| {
            // The github-workspace (open-source) mode ingests each cloned
            // repo's markdown as `:Doc` nodes and links them to code
            // (MENTIONS/DOCUMENTS) — a repo's prose is part of its
            // intelligence. Local workspace / watch modes keep the lean
            // code-only graph.
            let include_docs = matches!(request.mode(), WorkspaceGraphMode::Workspace);
            match request.revisions() {
                // Multi-rev build: the producer owns rev canonicalization —
                // dedup the requested labels, build over the deduped set, and
                // return the graph together with the canonical labels the
                // server records on the slot.
                Some(revisions) => {
                    let revisions = codingest::dedup_revs(revisions);
                    let graph = codingest::build_code_tree_revs(
                        request.root(),
                        &revisions,
                        None,
                        false,
                        true,
                        None,
                        None,
                        include_docs,
                    )?;
                    Ok(WorkspaceGraphResult::with_revisions(graph, revisions))
                }
                None => {
                    let graph = codingest::build_code_tree(
                        request.root(),
                        false,
                        true,
                        None,
                        None,
                        include_docs,
                    )?;
                    Ok(WorkspaceGraphResult::new(graph))
                }
            }
        }),
        // Watch relevance: is a change to this path graph-relevant?
        is_relevant: Box::new(|change| is_graph_source(change.path())),
    };

    kglite_mcp_server::run_with_extensions(
        std::env::args_os(),
        ServerExtensions::default().with_workspace_graph(hooks),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_source_predicate_includes_code_and_docs_only() {
        assert!(is_graph_source(Path::new("src/lib.rs")));
        assert!(is_graph_source(Path::new("README.md")));
        assert!(is_graph_source(Path::new("GUIDE.RST")));
        assert!(!is_graph_source(Path::new("notes.txt")));
        assert!(!is_graph_source(Path::new("artifact.kgl")));
    }
}
