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

## Requires kglite ≥ 0.17.5

codingest builds against engine APIs (`kglite::api::code_entities`,
`WorkspaceGraphHooks`, and `ServerExtensions`) exposed after KGLite removed its
in-tree builder. The floor sits at 0.17.5 to keep the Rust writer and the
Python reader on one engine release. Two MCP workspace fixes reach the embedded
`codingest-mcp` server. A relative sandbox path declared in a manifest now
resolves from the manifest's own directory rather than the process working
directory, so a server launched from elsewhere no longer sandboxes the wrong
tree. And the workspace watcher no longer replaces a built graph when a source
file is only *read* — the server's own `read_source`, an editor opening a file
— so a review session stops paying a full rebuild for its own reads; only real
mutations rebuild. Neither fix needed a source change here.

The preceding 0.17.4 floor rejects every missing Cypher parameter
before candidate selection, so an empty label scan, inline map or `WHERE`
cannot hide an absent binding behind zero rows. Installing or clearing a
declared schema refreshes cached diagnostics. The embedded MCP server
returns bounded, navigable previews backed by retained complete results, while
`codingest query --format json/csv` remains complete.

Beneath it, the 0.17.3 floor took 0.17.2 and 0.17.3 together, and neither needed
a source change here. Read `CALL` subqueries accept modern scope clauses and set
composition; procedures and read subqueries obey their incoming row pipeline;
Cypher 25 adds `FILTER`, `OFFSET`, `NODETACH DELETE`, `FINISH`, and `INSERT`.
Typed and undirected relationship counts and bound-node incident predicates
are faster, while correlated `EXISTS` patterns and fused aggregates no longer
drop endpoint constraints or null relationship bindings. These query-engine
changes reach `codingest query` and the embedded `codingest-mcp` server; the
builder and its `.kgl` persistence entries are unchanged.

Below that, 0.17.1 spans four upstream releases — codingest took none of
0.16.23, 0.16.24 or 0.17.0 — and none of them needed a source change here. In
the embedded MCP server, every route that reaches the
engine (`cypher_query`, manifest `tools[].cypher` templates, recipe queries)
now runs under a 180-second query deadline; it had none before, so a runaway
query held the graph's read lock and took the whole server with it. The
`cypher_query` inline preview renders list, map, node, relationship and path
cells as natural JSON (`{"start":1,"end":2,"type":"CALLS"}`) rather than
serde's tagged `Value` encoding, and a nested null is `null` instead of the
string `"Null"` — an agent parsing that text sees different field names.
In the engine, `codingest query --format csv` keeps numeric and timestamp
precision instead of compacting it, `ORDER BY` on a key an earlier `WITH`
produced is honoured instead of silently returning input order, and `RETURN *`
with `ORDER BY … LIMIT` returns rows instead of a `*` column of `1`s. Loading a
`.kgl` is slower — 0.17.0 normalizes legacy stored endpoint references on every
complete-snapshot load (12-15% on an ordinary portable file upstream), paid
once by `build()`'s handoff, not per query. `.kgl` persistence is byte-identical
and the golden digests prove
it. Beneath it, 0.16.22 spans two upstream releases: a `skills:` pack a
manifest declares but that does not exist on disk fails the boot instead of
booting a server with *every* skill silently gone, `save_graph(force: true)` is
offered only where mutations are (`--writable` / `extensions.writable: true`)
and refused on a `builtins.save_graph`-only server, a fired query deadline
raises `CypherTimeoutError` rather than `CypherExecutionError` — relevant to
`codingest query --timeout`, whose refusal comes from kglite — relationship
alternation `[:A|B]` works inside `EXISTS { }` / `count { }` / `size(...)`, and
ranked retrieval on a property with no index raises instead of answering zero
rows. Beneath it, 0.16.20 changes what the embedded MCP
server *says*, not what it computes: the `<active_graph>` header and the
`— active graph:` footer now report `load="N"` / `· load N ·` where they said
`generation`, and gain a `file_saved` field carrying the served artifact's
publish time (omitted for a workspace graph, which is what `--watch` and
`set_root_dir` serve). `reload_graph` answers "Load N on this server." A
write-enabled server's `save_graph` with nothing unsaved is now a no-op
(`Nothing to save: …`) instead of republishing identical bytes, `force: true`
still rewrites on purpose, and `extensions.writable: true` in a manifest says
what `--writable` says on the command line — whereas `builtins.save_graph:
true` alone registers only `save_graph` and leaves `cypher_query` read-only
(always the behaviour; the operator docs said otherwise). No engine, `.kgl`, or
Cypher change. Beneath it, 0.16.19 is the lazy-writer-lease and
automatic-refresh release for the embedded MCP server: a write-enabled server
takes the served `.kgl`'s writer lease at its first unsaved change rather
than at boot (several clients can boot off one manifest), a `--graph` server
re-reads the file automatically once its identity changes on disk — so a
`codingest build` that replaces the artifact reaches the server on the next
tool call — `save_graph` refuses to overwrite a file another writer changed,
and `extensions.graph_watch` is retired because the refresh is now
unconditional. Beneath it, 0.16.18 hardens the embedded MCP server's
boot — a CSV listener or source root that cannot start degrades that
peripheral with a warning instead of killing the server, and an omitted
`csv_http_server` port binds OS-assigned again — with no engine, API, or
Cypher change. Beneath it, 0.16.17 shrinks the dependency tree for
Rust consumers (kglite now pulls `geo` without its default features — five
transitive packages gone; no API or Cypher behavior change, and codingest has
no direct `geo` dependency to re-declare). Beneath it, 0.16.16 makes the deadline the CLI's
documented `--timeout` flag sets actually observed inside the MATCH row loops
and variable-length path expansion — before it, a query could run arbitrarily
far past its deadline once the pattern matcher had finished — with the timeout
contract itself unchanged; its one removal, the never-written
`QueryDiagnostics::timed_out` field, is a symbol codingest names nowhere.
Beneath it, 0.16.14 fixes two things codingest actually ships: two saves of one
graph write identical `.kgl` bytes again (four persisted lists were written in
hash-map iteration order), and a reloaded `.kgl` no longer reports every
relationship type as having zero edges to `describe()` and the planner's
selectivity estimates. 0.16.15's load-memory program (`LoadOptions`,
`estimate_load_memory` / `max_load_mb`, the `row_limit` result cap) is additive
at every surface codingest names. Behind them, 0.16.13 through 0.16.7 change
nothing codingest calls (lockstep refreshes), for two distinct reasons worth
keeping apart. The ontology declaration layer (0.16.11–0.16.13)
and the BM25 text index (0.16.10) are **opt-in surface codingest declares
nothing into**. 0.16.13's engine fixes are a different argument: they correct
an *index* answering wrongly, and codingest **builds no index at all**, so they
are unreachable here rather than unused. That second argument expires the day
anyone adds one. Separately, 0.16.9 speeds up loading digest-carrying `.kgl`
files (~1.6x) without changing a byte of what is written. Beneath them, 0.16.6 fixes the advisory
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
languages
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
