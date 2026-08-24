# codingest

Give AI agents a live, queryable map of any codebase. codingest builds
[KGLite](https://github.com/kkollsga/kglite) knowledge graphs with tree-sitter
parsers for 17 languages ([per-language support matrix](languages.md)), call / type / inheritance / route edges, an optional
documentation pass, and multi-git-revision merged graphs.

codingest owns code-graph construction: KGLite's former in-tree `code_tree`
component, its CLI and Python surfaces, the builder-backed MCP executable, and
the code-review Agent Skill. KGLite owns the graph engine and reusable
query/read infrastructure: storage, Cypher, `.kgl` persistence, code-entity
reads, and the underlying MCP server.

## Requires kglite ≥ 0.16.9

codingest builds against engine APIs (`kglite::api::code_entities`,
`WorkspaceGraphHooks`, and `ServerExtensions`) exposed after KGLite removed its
in-tree builder. The floor sits at 0.16.9 to keep the Rust writer and the
Python reader on one engine release; 0.16.9 speeds up loading digest-carrying
`.kgl` files (~1.6x) without changing a byte of what is written, and 0.16.9
and 0.16.8 — like 0.16.7 before them — change nothing codingest calls
(lockstep refreshes). Beneath them, 0.16.6 fixes the advisory
traversal caps and variable-length trail semantics, neither of which changes the
graph codingest produces. Beneath that, the 0.16.2 floor makes bulk-loaded edge
properties record
their observed types — every edge property codingest writes through
`add_connections` (`CALLS.call_count`, `IMPORTS.import_count`, …) reported
`Unknown` to schema consumers before it. It sits on 0.16.1's structured
wire-JSON shapes and the documentation contract for raw `NodeData` reads, on
top of 0.16.0's
columnar-from-first-node storage and `.kgl` v6; beneath those sit the
workspace lifecycle and
containment controls introduced through 0.15.5, the corrected mixed-selection
vector search, community modularity scoring, sampled-centrality validation
and persisted HNSW validation of 0.15.6, the mcp-methods 0.4.4 / rmcp 3.1.1
server integration of 0.15.7, 0.15.8's changed-path hints for workspace
graph producers, 0.15.10's per-call edge-property column resolution in
`add_connections`, and the `NodeView` / `DirGraph::set_node_property` API that
0.15.11 made reachable — codingest is built against that API and will not
compile on an earlier engine. Cargo and pip
install this compatible engine automatically; codingest adds the builder and
returns ordinary `kglite.KnowledgeGraph` objects.

## Install

```bash
pip install codingest          # Python API + CLI + MCP server + KGLite engine
codingest skill install        # the code-review Agent Skill
```

The wheel installs both `codingest` and `codingest-mcp`. Cargo installations
remain optional alternatives for Rust-only environments.

## 60-second quickstart

Build a graph, then query it with kglite:

```python
import codingest

g = codingest.build(".")                 # returns a real kglite.KnowledgeGraph
g.cypher("MATCH (f:Function) RETURN f.name LIMIT 10")
codingest.build(".", save_to="code.kgl") # also persist the .kgl
```

Or from the shell:

```bash
codingest build /path/to/repo            # → /path/to/repo/.kglite/code-review.kgl
codingest build /path/to/repo --revs v1.0 v2.0   # multi-rev merged graph
```

Or as a live agent workbench:

```bash
codingest-mcp --watch /absolute/path/to/repo   # live MCP server over stdio
```

## Contents

```{toctree}
:maxdepth: 1
:caption: Surfaces

cli
mcp
python-api
agc-assembly
```

```{toctree}
:maxdepth: 1
:caption: About

parity-and-goldens
mcp-parity
migrating-from-kglite-code-tree
```

## License

MIT © Kristian dF Kollsgård. codingest is an independent project; it depends on
`kglite` at runtime but is not otherwise affiliated with it.
