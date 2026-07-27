# Changelog

All notable changes to codingest are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add user-visible changes to `[Unreleased]` as you land them (per the
`phased-plan` skill). The `release` skill promotes `[Unreleased]` → `[x.y.z]` at
ship time — it's the only place the version bumps.

## [Unreleased]

### Changed
- Moved the kglite engine and MCP server pins to 0.15.0, and the Python engine
  requirement to `>=0.15.0,<0.16`. The upper bound keeps the two halves of the
  `.kgl` handoff — the Rust kglite compiled into the wheel that writes the
  bytes, and the separately-installed Python kglite wheel that reads them — on
  the same minor, mirroring the Cargo semver range exactly.
- Defaulted the `Makefile` Python-wheel gate (`make gate` steps 6-7) to a
  codingest-local `.venv` instead of the sibling KGLite checkout's `.venv`, and
  made the wheel step print the absolute path it writes into. The previous default made
  a gate in this repo `maturin develop --release` into *another repo's*
  environment, replacing whatever extension that repo's own conventions require,
  with no warning in either repo. Sharing an environment is now opt-in via
  `VENV=...`.
- **`codingest_bench` now defines its own corpus.** It copies the target's
  git-tracked files into a temporary directory and builds that, and prints
  `corpus_sha256` (plus file/byte counts) with every run. Previously it built
  the target directory as it sat on disk; because the builder has no notion of
  `.gitignore`, a repository's untracked working state was ingested through the
  docs pass, and the measured input could not be reconstructed on another
  machine or at another time — a single scratch `.md` file moved this
  workspace's graph. `--include-untracked` restores the old behaviour for
  one-off measurement of a non-git tree and prints a NOT-REPRODUCIBLE banner.
  Unknown flags are now rejected instead of ignored. **Node/edge counts and
  timings published before 2026-07-27 are not comparable with later ones** —
  see the notice at the top of `BENCHMARKS.md`.
- Made `golden_parity` build each corpus three times and require every build to
  agree with every other build as well as with the frozen golden, so it is the
  builder-determinism gate in addition to the behaviour gate. This replaced a
  `make gate` step that ran three builds of an unrelated *sibling* checkout and
  asserted an exact edge count against it: that verdict depended on a repository
  this project does not own (upstream refactoring alone turned it red), it never
  ran in CI, and it skipped silently when the sibling was absent. Determinism is
  now hermetic, in-repo, and CI-enforced on both OSes. `make determinism-soak
  REPO=…` keeps the large-repo reproducer as an opt-in diagnostic.

## [0.1.3] - 2026-07-22

### Added
- Added AGC `JUMPS_TO`, `BRANCHES_TO`, `ALIAS_OF`, and `POINTS_TO`
  relationships, program-local data access metadata, erasable-storage flags,
  and resolved BANKCALL/IBNKCALL/POSTJUMP destinations.

### Changed
- Separated AGC returning calls, unconditional jumps, and conditional branches
  into `CALLS`, `JUMPS_TO`, and `BRANCHES_TO`, preserving source operands and
  offsets while leaving register and relative-only destinations unresolved.

### Fixed
- Made `pip install codingest` install the builder-aware `codingest-mcp`
  console command. The wheel now bundles Codingest's thin builder composition
  over KGLite's graph server and transitive `mcp-methods` infrastructure, so
  Python users no longer need Cargo or a separately rebuilt MCP binary.
- Removed false cross-program AGC references and false edges to inter-bank
  trampoline helpers; preserved BANKJUMP/SWCALL as unresolved indirect sites.

## [0.1.2] - 2026-07-22

### Added
- Added hand-written yaYUL AGC assembly (`.agc`) parsing for program-scoped
  labels, constants, transfers, references, and `$` includes.
- Added `codingest skill install|uninstall` for user- or project-scoped Codex
  and Claude Code installations of Codingest's code-review Agent Skill.

### Changed
- Refreshed every direct dependency to its current supported release, adopted
  SHA-2 0.11, removed four unused Rust dependencies, declared Rust 1.88 as the
  MSRV, and pinned the tested Python 3.10/3.14 and documentation toolchains.
- Reworked the PyPI/README quick start around agent MCP setup, local code
  analysis, and one-call GitHub repository analysis; aligned the Python engine
  floor and live MCP documentation with KGLite 0.14.5.
- Moved ownership and distribution of the code-review Agent Skill from KGLite
  to Codingest. Installation safely migrates KGLite-managed legacy copies while
  preserving unmanaged directories.
- Migrated `codingest-mcp` to KGLite's generic workspace-graph lifecycle
  (`WorkspaceGraphHooks`): one unified plain/revision-set build closure plus a
  watch-relevance policy, with document-ingestion policy owned here (markdown
  `:Doc` nodes for the github-workspace mode only). This adopts KGLite 0.14.5's
  generation-safe activation transactions and coherent active-graph identity.

### Fixed
- Preserved same-scope function overloads as distinct, stable graph nodes and
  resolved calls conservatively across every matching overload.
- Validated and refreshed cached repository clones on every build, rejected
  dirty or wrong-origin caches, and kept GitHub credentials out of process
  arguments and diagnostic text.
- Rebuilt MCP graphs after Markdown/reStructuredText edits and made CLI status
  detect truncated or replaced graph artifacts as stale.
- Accepted manifest paths for revision builds, rejected sources outside an
  explicit repository root, and kept duplicate-node provenance lists aligned.
- Preserved relative directories in fallback module IDs so nested same-name
  HTML, CSS, PHP, Swift, and Dart files no longer collapse together.
- Made manifest-driven builds reject malformed manifests, avoid overlapping
  source/test walks, honor test exclusion for broad and fallback scans, and
  report every parsed project language.
- Prevented ordinary comment prose from becoming TODO-style annotations,
  preserved multiline annotation locations, and made generated-file detection
  robust when its scan window ends inside a UTF-8 character.
- Restored PHP, C/C++, and Swift module hierarchy edges and resolved local
  C/C++ includes, HTML/CSS assets, and SwiftPM target imports against their
  actual project files/modules.
- Resolved program-local AGC transfers to dotted labels such as `P61.1`
  without leaking calls across AGC programs.
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
    `make gate` determinism reproducer. (Superseded in `[Unreleased]`: the
    reproducer moved into `golden_parity`'s repeat-build loop.)
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
