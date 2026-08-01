# Changelog

All notable changes to codingest are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add user-visible changes to `[Unreleased]` as you land them (per the
`phased-plan` skill). The `release` skill promotes `[Unreleased]` → `[x.y.z]` at
ship time — it's the only place the version bumps.

## [Unreleased]

### Added
- **`codingest query "<cypher>"` (visible alias `cypher`)** — a one-shot,
  read-only Cypher query against a saved `.kgl`, the second interface alongside
  the MCP server for CI, cross-session artifact reuse, and non-MCP hosts. It
  queries an artifact and never builds one: `codingest build <dir> && codingest
  query '<cypher>'` composes the two. `-g/--graph` selects the artifact
  (default `.kglite/code-review.kgl`; `--graph`, not `--output`, because here
  the artifact is an input), `-` as the query reads it from stdin, and
  `--timeout <secs>` bounds execution. Output is **never truncated** — unlike
  the MCP server's 15-row inline preview, which is a host-context budget; use
  Cypher `LIMIT` to bound rows. The default rendering is TSV (a header line of
  column names, then every row) on stdout with an `N row(s)` summary on stderr,
  so stdout stays pure data. `--format csv` emits `CypherResult::to_csv()`
  verbatim — byte-identical to the MCP server's `FORMAT CSV` export — and
  `--format json` emits one compact `{"columns": […], "rows": [[…]]}` object per
  query. An in-query `FORMAT CSV` overrides `--format`, so a query renders the
  same on the CLI as it does through MCP. `EXPLAIN` renders its plan rows like
  any other result. Mutation Cypher is rejected by the engine's read path.
- **`docs/mcp.md` now has an opencode section**, verified against the shipping
  `opencode` binary at `v1.2.25-1505` (not the `lildax` v2 rewrite, which does
  not wire MCP tools up yet). It documents the zero-absolute-path global config
  block — `["codingest-mcp", "--watch", "."]`, which works because opencode
  spawns local servers with their working directory set to the instance
  directory — plus the V1 config key names, the real 30 s connect/request
  timeout (opencode's own docs say 5 s), the 2000-line / 50 KiB tool-output cap
  and what actually trips it, the fact that a `--host claude` skill install is
  already discovered as-is, the measured cost of opting into the manifest's
  `skills:` key (tool descriptions go from 3.3 KB to 40.5 KB), the root-mechanism
  decision table, and how to triage a server that will not start.
- **The sample `workspace_mcp.yaml` in `docs/mcp.md` now carries an
  `instructions:` block** with the graph-first routing doctrine (`graph_overview`
  → `cypher_query`; `grep`/`read_source` for literal text only) and one line of
  result-budget discipline. Hosts that inject MCP `initialize` instructions into
  the system prompt — opencode does, verbatim, every session — now get the
  routing rule without the operator writing it themselves. The server preserves
  a manifest's `instructions:` and appends its own tool-discovery steer on top.

### Security
- **`docs/mcp.md` documented a containment boundary that was never enforced.**
  The local-workspace section stated that "every activated repository must stay
  within the declared sandbox". That was false for every version of codingest
  that has shipped: `workspace.root` set where the server *started*, and nothing
  constrained where `set_root_dir` could subsequently point. The read window was
  derived *from* the active root, so the source tools bounded reads relative to
  wherever the server already pointed rather than confining where it could be
  pointed. Anyone who read that sentence as confining an agent to a directory
  tree did not have the guarantee they were promised.

  The page now describes `root` as the starting root and **not** a boundary, and
  documents `workspace.sandbox_root` — opt-in, requires the kglite 0.15.5 floor
  raised below — as the real containment: with it a swap outside the boundary is
  refused and the active root does not move. The correction is stated in place
  rather than silently edited away, because a reader who already configured
  against the old sentence needs to know to add the key. The watch-scope
  paragraph no longer calls the watched tree "the sandbox" either — watch scope
  decides what can *trigger a rebuild*, never what can be read or activated.

### Changed
- **Engine floor moved to kglite 0.15.5**, skipping 0.15.4. 0.15.4 was WAL,
  durability and mapped-graph work that codingest consumes none of — we use no
  `durable=` graph, no `MappedGraph`, and no disk storage — so bumping to it
  would have been cost without benefit. 0.15.5 is the release we wanted: it adds
  `workspace.sandbox_root` and `workspace.adopt_client_roots`, the containment
  boundary and MCP-client root adoption codingest requested upstream. The exact
  `kglite==0.15.5` pin the CI wheel test installs moves with it, so that gate
  validates the engine we ship against. Parity goldens did not move.

  Note for anyone building on `adopt_client_roots`: MCP `roots` was deprecated
  upstream in protocol revision `2026-07-28` (SEP-2577). The key works and is
  inert when unset, but passing the directory as a tool parameter or server
  configuration is the spec's own migration path.
- A release runs to completion again. Between 2026-07-30 and 2026-07-31 the
  publish push took a separate blocking confirmation; that is reverted.
  Invoking `/release` authorizes the whole run including the tag push, which is
  now preceded by a *report* rather than a gate. The blocking prompt fired after
  the decision it claimed to guard — by the time the release commit exists the
  bump, constants and CHANGELOG are settled — and it broke unattended runs,
  where the failure mode is publishing nothing silently rather than publishing
  something wrong. The safety on that push lives in checks that can fail (green
  CI, the resolving `cargo metadata`, the `--dry-run --workspace` preflight,
  parity, artifact-set verification), all upstream of it. The release and
  phased-plan runbooks now also state their completion condition and name the
  pause points that are not endings.
- The release procedure now dry-runs the crates.io publish before tagging.
  `release.yml` publishes crates.io **first** and hangs every other job off it,
  so a packaging or metadata fault in `codingest-cli` or `codingest-mcp` used to
  surface only after `codingest` was already published permanently — a
  half-published release with no undo. `cargo publish --dry-run --workspace`
  packages all three crates and builds each packaged copy up front. The
  `--workspace` flag is required rather than incidental: a bare
  `--dry-run -p codingest-cli` resolves the internal dependency against
  crates.io, where the new version does not exist yet, and fails on resolution
  rather than on any real defect. The wheel and sdist contract checks
  (`verify_wheel.py`, the sdist LICENSE count) are preflighted locally for the
  same reason — both otherwise run only after crates.io has published.

## [0.1.4] - 2026-07-30

### Changed
- Moved the kglite engine and MCP server pins to 0.15.3, and the Python engine
  requirement to `>=0.15.3,<0.16` (including the exact `kglite==0.15.3` pin the
  CI wheel test installs, so that gate validates the engine we actually ship
  against). The upper bound keeps the two halves of the `.kgl` handoff — the
  Rust kglite compiled into the wheel that writes the bytes, and the
  separately-installed Python kglite wheel that reads them — on the same minor,
  mirroring the Cargo semver range exactly. The 0.15.1-0.15.3 patches change
  nothing in `kglite::api`, graph output, property encoding, `.kgl`
  serialization, or the MCP server interface, and the parity goldens are
  unchanged; they add a declared `rust-version = 1.88.0` on both crates we
  consume (matching our own floor), fix a `storage="disk"` save that could emit
  a directory the same build could not load, widen the unknown-label diagnostic
  to subqueries, and correct understated dependency floors.
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
- Hardened the release workflow against checks that could not report failure.
  Ten such gates were found on the publish path; the four externally reported
  ones were the least serious. The extracted version is now asserted to be
  well-formed (the old `grep … | cut` reported *cut's* status, always 0), the
  wheel and sdist uploads set `if-no-files-found: error` instead of the default
  `warn`, the artifact *set* is asserted against the build matrix rather than
  `ls`-ed, an inconclusive crates.io probe fails loudly instead of silently
  skipping all three publishes, a missing CHANGELOG section is fatal rather than
  degrading to auto-generated notes, and `continue-on-error` is narrowed from
  the whole `release-binaries` job to its three genuinely fragile steps.
  `workflow_dispatch` is removed as a trigger: on a dispatch run the ref is a
  branch, and `softprops/action-gh-release` handed a non-tag ref creates a tag
  and release named after it. Because `release.yml` runs only on a `v*` tag and
  so can never be *seen* to fail on a branch, the logic now lives in
  `scripts/release_gates.sh` behind 206 offline tests that drive every function
  through both its pass and its fail path on every push.

### Fixed
- **The five internal path-dependency pins now match the workspace version.**
  Each crate's own `package.version` inherits `[workspace.package] version`, but
  the *requirement* on an internal path dependency does not — it is a
  hand-written literal that `cargo publish` emits verbatim. All five had sat at
  `0.1.0` across two releases, so published `codingest-cli 0.1.3` declared a
  dependency on `codingest ^0.1.0`. Nothing broke in the field (`^0.1.0`
  resolves to `0.1.x`), but the published metadata was wrong, it would have
  broken outright at the first minor bump, and it was already wrong under
  minimal-versions resolution. A discovery-based gate now asserts every internal
  pin against the workspace version on each release, deriving the site list from
  `[workspace] members` so a newly added crate cannot slip past it.
- Removed a `dev-docs/` citation from committed source (`rev.rs`). `dev-docs/`
  is gitignored working state, so the reference outlived the file and was
  already dead for anyone cloning the repository.

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
