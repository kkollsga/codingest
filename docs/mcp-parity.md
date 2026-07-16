# MCP parity: current state and the upstream hook proposal

## Where MCP↔code-tree coupling actually lives

`kglite-mcp-server` touches the code-tree *builder* in exactly three places
(everything else — `cypher_query`, `graph_overview`, `read_code_source`'s
`source_location`/`SourceLookup` — operates on the engine or an already-built
`DirGraph` and is builder-agnostic). Each site now dispatches through the
`CodeTreeHooks` closures; the "What it does" column below shows the call shape
the injected `codingest` builder receives (previously the deleted in-tree
`kglite::api::code_tree::*` functions, called directly):

| Site | What it does |
|---|---|
| `lib.rs` watch predicate | `is_code_file(p)` → `codingest::language_for_path(p).is_some()` (is this file graph-relevant?) |
| `GraphState::build_code_tree` | `build(dir, include_docs)` → `codingest::build_code_tree(dir, false, true, None, None, include_docs)` on `set_root_dir` / lazy rebuild |
| `GraphState::build_code_tree_revs` | `build_revs(dir, revs, include_docs)` → `dedup_revs(..)` + `codingest::build_code_tree_revs(dir, &revs, None, false, true, None, None, include_docs)` for multi-rev activation |

## Current state: codingest is the sole builder

KGLite deleted its in-tree `code_tree` builder on 2026-07-16. The
`kglite-mcp-server` build path now **requires** builder hooks: with `None` it
refuses to build a workspace (there is no in-tree fallback left to call).
`codingest-mcp` injects hooks backed by this crate, so the `codingest-mcp`
binary is the only thing that can serve a graph — and it serves graphs built by
*this* workspace's crate exclusively.

Graph fidelity is guarded by the frozen golden digests in
`crates/codingest/tests/` (captured while codingest was verified byte-for-byte
identical to the last in-sync in-tree authority — see `PARITY.md` and
`crates/codingest/tests/parity.rs`), plus the cross-build query-result parity
check in `codingest_bench`. The MCP surface therefore serves exactly the graph
`golden_parity` pins.

Historically (before the hook landed) the build calls were hard-wired to the
in-tree builder with no injection point; the hook closed that gap, and the
subsequent upstream deletion made the hook the *only* build path.

## The upstream hook (IMPLEMENTED)

`kglite-mcp-server` already shipped this pattern for Python embedders:

```rust
pub fn run(args) -> Result<()>                 // thin: run_with_embedder_factory(args, None)
pub fn run_with_embedder_factory(args, Option<PyEmbedderFactory>) -> Result<()>
```

The analogous code-tree hook is now **implemented upstream** (all inside
`kglite-mcp-server`; no `kglite` core change needed). The real, shipped
signatures — note `build_revs` returns a **tuple** so the hook owns rev
canonicalization:

```rust
/// Builder hooks for the code-tree activation path. Since KGLite removed
/// its in-tree builder (2026-07-16), `None` no longer has a builder to
/// fall back to — the server refuses to build a workspace without hooks.
pub struct CodeTreeHooks {
    pub build: Box<
        dyn Fn(&Path, /*include_docs:*/ bool) -> Result<Arc<DirGraph>, String>
            + Send + Sync,
    >,
    /// Returns the graph AND the canonical (deduped) rev labels — the hook
    /// owns rev dedup; the server records the returned labels on the active
    /// slot and surfaces them in the activation banner.
    pub build_revs: Box<
        dyn Fn(&Path, &[String], /*include_docs:*/ bool)
                -> Result<(Arc<DirGraph>, Vec<String>), String>
            + Send + Sync,
    >,
    /// Watch predicate: does this path affect the code graph?
    pub is_code_file: Box<dyn Fn(&Path) -> bool + Send + Sync>,
}

pub fn run_with_code_tree_hooks(args, hooks: Option<CodeTreeHooks>) -> Result<()>
```

Plumbing: `Option<Arc<CodeTreeHooks>>` stored on `GraphState`, branched at the
three sites above, threaded through `run_async` exactly like
`py_embedder_factory`. With the in-tree builder deleted (2026-07-16), the `None`
branch no longer has an in-tree call to fall back to — the server refuses the
build instead, so a real builder must be injected (as `codingest-mcp` does).

`crates/codingest-mcp/src/main.rs` here now **injects** this workspace's
builder:

```rust
fn main() -> anyhow::Result<()> {
    let hooks = kglite_mcp_server::CodeTreeHooks {
        build: Box::new(|dir, include_docs| {
            codingest::build_code_tree(dir, false, true, None, None, include_docs)
        }),
        build_revs: Box::new(|dir, revs, include_docs| {
            let revs = codingest::dedup_revs(revs);
            let graph = codingest::build_code_tree_revs(
                dir, &revs, None, false, true, None, None, include_docs,
            )?;
            Ok((graph, revs))
        }),
        is_code_file: Box::new(|p| codingest::language_for_path(p).is_some()),
    };
    kglite_mcp_server::run_with_code_tree_hooks(std::env::args_os(), Some(hooks))
}
```

The `build_revs` closure deduplicates the requested labels itself
(`codingest::dedup_revs`) and returns `(graph, revs)`, matching the tuple the
hook expects — the server no longer assumes an in-tree `dedup_revs`.

## The longer-term alternative

KGLite dropped its in-tree `code_tree` on 2026-07-16. The remaining
consolidation step would be to invert the dependency outright:
`kglite-mcp-server` (and `kglite-cli`) depend on the standalone `codingest`
crate directly, so no hook injection is needed. That is a single source of
truth, but it couples KGLite's release train to this repo — the hook keeps the
two release trains independent, which is why it remains the mechanism today.
