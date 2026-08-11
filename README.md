# codingest

[![CI](https://github.com/kkollsga/codingest/actions/workflows/ci.yml/badge.svg)](https://github.com/kkollsga/codingest/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/codingest.svg)](https://crates.io/crates/codingest)
[![PyPI](https://img.shields.io/pypi/v/codingest.svg)](https://pypi.org/project/codingest/)
[![Docs](https://readthedocs.org/projects/codingest/badge/?version=latest)](https://codingest.readthedocs.io)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Give AI agents a live, queryable map of any codebase — locally, privately, and
without running the code you are analysing.

codingest turns polyglot source into a
[KGLite](https://github.com/kkollsga/kglite) knowledge graph of functions,
types, calls, imports, inheritance, routes, documentation, and git revisions.
Use it from Python, the command line, or as an MCP server for coding agents.

```bash
pip install codingest
```

```python
import codingest

graph = codingest.build(".")
graph.cypher("MATCH (f:Function) RETURN f.name LIMIT 10")
```

No language servers, databases, or repository-specific build steps are
required. The native Rust builder bundles parsers for 15 languages and returns
a real `kglite.KnowledgeGraph`, ready for Cypher queries.

## Built for agents and code analysis

- **Agent-ready MCP:** give an MCP client `graph_overview`, `cypher_query`,
  `read_code_source`, repository switching, and automatic graph refresh.
  [opencode is set up and verified end to end](https://codingest.readthedocs.io/en/latest/mcp.html#using-codingest-mcp-with-opencode)
  — including a config that needs no absolute paths — and `codingest skill
  install` packages the review skill for Claude Code and Codex, which opencode
  also discovers with no extra step.
- **Fast local review:** map definitions, callers, dependencies, routes,
  inheritance, and affected tests without uploading or executing the project.
- **Open-source intelligence:** clone and analyse a GitHub repository with one
  Python call, or let an agent manage cached public repositories through MCP.
- **Polyglot by default:** one graph across Python, Rust, TypeScript,
  JavaScript, Java, Go, C/C++, C#, Swift, PHP, HTML, CSS, Dart, and AGC
  assembly.
- **Revision-aware:** analyse a tag, branch, commit, or multiple revisions in
  one graph and query what changed.

AGC assembly receives architecture-aware control and data relationships rather
than a generic text-level call graph; see the [AGC graph model](https://codingest.readthedocs.io/en/latest/agc-assembly.html).

## Give your agent a code-review MCP server

```bash
pip install codingest
codingest-mcp --watch /absolute/path/to/repo
```

Point an MCP client at the same command:

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

The server builds the graph at startup, refreshes it when source files change,
and gives the agent schema discovery, Cypher, and source-reading tools over
stdio. For a multi-repository agent sandbox with runtime `set_root_dir`, use
the local-workspace manifest described in the [MCP guide](https://codingest.readthedocs.io/en/latest/mcp.html).

For a zero-configuration CLI workflow, install the packaged code-review Agent
Skill instead:

```bash
pip install codingest
codingest skill install
```

The skill teaches compatible agents to build a fresh graph, discover its
schema before querying, combine Cypher with the git diff, and verify findings
against source lines. See the [MCP guide](https://codingest.readthedocs.io/en/latest/mcp.html)
and [CLI guide](https://codingest.readthedocs.io/en/latest/cli.html).

## Analyse a local codebase

```python
import codingest

graph = codingest.build(
    ".",
    include_docs=True,
)

rows = graph.cypher("""
MATCH (caller:Function)-[:CALLS]->(target:Function)
RETURN target.qualified_name, count(caller) AS callers
ORDER BY callers DESC
LIMIT 10
""")

codingest.build(".", save_to="code-review.kgl")
```

The bundled terminal command offers the same workflow:

```bash
codingest build /path/to/repo
codingest status --output /path/to/repo/.kglite/code-review.kgl
codingest query -g /path/to/repo/.kglite/code-review.kgl \
  "MATCH (f:Function)-[:CALLS]->(g:Function) RETURN f.name, g.name LIMIT 20"
kglite describe /path/to/repo/.kglite/code-review.kgl --connections
```

`codingest query` prints unbounded TSV to stdout (use `--format csv|json`, or
Cypher `LIMIT` to bound rows), warns on stderr when the graph has gone stale,
and with `--require-fresh` refuses a stale graph with exit code 3 — see
[docs/cli.md](docs/cli.md).

## Analyse an open-source repository

```python
import codingest

graph = codingest.repo_tree("pallets/flask")
graph.cypher("""
MATCH (route:Function)-[:CALLS]->(dependency:Function)
RETURN route.qualified_name, dependency.qualified_name
LIMIT 25
""")
```

`repo_tree()` shallow-clones the requested repository into a temporary
directory, builds the graph, and cleans up afterward. Pass `branch=`,
`clone_to=`, or `token=` for a specific revision, a reusable cache, or a
private repository.

## Compare revisions

```python
graph = codingest.build(".", revs=["v1.0", "v2.0"])
graph.cypher("""
MATCH (f:Function)
WHERE 'v2.0' IN f.revs AND NOT 'v1.0' IN f.revs
RETURN f.qualified_name
""")
```

Revision builds use tracked git content without changing the working tree.
Multi-revision graphs store shared entities once and expose revision membership
for direct Cypher analysis.

## Install options

```bash
pip install codingest          # Python API + CLI + MCP server + KGLite runtime
codingest skill install        # code-review Agent Skill
```

The wheel bundles every grammar plus the `codingest` and `codingest-mcp`
commands. KGLite ≥0.15.11 is installed automatically as the query/storage
engine; its MCP server and the transitive `mcp-methods` framework power the
builder-aware Codingest server. Nothing else needs to be installed.

Rust-only environments can instead use `cargo install codingest-cli
codingest-mcp`; these are alternative distributions of the same commands, not
Python prerequisites.

### Rust crate

```toml
[dependencies]
codingest = "0.1"
kglite = "0.15.11"
```

```rust
use codingest::build_code_tree;

let graph = build_code_tree(
    dir,
    false, // verbose
    true,  // include tests
    None,  // save path
    None,  // max lines per file
    false, // include docs
)?;
```

## How it fits together

codingest owns code-graph construction, the Python/CLI interfaces, the
builder-backed MCP executable, and the code-review Agent Skill. KGLite provides
the graph engine, Cypher, persistence, and reusable query/read tools. Keeping
that boundary explicit lets codingest focus on accurate code understanding
while reusing a dedicated graph engine.

Documentation: **[codingest.readthedocs.io](https://codingest.readthedocs.io)**

## Workspace layout

| Crate | What it is |
|---|---|
| `crates/codingest` | The component library (`codingest`): builder, parsers, manifest reader, docs pass, multi-rev merge, cross-language edges. Extracted from the former `KGLite/crates/kglite/src/code_tree/` (removed upstream 2026-07-16) and re-targeted at the public `kglite::api` facade. Ships the `codingest_stats` + `codingest_bench` binaries. |
| `crates/codingest-cli` | `codingest` binary — `build` a checkout or git revision(s) into a `.kgl` graph, `status` to check staleness, and `skill` to install Codingest's code-review Agent Skill. |
| `crates/codingest-mcp` | `codingest-mcp` binary — the full MCP tool surface imported from the `kglite-mcp-server` library, with the codingest builder injected. |
| `crates/codingest-py` | PyO3 wrapper built by maturin into the `codingest` wheel (`pip install codingest`). Python package source is `codingest/`; `pyproject.toml` drives the maturin build. Not published to crates.io (`publish = false`). |

## CI-equivalent local gate

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p codingest --test parity
```

### The golden-digest oracle (also the determinism gate)

KGLite deleted its in-tree `code_tree` builder on 2026-07-16, so the old
two-builder parity sweep is gone. The authority it enforced was **frozen** while
the builders were still verified identical, into per-corpus SHA-256 digests
under `crates/codingest/tests/goldens/` (committed fixtures — no network). The
`golden_parity` test builds each corpus **three times** with only the codingest
builder, digests a canonical exhaustive graph rendering, and requires every
build to match every other build *and* the frozen golden. Repeating the build is
what makes this the determinism gate: hash iteration order is randomized per
`HashMap`, so an order-dependent builder fails it (the
`tests/corpus/dup_minified_assets` fixture reproduces the original DEFINES-edge
nondeterminism bug). Disagreement between builds is reported as NONDETERMINISM;
agreement between builds but not with the golden is reported as a behaviour
change. The multi-rev fixture is guarded instead by `rev_self_consistency`.
Regenerate goldens only for deliberate builder-behavior changes:
`cargo test -p codingest --test parity -- --ignored capture_goldens` (details in
`crates/codingest/tests/goldens/README.md`).

Everything the gate needs is committed to this repository, so it is hermetic and
runs in CI. It replaced a local-only `make` step that asserted an exact edge
count against a *sibling* checkout — a verdict this project did not control.
`make determinism-soak REPO=…` keeps the large-repo reproducer available as a
diagnostic.

## Dependency policy

`kglite` and `kglite-mcp-server` use matching crates.io requirements with a
0.15.11 minimum and a shared lockfile. This keeps the builder, persistence
handoff, and embedded MCP server on one compatible engine patch line.

## Parity with the (now-removed) in-tree component

codingest was extracted to be feature- and performance-identical to kglite's
in-tree `code_tree`. KGLite removed that module on 2026-07-16, so parity is now
enforced against a **frozen record** of its last-known-good output rather than
live cross-comparison. See `PARITY.md` (stats-diff + timing), `BENCHMARKS.md`
(build-time + Cypher benchmarks), `crates/codingest/tests/parity.rs` (the golden
oracle), and `docs/mcp-parity.md` (the MCP↔builder coupling and the hook).

`tests/python-legacy/` preserves KGLite's full 47-file `kglite.code_tree`
behavioral suite verbatim as the dormant behavioral spec — the source of truth
for what the Python builder guaranteed (see its README).

## License

MIT © Kristian dF Kollsgård. codingest is an independent project; it depends on
`kglite` at runtime but is not otherwise affiliated with it.
