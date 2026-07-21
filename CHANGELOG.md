# Changelog

All notable changes to codingest are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add user-visible changes to `[Unreleased]` as you land them (per the
`phased-plan` skill). The `release` skill promotes `[Unreleased]` → `[x.y.z]` at
ship time — it's the only place the version bumps.

## [Unreleased]

### Fixed
- Restored PHP, C/C++, and Swift module hierarchy edges and resolved local
  C/C++ includes, HTML/CSS assets, and SwiftPM target imports against their
  actual project files/modules.
- Scoped call-resolution noise names to languages present in the parsed file
  set, so foreign stdlib names no longer hide valid project CALLS edges in
  single-language repositories.

## [0.1.1] - 2026-07-20

### Changed
- Raised the Rust and Python engine floor to KGLite 0.14.4, adopting its
  Postcard-only persistence stack and current MCP override/context fixes.
- Enabled the full GitHub Actions test matrix now that the required KGLite
  release is available from crates.io and PyPI.

### Fixed
- Stabilized codingest-owned graph insertion order for external type stubs,
  file-import aggregates, `USES_TYPE` matches, and documentation edge groups.
- Consolidated marker-specific and mixed-manifest dependency variants by their
  logical graph ID, preserving every constraint without duplicate nodes/edges.
- Corrected the Python sdist license payload and added a release check requiring
  exactly one packaged `LICENSE` file.

## [0.1.0] - 2026-07-16
Initial public release. codingest is the standalone home of KGLite's code-tree
component, extracted so the kglite engine can ship without tree-sitter grammars.

> **Requires kglite ≥ 0.14.** codingest builds against 0.14-only engine APIs
> (`kglite::api::code_entities`, `kglite_mcp_server::run_with_code_tree_hooks`)
> that are not in any 0.13.x release. Nothing here can be published to
> crates.io / PyPI until kglite 0.14.0 ships; see the README "Requirements" and
> the workspace `Cargo.toml` dependency note.

### Added
- **`codingest` builder library** (`crates/codingest`) — the code-tree
  component extracted from KGLite's former `crates/kglite/src/code_tree/`
  (removed upstream 2026-07-16) and re-targeted at the public `kglite::api`
  facade: tree-sitter parsers for 14 languages, call / type / inheritance /
  route edges, an optional markdown-docs pass (`docs` feature → `:Doc` nodes),
  multi-git-revision merged graphs, and the manifest reader. Ships the
  `codingest_stats` accuracy harness and the `codingest_bench` query/parity
  benchmark.
- **`codingest` CLI** (`crates/codingest-cli`, binary `codingest`) — `build` a
  checkout or git revision(s) into a `.kgl` graph, `status` to check staleness.
  Port of KGLite's former `kglite code-tree` subcommand.
- **`codingest-mcp` server** (`crates/codingest-mcp`) — the full MCP tool
  surface (`set_root_dir`, `graph_overview`, `cypher_query`, `read_code_source`,
  `read_source`, `grep`, `list_source`, …) imported from the
  `kglite-mcp-server` library. It injects `CodeTreeHooks` backed by this
  workspace's builder; since KGLite removed its in-tree builder, the server
  **refuses to build a workspace without these hooks**, so codingest-mcp is the
  sole builder behind the MCP surface (`kglite-mcp-server` alone still serves an
  existing `.kgl`).
- **`codingest` Python wheel** (`pip install codingest`, `crates/codingest-py`)
  — a maturin/PyO3 extension resurrecting the builder surface kglite 0.14
  removed: `build(src_dir, *, save_to, verbose, include_tests, max_loc_per_file,
  include_docs, rev, revs, repo_root)`, `repo_tree(...)`, `read_manifest(path)`,
  and `language_for_path(path)`. `build()` returns a real
  `kglite.KnowledgeGraph` via a `.kgl`-bytes handoff (build native → serialize →
  the installed `kglite` wheel's `load()`), so every downstream kglite API works.
  Ships type stubs (`codingest/__init__.pyi`) and the `tests/python` acceptance
  suite. The wheel also **bundles the `codingest` terminal command** — the
  `codingest-cli` Rust library is linked into the wheel's extension and a thin
  `codingest/cli.py` shim (`[project.scripts] codingest = "codingest.cli:main"`)
  forwards `sys.argv[1:]` into it via `codingest._run_cli`. So `pip install
  codingest` provides the same `codingest build`/`status` command as `cargo
  install codingest-cli`, with no second wheel or duplicated builder build;
  cargo remains the pure-Rust route. This makes the pip-only workflow
  `pip install kglite codingest && kglite skill install` self-sufficient (the
  installed code-review skill shells out to `codingest build`/`status`).

### Parity & provenance
- Full feature- and performance-parity with the (now-removed) in-tree
  `kglite::code_tree`, originally proven by a live two-builder equivalence sweep.
  KGLite deleted its in-tree builder on 2026-07-16; parity is now enforced
  against a **frozen record** captured while both builders were verified
  byte-for-byte identical:
  - `golden_parity` (`crates/codingest/tests/parity.rs`) builds each corpus with
    the codingest builder and compares a canonical exhaustive graph digest to
    the frozen per-corpus SHA-256 goldens under
    `crates/codingest/tests/goldens/`.
  - `rev_self_consistency` guards the multi-rev `revs`/`rev_fp` stamping path
    (which can't be frozen — fresh commit SHAs leak into the graph).
  - `codingest_bench` asserts cross-build query-result parity (determinism).
  - The DEFINES-edge nondeterminism bug (randomized HashMap iteration over
    duplicate `(file, entity)` pairs) is fixed (BTreeMap + within-pair
    consolidation) and guarded by the `dup_minified_assets` corpus + the
    `make gate` determinism reproducer.
- `tests/python-legacy/` preserves KGLite's full 47-file `kglite.code_tree`
  behavioral suite verbatim as the dormant behavioral spec (see its README).

### Packaging & automation
- Workspace release profile mirrors KGLite's (`lto = "thin"`,
  `codegen-units = 1`, stripped symbols).
- CI (`.github/workflows/ci.yml`): Rust (fmt / clippy / workspace test incl. the
  golden oracle) + Python (maturin wheel + `tests/python`) on ubuntu + macOS.
  Gated on the `CODINGEST_KGLITE_READY` repo variable until kglite 0.14.0 is on
  crates.io.
- Release (`.github/workflows/release.yml`, tag `v*`): ordered crates.io publish
  (`codingest` → `codingest-cli` → `codingest-mcp`, 404-guarded) then a maturin
  wheel matrix + sdist published to PyPI via Trusted Publishing.
- Docs at [codingest.readthedocs.io](https://codingest.readthedocs.io).
