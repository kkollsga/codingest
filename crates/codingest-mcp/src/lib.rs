//! Reusable Codingest MCP server composition.
//!
//! KGLite owns the graph/Cypher server and `mcp-methods` owns the generic MCP
//! lifecycle. This crate contributes the Codingest workspace-graph producer —
//! source parsing, revision builds, and watch relevance — and the producer's
//! own methodology: the `code_review` skill and the `code_review/*` recipe
//! catalogue from `codingest::methodology`, registered once per server so they
//! apply to every graph it serves in every mode, including the manifest-less
//! workspace boot where graph-carried records are never read (kglite 0.17.7).
//! With no manifest, or one that never mentions `skills:`, that turns skills
//! on (kglite's bundled set plus ours); an explicit `skills: false` silences
//! both, and operator layers win collisions.

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

fn server_extensions() -> anyhow::Result<ServerExtensions> {
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

    // A producer record that fails validation fails the boot naming itself
    // (it is our code, not graph data); `codingest::methodology`'s unit tests
    // run the same compile, so this `?` fires in CI before it can at a user's.
    let recipes = codingest::methodology::recipe_catalog()
        .map_err(|error| anyhow::anyhow!("codingest recipe catalogue is invalid: {error}"))?;
    Ok(ServerExtensions::default()
        .with_workspace_graph(hooks)
        .with_skills([codingest::methodology::skill_record()])
        .with_recipes(recipes))
}

/// Run the KGLite MCP server with Codingest's workspace builder installed.
///
/// `args` includes the program name, matching `std::env::args_os()` and clap's
/// normal process-level contract. Both the standalone binary and the Python
/// wheel call this entry point so their behavior cannot drift.
pub fn run<I, T>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    kglite_mcp_server::run_with_extensions(args, server_extensions()?)
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
