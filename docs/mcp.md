# MCP server

`codingest-mcp` is the code-graph workbench for MCP clients and agents. It
embeds the `kglite-mcp-server` query and source-reading surface and injects the
codingest builder so those tools operate on a freshly built code graph.

```bash
pip install codingest
codingest-mcp --watch /absolute/path/to/repo
```

The Codingest wheel installs this command. Its thin builder composition reuses
KGLite's graph server and the transitive `mcp-methods` MCP infrastructure; no
separate server or Cargo installation is required.

For one repository, add that command to the common MCP client configuration
shape:

```json
{
  "mcpServers": {
    "code-review": {
      "command": "codingest-mcp",
      "args": ["--watch", "/absolute/path/to/repo"]
    }
  }
}
```

The graph is ready when the server starts and is rebuilt lazily after relevant
source changes.

For an agent that switches among several local repositories, create
`workspace_mcp.yaml`:

```yaml
workspace:
  kind: local
  root: /absolute/path/to/starting-repository
  sandbox_root: /absolute/path/to/repository-parent   # the actual boundary
  watch: true
```

Then configure the client command as `codingest-mcp --mcp-config
/absolute/path/to/workspace_mcp.yaml`. This local-workspace mode registers
`set_root_dir`, which lets an agent swap the active repository at runtime.

**`root` is the starting root, not a boundary.** It sets where the server
points initially; on its own it does **not** constrain where `set_root_dir` can
subsequently point. Containment is `sandbox_root`, it is **opt-in**, and it
requires the kglite 0.15.5 engine floor: with it, a swap outside the boundary is
refused and the active root does not move; without it, a root swap is
unbounded.

> **Correction (2026-07-31).** Earlier revisions of this page stated that "every
> activated repository must stay within the declared sandbox." That was **false
> for every version of codingest that has shipped**. No containment existed: the
> read window was *derived from* the active root, so the source tools bounded
> reads relative to wherever the server already pointed, and never constrained
> where it could be pointed. The boundary those revisions described is real only
> from kglite 0.15.5 onward, and only when you set `sandbox_root`. If you relied
> on that sentence to confine an agent to a directory tree, **set `sandbox_root`
> explicitly** — the guarantee you were promised was not being enforced.

## codingest-mcp is the builder; `kglite-mcp-server` alone is not

When KGLite removed its in-tree `code_tree` builder (2026-07-16), the MCP server
lost its ability to build a workspace on its own. The generic server accepts a
`WorkspaceGraphHooks` producer through `ServerExtensions` and **refuses to
build a workspace when no producer is injected** — there is no in-tree
fallback left.

`codingest-mcp` is the process shell that injects those hooks, backed by this
project's builder. So:

- **`codingest-mcp`** builds and serves a code graph from a directory of source.
- **`kglite-mcp-server`** (standalone) still serves an already-built `.kgl`, but
  it will refuse to build a workspace from source.

The injected extension owns one unified plain/revision-set build closure plus
the watch predicate (`is a change to this path graph-relevant?`). Revision
requests return canonical labels together with the graph.
See [MCP parity](mcp-parity.md) for the coupling detail.

## Key tools

- **`set_root_dir(path)`** — in manifest-declared local-workspace mode, switch
  the active root and build that directory's graph.
- **`repo_management(...)`** — clone/activate/update a GitHub repo and build its
  graph (the open-source variant of `set_root_dir`).
- **`graph_overview()` / `cypher_query(...)`** — inventory node types, then
  query structure (calls, types, paths, counts).
- **`read_code_source(qualified_name=…)` / `read_source(...)`** — read the
  underlying source for a graph entity or a file slice.

## Watch semantics

With `--watch`, a change to a code file under the fixed root tags the graph
dirty and the rebuild fires lazily on the next graph tool call. In local-
workspace mode, the watcher monitors the wider configured tree but rebuilds only
for changes under the active `set_root_dir` target. (Watch scope is a
performance concern, not a security one — it decides what can *trigger a
rebuild*, never what can be read or activated. Confinement is `sandbox_root`,
above.) The graph is built in memory
and discarded on shutdown — nothing is written to disk unless you ask for it.

## Migrating an MCP config from kglite-mcp-server

Point your MCP client at the `codingest-mcp` binary instead of
`kglite-mcp-server`. Every graph tool behaves identically — the difference is
that `codingest-mcp` can build a workspace from source, which is what the code
graph needs. See [Migrating from kglite.code_tree](migrating-from-kglite-code-tree.md).
