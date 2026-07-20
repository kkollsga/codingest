# MCP server

`codingest-mcp` is the code-graph workbench for MCP clients and agents. It
embeds the entire `kglite-mcp-server` tool surface — `set_root_dir`,
`graph_overview`, `cypher_query`, `read_code_source`, `read_source`, `grep`,
`list_source`, … — and injects the codingest builder so those tools operate on a
freshly built code graph.

```bash
cargo install codingest-mcp
codingest-mcp --root-dir /path/to/repo    # stdio MCP server
```

## codingest-mcp is the builder; `kglite-mcp-server` alone is not

When KGLite removed its in-tree `code_tree` builder (2026-07-16), the MCP server
lost its ability to build a workspace on its own. The server now exposes
`run_with_code_tree_hooks(args, Option<CodeTreeHooks>)` and **refuses to build a
workspace when no hooks are injected** — there is no in-tree fallback left.

`codingest-mcp` is the process shell that injects those hooks, backed by this
project's builder. So:

- **`codingest-mcp`** builds and serves a code graph from a directory of source.
- **`kglite-mcp-server`** (standalone) still serves an already-built `.kgl`, but
  it will refuse to build a workspace from source.

The injected hooks are exactly three call sites — the single-tree build, the
multi-rev build (the hook owns rev de-duplication and returns the canonical
labels), and the watch predicate (`is a change to this path graph-relevant?`).
See [MCP parity](mcp-parity.md) for the coupling detail.

## Key tools

- **`set_root_dir(path)`** — switch the active root; the code graph is
  (re)built for that directory. This is the build entry point that the injected
  hook drives.
- **`repo_management(...)`** — clone/activate/update a GitHub repo and build its
  graph (the open-source variant of `set_root_dir`).
- **`graph_overview()` / `cypher_query(...)`** — inventory node types, then
  query structure (calls, types, paths, counts).
- **`read_code_source(qualified_name=…)` / `read_source(...)`** — read the
  underlying source for a graph entity or a file slice.

## Watch semantics

File-watching is on: a change to a code file **under the active root** tags the
graph dirty, and the rebuild fires lazily on the next graph tool call. The
watcher monitors the wider sandbox cheaply (FSEvents/inotify) but only rebuilds
for changes under the active `set_root_dir` target. The graph is built in memory
and discarded on shutdown — nothing is written to disk unless you ask for it.

## Migrating an MCP config from kglite-mcp-server

Point your MCP client at the `codingest-mcp` binary instead of
`kglite-mcp-server`. Every graph tool behaves identically — the difference is
that `codingest-mcp` can build a workspace from source, which is what the code
graph needs. See [Migrating from kglite.code_tree](migrating-from-kglite-code-tree.md).
