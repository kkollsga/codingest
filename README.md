# codingest

[![CI](https://github.com/kkollsga/codingest/actions/workflows/ci.yml/badge.svg)](https://github.com/kkollsga/codingest/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/codingest.svg)](https://crates.io/crates/codingest)
[![PyPI](https://img.shields.io/pypi/v/codingest.svg)](https://pypi.org/project/codingest/)
[![Docs](https://readthedocs.org/projects/codingest/badge/?version=latest)](https://codingest.readthedocs.io)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Parse polyglot codebases into queryable
[kglite](https://github.com/kkollsga/kglite) knowledge graphs — tree-sitter
parsers for 14 languages, call / type / inheritance / route edges, an optional
markdown-docs pass, and multi-git-revision merged graphs.

codingest is the standalone home of KGLite's former in-tree `code_tree`
component. The graph engine (storage backends, the Cypher pipeline, `.kgl`
persistence) and the MCP protocol server are **imported from kglite as cargo
libraries** — this repo owns only the code-tree component itself. Four surfaces
ship from one workspace: a **CLI**, an **MCP server**, a **Python wheel**, and a
**Rust crate**.

Documentation: **[codingest.readthedocs.io](https://codingest.readthedocs.io)**

## Requirements — kglite ≥ 0.14

codingest builds against 0.14-only engine APIs (`kglite::api::code_entities`,
`kglite_mcp_server::run_with_code_tree_hooks`) that KGLite added when it removed
its in-tree builder. **These are not in any 0.13.x release.**

- **Rust / crates.io:** codingest cannot be published, and a checkout without a
  sibling `../KGLite` cannot build, until **kglite 0.14.0 is on crates.io**.
  Local development uses a path dependency on the sister checkout (see
  [Dependency policy](#dependency-policy)).
- **Python wheel:** `pip install codingest` pulls `kglite>=0.13` for the
  loader half of the handoff, but the *builder* half needs a kglite that
  understands 0.14 graphs. Install **`kglite>=0.14`** alongside it.

## Install

```bash
pip install codingest          # Python wheel (grammars bundled) — ALSO installs the `codingest` CLI
cargo install codingest-cli    # pure-Rust `codingest` builder CLI (no Python)
cargo install codingest-mcp    # the code-graph MCP server
```

The Python wheel bundles the `codingest` terminal command (the `codingest-cli`
Rust library is linked into the wheel's extension), so `pip install codingest`
alone gives you both `import codingest` and `codingest build`/`status` — the
`cargo install codingest-cli` route is only needed for a pure-Rust install.
This makes the pip-only flow `pip install kglite codingest && kglite skill
install` self-sufficient (the installed code-review skill shells out to
`codingest build`/`status`).

(All become available once kglite 0.14.0 is published — see
[Requirements](#requirements--kglite--014).)

## Quick start

### CLI

```bash
# Build a code graph from a checkout
codingest build /path/to/repo
# → /path/to/repo/.kglite/code-review.kgl

# Build committed content at specific revisions (multi-rev merged graph)
codingest build /path/to/repo --revs v1.0 v2.0

# Check whether an existing graph is stale relative to the tree
codingest status /path/to/repo
```

Then query or describe the `.kgl` with kglite (`kglite.load(...)`, the `kglite`
shell, or any MCP/Bolt client).

### MCP server

```bash
# Serve a live code-graph workbench over stdio for an MCP client / agent
codingest-mcp --root-dir /path/to/repo
```

`codingest-mcp` embeds the `kglite-mcp-server` tool surface (`set_root_dir`,
`graph_overview`, `cypher_query`, `read_code_source`, …) and injects the
codingest builder, so `set_root_dir` builds a graph. Running plain
`kglite-mcp-server` against a workspace **refuses to build** — it has no
in-tree builder anymore. See the [MCP docs](https://codingest.readthedocs.io).

### Python wheel

```python
import codingest

g = codingest.build(".")                 # returns a real kglite.KnowledgeGraph
g.cypher("MATCH (f:Function) RETURN f.name LIMIT 10")
codingest.build(".", save_to="code.kgl") # also persist the .kgl
codingest.build(".", rev="v1.0")         # build a git revision (or revs=[...])
codingest.repo_tree("owner/name")        # clone a GitHub repo and build
```

**The `.kgl`-bytes handoff.** The `codingest` and `kglite` wheels are two
separate compiled extensions and can't share live Rust objects, so `build()`
constructs the graph with codingest's native builder, serializes it to a `.kgl`,
then calls the *installed* `kglite` wheel's `load()` and returns **that** object
— a genuine `kglite.KnowledgeGraph`, so every downstream kglite API works.

**Bundled CLI.** The same wheel also provides the `codingest` terminal command
(a `codingest/cli.py` console-script shim forwarding into the linked
`codingest-cli` library), so `codingest build`/`status` works straight after
`pip install codingest` — identical semantics to `cargo install codingest-cli`.

### Rust crate

```toml
[dependencies]
codingest = "0.1"   # needs kglite 0.14 on crates.io (see Requirements)
kglite = "0.14"
```

```rust
use codingest::build_code_tree; // = builder::run_with_options
let graph = build_code_tree(dir, /*verbose=*/false, /*include_tests=*/true,
                            /*save_to=*/None, /*max_loc=*/None, /*include_docs=*/false)?;
// Query with kglite's Cypher pipeline, persist with kglite::api::io, …
```

## Workspace layout

| Crate | What it is |
|---|---|
| `crates/codingest` | The component library (`codingest`): builder, parsers, manifest reader, docs pass, multi-rev merge, cross-language edges. Extracted from the former `KGLite/crates/kglite/src/code_tree/` (removed upstream 2026-07-16) and re-targeted at the public `kglite::api` facade. Ships the `codingest_stats` + `codingest_bench` binaries. |
| `crates/codingest-cli` | `codingest` binary — `build` a checkout or git revision(s) into a `.kgl` graph, `status` to check staleness. |
| `crates/codingest-mcp` | `codingest-mcp` binary — the full MCP tool surface imported from the `kglite-mcp-server` library, with the codingest builder injected. |
| `crates/codingest-py` | PyO3 wrapper built by maturin into the `codingest` wheel (`pip install codingest`). Python package source is `codingest/`; `pyproject.toml` drives the maturin build. Not published to crates.io (`publish = false`). |

## CI-equivalent local gate: `make gate`

`make gate` is the single-entry-point local gate that mirrors CI (`cargo fmt
--check`, `cargo clippy --workspace --all-targets -- -D warnings`, workspace
build + test incl. the golden oracle) and adds codingest-specific checks
(determinism reproducer, `codingest_bench` parity smoke, the wheel build + the
`tests/python` acceptance suite). Run it before pushing. Individual steps are
also targets (`make clippy`, `make determinism`, …); `make fmt` auto-formats.

### The golden-digest oracle

KGLite deleted its in-tree `code_tree` builder on 2026-07-16, so the old
two-builder parity sweep is gone. The authority it enforced was **frozen** while
the builders were still verified identical, into per-corpus SHA-256 digests
under `crates/codingest/tests/goldens/` (committed fixtures — no network). The
`golden_parity` test builds each corpus with only the codingest builder, digests
a canonical exhaustive graph rendering, and compares to the frozen golden. The
multi-rev fixture is guarded instead by `rev_self_consistency`. Regenerate
goldens only for deliberate builder-behavior changes:
`cargo test -p codingest --test parity -- --ignored capture_goldens` (details in
`crates/codingest/tests/goldens/README.md`).

## Dependency policy

`kglite` and `kglite-mcp-server` are path dependencies on the sister checkout
`../KGLite` with a `version = "0.13"` crates.io fallback. The fallback is kept
at `0.13` (not `0.14`) **only** because the local sister checkout is still
versioned 0.13.4 while carrying the 0.14 APIs — a path dep's version requirement
must be satisfiable by the path crate, so `0.14` would break the local build
today. When kglite 0.14.0 ships, the maintainer bumps both fallbacks to `0.14`
and flips the `CODINGEST_KGLITE_READY` CI variable (see the workspace
`Cargo.toml` note and `.github/workflows/ci.yml`).

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
