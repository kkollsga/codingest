# Migrating from `kglite.code_tree` (≤ 0.13)

Through kglite 0.13, the code-graph builder shipped **inside** kglite — as
`kglite.code_tree` / `kglite.build_code_tree` (Python), `kglite::api::code_tree`
(Rust), the `kglite code-tree` CLI subcommand, and the builder behind the MCP
server's `set_root_dir`. As of 2026-07-16 that in-tree builder was removed from
kglite and now lives here, in **codingest**. If you built code graphs with
kglite, switch to codingest.

Everything else — the engine, Cypher, `.kgl` files, the MCP server's *graph*
tools — is unchanged, and every `.kgl` you saved keeps loading. codingest-built
graphs are ordinary kglite graphs.

## Install

```bash
pip install codingest          # the builder + compatible kglite engine
```

You keep `kglite` for the graph engine (querying, `.kgl` persistence, the MCP
graph tools). codingest adds the builder back on top and returns
`kglite.KnowledgeGraph` objects.

## What moved where

| You used (kglite ≤ 0.13) | Now (codingest) |
|---|---|
| `kglite.code_tree.build(...)` / `kglite.build_code_tree(...)` / `repo_tree(...)` | `import codingest`; `codingest.build(...)` / `codingest.repo_tree(...)` — returns a real `kglite.KnowledgeGraph` |
| `kglite code-tree build/status` (CLI) | `codingest build` / `codingest status` (installed by `pip install codingest`) |
| `kglite skill install` (code-review skill) | `codingest skill install`; managed legacy installs migrate automatically |
| MCP workspace code-graph building (`set_root_dir`, `repo_management`) | Run **codingest-mcp** instead of `kglite-mcp-server` — it embeds the same server and injects the builder; every graph tool behaves identically |
| `kglite::api::code_tree::*` (Rust) | the `codingest` crate (`codingest::build_code_tree`, …) |

The **read-side** code-entity helpers stayed in kglite, at
`kglite::api::code_entities` — codingest depends on them (and currently
requires kglite ≥ 0.16.20).

## Python import changes

| Before (kglite ≤ 0.13) | Now (codingest) |
|---|---|
| `from kglite.code_tree import build` | `from codingest import build` |
| `import kglite; kglite.code_tree.build(dir)` | `import codingest; codingest.build(dir)` |

The signatures are preserved — `build(src_dir, *, save_to, verbose,
include_tests, max_loc_per_file, include_docs, rev, revs, repo_root)`,
`repo_tree(...)`, `read_manifest(path)`, `language_for_path(path)`. One
convenience: codingest bundles the tree-sitter grammars, so the old
`import tree_sitter` guard is gone.

The graph-engine imports stay on kglite:

```python
from kglite import KnowledgeGraph, load   # unchanged
```

## Rust import changes

```toml
[dependencies]
kglite = "0.16.20"    # the engine (graph build, persistence, code_entities)
codingest = "0.2"      # the builder
```

```rust
use codingest::build_code_tree;   // was kglite::api::code_tree::build_code_tree
```

## MCP config change

Point your MCP client at the `codingest-mcp` binary instead of
`kglite-mcp-server`. `kglite-mcp-server` still *serves* an already-built `.kgl`,
but it refuses to build a workspace from source (no in-tree builder anymore) —
`codingest-mcp` injects the builder, so `set_root_dir` works again. All graph
tools (`cypher_query`, `graph_overview`, `read_code_source`, …) are identical.
The command is included in the Codingest wheel; no Cargo installation is
needed.

## If anything broke: pin the old kglite

```bash
pip install "kglite<0.14"
```

0.13.x stays on PyPI. Pin, keep working, migrate when ready.

## Why the split

The builder carries 16 tree-sitter grammars that the kglite engine should not
have to; extracting it dropped kglite's wheel from ~40 MB to ~19 MB and lets the
builder and engine release on their own cadence. Extraction fidelity is enforced
by frozen golden digests captured while both copies were verified byte-identical
— see [Parity & goldens](parity-and-goldens.md).
