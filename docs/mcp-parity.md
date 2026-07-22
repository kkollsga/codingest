# MCP parity: codingest builder, KGLite server

## The ownership boundary

`codingest-mcp` combines two deliberately separate components:

- codingest builds code graphs from local or public-repository source;
- `kglite-mcp-server` owns the generic workspace lifecycle, graph activation,
  watching, Cypher, schema discovery, and source-reading tools.

KGLite removed its in-tree `code_tree` builder on 2026-07-16. A standalone
`kglite-mcp-server` can still serve an existing graph, but it refuses to build
a source workspace without a graph producer. `codingest-mcp` supplies that
producer and is therefore the supported builder-backed server.

## The generic workspace extension

KGLite 0.14.5 exposes the builder seam through `WorkspaceGraphHooks` and
`ServerExtensions`:

```rust
let hooks = WorkspaceGraphHooks {
    build: Box::new(|request: WorkspaceGraphRequest| {
        // Build either the current tree or request.revisions().
        // Return WorkspaceGraphResult, including canonical revision labels.
    }),
    is_relevant: Box::new(|change| is_graph_source(change.path())),
};

kglite_mcp_server::run_with_extensions(
    std::env::args_os(),
    ServerExtensions::default().with_workspace_graph(hooks),
)
```

The single build closure receives the root, activation mode, and optional
revision set through `WorkspaceGraphRequest`. codingest owns revision
canonicalization and returns a `WorkspaceGraphResult`; KGLite owns the active
slot transaction and records the returned labels only after a successful,
generation-safe activation.

The relevance closure includes registered source extensions plus Markdown and
reStructuredText, ensuring documentation-backed graph nodes are refreshed when
their inputs change.

## Ingestion policy

Policy stays on the builder side. `codingest-mcp` uses
`WorkspaceGraphRequest::mode()` to include repository documentation for the
GitHub workspace mode while keeping ordinary local/watch builds code-only.
KGLite does not need language, parser, or documentation policy.

## Fidelity guarantee

The MCP surface serves the same graph produced by the public codingest builder.
That output is guarded by frozen golden digests in
`crates/codingest/tests/parity.rs`, captured while codingest was verified
byte-identical to the last in-tree authority, plus the independent-build query
parity checks in `codingest_bench`.

This keeps a clean release boundary: codingest can evolve parsers and graph
construction independently, while KGLite can evolve storage, Cypher, and the
generic MCP lifecycle without acquiring code-specific policy.
