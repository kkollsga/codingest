//! codingest-mcp — MCP server frontend for code-tree graphs.
//!
//! The entire server (tool surface, Cypher pipeline, code-tree
//! activation on `set_root_dir`, file watching) is imported from the
//! `kglite-mcp-server` library crate; this binary is only the process
//! shell. The one thing it injects is the code-tree *builder*: via
//! [`kglite_mcp_server::CodeTreeHooks`] it points the server's
//! `set_root_dir` build path at THIS workspace's `codingest` crate.
//! KGLite deleted its own in-tree `code_tree` on 2026-07-16, so the
//! server now REFUSES to build a workspace without builder hooks —
//! injecting them here is what keeps the MCP surface working, and it
//! makes `codingest` the sole builder behind it. Graph correctness is
//! guarded by the frozen golden digests in `crates/codingest/tests/`
//! (see `PARITY.md`), captured while codingest was verified byte-for-byte
//! identical to the last in-sync in-tree authority.

use kglite_mcp_server::CodeTreeHooks;

fn main() -> anyhow::Result<()> {
    let hooks = CodeTreeHooks {
        // Single-tree build: mirror the in-tree activation call shape
        // (`build_code_tree(dir, verbose=false, include_tests=true,
        // save_to=None, max_loc=None, include_docs)`).
        build: Box::new(|dir, include_docs| {
            codingest::build_code_tree(dir, false, true, None, None, include_docs)
        }),
        // Multi-rev build: the hook owns rev canonicalization — dedup the
        // requested labels, build over the deduped set, and return the graph
        // together with the canonical labels the server records on the slot.
        build_revs: Box::new(|dir, revs, include_docs| {
            let revs = codingest::dedup_revs(revs);
            let graph = codingest::build_code_tree_revs(
                dir,
                &revs,
                None,
                false,
                true,
                None,
                None,
                include_docs,
            )?;
            Ok((graph, revs))
        }),
        // Watch predicate: is a change to this path graph-relevant?
        is_code_file: Box::new(|p| codingest::language_for_path(p).is_some()),
    };

    kglite_mcp_server::run_with_code_tree_hooks(std::env::args_os(), Some(hooks))
}
