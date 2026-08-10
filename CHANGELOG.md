# Changelog

All notable changes to codingest are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add user-visible changes to `[Unreleased]` as you land them (per the
`phased-plan` skill). The `release` skill promotes `[Unreleased]` → `[x.y.z]` at
ship time — it's the only place the version bumps.

## [Unreleased]

### Added
- **A release perf anchor — committed per-release bench baselines (docs-on and
  docs-off) and `scripts/bench_anchor.sh`, which refuses cross-corpus
  comparison, voids on control movement, and blocks the release tag at +30 %
  per-row drift.** Release 0.1.6 published two perf breaches nothing caught —
  opencode nodes +13.03 % against a ≤12 % budget and build +20.16 % against
  ≤15 % — because those budgets lived in prose and were read by a human against
  the wrong denominator. Four verdicts, each a distinct exit code because
  "re-measure" and "you have a regression" are opposite instructions: PASS (0),
  FAIL (1, blocks the tag), REFUSE (3, corpus digest or docs mode differs — no
  delta is computed at all), VOID (4, the designated control query moved, so
  the capture reports no per-row verdicts). Queries are judged **per row
  returned**, never raw: 0.1.6 was 6 of 11 queries over a raw +10 % ceiling and
  every one was correct, the call graph having gotten denser on purpose. The
  anchor runs on the frozen `tests/corpus` fixture tree rather than this repo's
  own sources — those *are* the code under test, so their digest moves nearly
  every release, which would make the gate REFUSE every time. Baselines carry
  their own noise floors and their own control query, both derived from
  measured run-to-run spread. `tests/benchmarks/README.md` is the record.
- **`codingest_bench` gains `--no-docs`, and the JSON labels the mode.** The
  harness previously hardcoded the docs pass ON in both of its builds, so a
  recorded row's `"include_docs": true` was a literal, not a reading. It is now
  the *effective* value and is echoed in the JSON and in the human header either
  way, so no bench row can be filed without stating the mode it was taken under.
  The default stays docs-**ON** — the opposite of `codingest_stats`'
  `--include-docs`, and deliberately so: each binary's default must reproduce
  that binary's own historical rows, and the two have mirror-image histories.
  Unknown flags are still rejected, now including `--include-docs`: it is
  `codingest_stats`' spelling, and silently accepting it as a no-op would
  swallow a request to *change* the mode.
- **CI now runs the bench determinism smoke and verifies the wheel's pip
  contract before anything can publish.** A new `bench-smoke` job runs
  `make bench-smoke`, which builds `codingest_bench` `--release` and requires
  all 11 Cypher queries to return identical results across two independent
  builds *through the read path* (`execute_read`) — a determinism bug living in
  query evaluation rather than graph construction is invisible to `parity.rs`'s
  digest-of-the-graph check and visible here. It also fails if the harness falls
  back to a `working-tree` corpus. Until now it ran only in `make gate`, i.e.
  when someone remembered. Separately, `scripts/verify_wheel.py` ran **only** in
  `release.yml`, which triggers on a `v*` tag push — a broken console-script
  entry point was therefore first observed *after* the crates.io publish in the
  same workflow. One matrix leg of the `python` job (ubuntu + 3.14) now builds a
  real wheel with `maturin build` and verifies its entry points, payload shims,
  native extension and KGLite requirement on every push.
- **CI now lints its own workflows**, and the release-gate suite pins the CI
  KGLite install to the Cargo floor. A new `actionlint` job (pinned
  `rhysd/actionlint:1.7.12`) statically checks every file in
  `.github/workflows/` — undefined matrix keys and step-id references in
  `if:`/`${{ }}` expressions, plus shellcheck over every `run:` block. This
  matters most for `release.yml`, which runs only on a `v*` tag push and so was
  previously first exercised *during a release*. Separately,
  `test_kglite_cargo_and_python_floors_are_in_lockstep` gained a **third
  source**: it now also parses `kglite==` out of `ci.yml` and requires it to
  equal the Cargo floor. The two-source version could not see the CI pin sitting
  at 0.15.5 while the floor moved to 0.15.6 — the exact drift fixed in 9786c27,
  where CI's acceptance suite ran against a different engine than the Rust
  writer targets. An audit of the remaining version-pinned CI/release fixtures
  turned up two more pins asserted only by prose: `ci.yml` pins `pytest==` in
  two jobs under a comment claiming they match, and its `maturin==` install is
  what builds the extension the acceptance suite proves, while `pyproject.toml`
  separately declares the build-backend floor. Both now have a test.
- **Permanent `[timing]` diagnostics for the three previously unmeasured
  once-per-build costs**: graph persistence (`save_graph`), source
  fingerprinting (the freshness hash, which runs on `build`, `status` *and*
  every `query`) and manifest discovery. **Every `[timing]` line now respects
  `KGLITE_CODE_TREE_VERBOSE`** — the builder's phase timers (`walk`, `parse`,
  `parse dispatch`, `dedup`, `js workspace discovery`, `load`, `docs`,
  `cross-lang`) previously answered only to the `--verbose` build flag, so
  setting the documented env var printed an incomplete set. They now fire on
  either switch, which matters because the fingerprint path is reached from
  commands that have no `--verbose` flag at all. **All lines are stderr-only**,
  so `query --format json`
  and `status --format json` keep emitting a single clean JSON object on stdout
  with the switch set; a CLI test parses that stdout to hold the line.
- **Repositories without a recognized manifest now get an inferred `:Project`
  node.** Only `pyproject.toml` and `Cargo.toml` are read as manifests, so every
  other repository — `package.json`-only JS/TS, Go, Java, C++, or any plain
  directory — built with no `:Project` node at all: nothing owned its files, and
  docs were attached to nothing structural. Such a build now synthesizes a
  project named after the project root, with `languages` taken from the files
  actually parsed, anchoring every source file via `HAS_SOURCE` and every doc
  via the new **`HAS_DOC`** edge (`Project`→`Doc`, one per doc node, structural;
  the semantic `MENTIONS` / `DOCUMENTS` edges are unchanged). The Project's
  `manifest` property carries the sentinel `(inferred)` to mark it — so
  `MATCH (p:Project) WHERE p.manifest = '(inferred)'` finds these, and no new
  property was added to the node. Manifest-backed graphs are unchanged except
  for gaining `HAS_DOC`; the `rust_xfile` golden digest is byte-identical, which
  is the proof. Thirteen of the fourteen corpus goldens moved deliberately.
- **The Python acceptance suite now fails fast on hangs and refuses to run
  against a stale build.** Two independent false-green shapes are closed.
  (1) `pytest-timeout` is pinned in both CI pip-install sites and `pyproject.toml`
  sets `timeout = 120`, so a test that *blocks* — on the native extension, a
  subprocess CLI, or the MCP server's stdio loop — now fails as a named test
  after 120s instead of consuming the 45-minute job budget and reporting as a
  runner timeout that names nothing. A genuinely slow test opts out per-test with
  `@pytest.mark.timeout(<n>)`. (2) `tests/python/conftest.py` compares the mtime
  of the installed `codingest` extension against the newest `crates/**/*.rs`: if
  the extension is **older**, the suite hard-errors (exit 4) naming
  `.venv/bin/maturin develop --release`, because otherwise it would return a
  confident verdict about code that is no longer in the working tree. If the
  extension is **absent** it cleanly skips instead — nothing was built, so there
  is no false verdict to prevent. The distinction is the point: a guard that
  skipped on staleness would be disarmed by the exact condition it guards. The
  guard caught a real stale extension on its first run.

### Fixed
- **Two builds of the same git revision now agree on node ids.** The single-rev
  build path (`build(..., rev=…)`) extracted the revision into a randomly-named
  tempdir and built from it, and the build root's directory name is user-visible
  output — the manifestless fallback derives Python module names from it — so
  each build stamped a fresh `kglite-rev-XXXXXX` into every such id
  (`kglite-rev-xiPP2N.app.compute`). It now extracts into the same fixed
  `snapshot` basename the multi-rev path already used, so single-rev builds are
  reproducible and their ids also match the multi-rev builds of the same
  revision.
- **Python absolute imports now resolve for standard project layouts**
  (root-relative and `src/` layouts): `IMPORTS` `File→Module` and `File→File`
  edges are emitted, and `import_backed` becomes meaningful on Python `CALLS`
  edges. The Python parser prefixes module paths with the source root's own
  directory name (`pkg/app.py` → `myproj.pkg.app`) while specifiers are
  root-relative (`pkg.util`), so the resolver's prefix walk could never match
  and a Python project produced **zero** import edges; a `src/` layout added two
  such segments. The resolver now retries Python specifiers under the recovered
  root prefix after the plain walk misses. This **supersedes the 0.1.5 guidance
  below to not filter Python graphs on `import_backed`** — that guidance was
  correct for 0.1.5 through 0.1.7 and no longer applies. Only candidates that
  name a module the project actually defines become edges, so no target is
  invented; the clone layout that already worked
  (`xarray/core/dataset.py` → `xarray.core.dataset`) is unchanged, as are all
  TS/JS graphs. Two golden digests moved deliberately (`py_basic`,
  `py_nested_defs`). Python *relative* imports (`from .util import x`) are still
  dropped at parse time and remain unresolved.
- **Builds rooted at a dot-named or ignore-listed directory no longer produce a
  silently empty graph.** `WalkDir::filter_entry` applies its predicate to the
  walk *root* as well as its descendants, so pointing the builder at a `.`-named
  path (any bare `tempfile::tempdir()`, `~/.config/thing`), a checked-out
  `target/`, a `venv/` or a vendored `node_modules/` pruned the walk before it
  started — the build reported success and wrote a graph with no files in it.
  The name list now applies only below the root.
- **Cross-file call resolution now splits owner qualified-names at the last
  separator, matching the type-name derivation.** The owner/prefix split took
  the first separator present in list order (`::`, then `.`, then `/`) while
  `short_type_name` took the last one by position, so mixed-separator names
  (dotted directories in path-style qnames, e.g. `pkg/api/v1.2/handlers.Run`)
  previously narrowed the receiver and same-owner resolution tiers on a wrong
  owner, and could never match the inheritance tier at all.
- **Doc concept-ids now strip their extension case-insensitively.** The docs
  walk accepted `.md` / `.mdx` / `.rst` in any case while the id-stripper
  matched six literal suffixes, so a `Guide.Mdx` became a `:Doc` with the
  extension welded into its id — and since doc→doc links resolve against
  extension-stripped ids, nothing could ever link to it. Admission and
  stripping now derive from one table.
- **Same-name docs with different extensions in one directory no longer
  silently collapse.** `guide.md` beside `guide.mdx` (or `guide.rst`) mapped to
  one concept id and overwrote each other in the node DataFrame, so the
  surviving node's title, path and frontmatter came from whichever file the
  walk happened to reach second. Precedence is now explicit — `.mdx` > `.md` >
  `.rst` — the winner keeps the id, the losers are dropped from the graph, and
  each drop is reported with a warning naming both files.

### Changed
- **Declared external dependency floors now match what the code needs**, and a
  nightly `direct-minimal-versions` CI job keeps them honest. `anyhow = "1"`,
  `clap = "4"`, `regex = "1"` and `tempfile = "3"` each claimed the code built
  against that major's `.0.0` release; none of them did, and nothing tested the
  claim, because the committed `Cargo.lock` always pinned something far newer.
  `serde_json` was worse than understated — it was **inconsistent**, declared
  `1.0.151` by `codingest` and `1` by `codingest-cli`, a split that has no
  solution at all once floors are actually resolved. The floors now name the
  versions the workspace is built and tested against (`anyhow 1.0.104`,
  `clap 4.6.4`, `serde_json 1.0.151` in both manifests, `regex 1.13.1`,
  `tempfile 3.27.0` across all four sites). The new `minimal-versions` job
  resolves every direct dependency down to its declared floor and type-checks
  the workspace there, so a floor that stops being true fails CI instead of
  failing a downstream consumer. It is this repo's only nightly consumer.
  `base64` was audited and left at `0.22`: that floor resolves to 0.22.0 and
  compiles, and the second `base64 0.23.1` in the lock is transitive-only, via
  `rmcp`/`mcp-methods` behind `kglite-mcp-server`.
- **AGC control-edge property frames omit all-empty columns**, trimming per-edge
  load cost on sparse graphs (no output change). `JUMPS_TO`/`BRANCHES_TO` frames
  emitted `raw_targets`, `offsets`, `via` and `address_lines` unconditionally,
  even when no edge in the frame carried them; they now use the same
  `if edges.iter().any(...)` conditional-column pattern the `CALLS` and
  `REFERENCES` frames already use. An all-`None` column stores nothing
  engine-side, so the graph is byte-identical — the frozen parity goldens,
  including the `agc_basic` corpus, are unchanged.
- **The freshness fingerprint now hashes only ingestible inputs, in parallel.**
  It used to hash nearly every file under the source root — on a KGLite checkout
  that is 232 MB across 3250 files, of which 169 MB is `.so`/`.dylib`/`.jar`
  that can never reach the graph — and it did so on `build`, on `status`, and on
  every `query` (via the freshness check). The scope is now derived from what
  the builder actually ingests: the parser registry's extension map, the docs
  extensions (`.md`/`.mdx`/`.rst`, matched case-insensitively), and the
  manifests that shape the graph (`pyproject.toml`, `Cargo.toml`,
  `package.json`, `tsconfig.json`); directory pruning calls the builder's own
  `is_ignored_dir_name` instead of a narrower copied list, so `_build`, `venv`,
  `env` and `site-packages` are no longer hashed. **Scoping is by ingestibility,
  never by gitignore** — the docs pass ingests gitignored markdown, so a
  gitignore-scoped fingerprint would report an edited repo as fresh. The
  surviving files are hashed across up to 8 threads and folded together in
  sorted path order, so the value stays deterministic. Measured on a KGLite
  checkout (`80a0df52`, live working tree): fingerprint wall **0.299s → 0.013s**
  warm (min of 5), **1.440s → 0.105s** on the first run of the process; the
  hashed set drops from 232 MB / 3250 files to 17 MB / 1150 files. This also
  **fixes a live false-stale**: rebuilding a shared library no longer flips
  `status` to stale. **The fingerprint value itself changes**, so every existing
  `.kgl.meta.json` reads stale exactly once after upgrading; one rebuild
  restores it. Graph bytes are untouched — all parity goldens are unchanged.
- **Parsing now dispatches every file through ONE parallel worklist** instead of
  a separate parallel batch per source root per language. The old shape ended
  each batch in a join that idled the pool whenever a batch was smaller than the
  core count or held one slow file — a cost paid once per language per root, and
  worst on the multi-root polyglot repos that have the most batches. Measured
  parse-phase wall: **-12.0%** on KGLite (`0.320s → 0.281s`, git-archive of
  `80a0df52`) and **-18.9%** on mistral.rs (`0.294s → 0.239s`, archive of
  `1d0884d`); whole-build wall **-9.6%** / **-13.3%** on the same two repos
  (`codingest_bench` corpora `0882abb4c2b1` / `8c44399b4047`), means over 6
  alternating samples per side. **Graphs are byte-identical**: results are
  merged in the unchanged (root, language, path) order, both repos produced
  identical node/edge counts before and after, and all 15 parity goldens are
  untouched. Cross-file post-passes (AGC's `role_hint` promotion and
  ALIAS_OF / POINTS_TO synthesis) keep their per-(root, language) scope through
  a new `LanguageParser::finalize` hook, so their resolution still cannot reach
  across source roots.
- **`Route` nodes now represent registrations, not URLs.** The node id includes
  the declaring file (`{framework}::{method}::{path}::{file_path}`), so the same
  path registered from two files is two distinct nodes, each with truthful
  `file_path` and `line_number`. Previously the id was
  `{framework}::{method}::{path}`, so every methodless `@app.route('/')` in a
  repo collapsed into ONE node whose source location described whichever file
  the sorted walk reached first — wrong for every other registration. Within a
  single file the identity is unchanged: two registrations of the same
  method+path there remain one node with parallel `HANDLES` edges. Cross-language
  `CALLS_SERVICE` linking matches on the `path` property and is unaffected,
  except that a path with N registrations now links to all N. One golden digest
  moved deliberately (`cross_ts_py`, id shape only — node and edge counts
  unchanged).
- **Call-resolution language groups are now declared per language in the parser
  registry instead of inferred from qualified-name separators.** The
  `lang_group` CALLS tier previously guessed a symbol's language family by
  sniffing its qualified name — `::` meant Rust/C++, `/` meant Go/TS/JS, and
  anything else meant Python/Java — which read the wrong answer whenever a
  qname's punctuation did not match its language. **HTML-embedded JavaScript is
  the case that shows it:** a `<script>` body is rescoped to
  `index.html:script_N.<name>`, all dots, so an ambiguous call inside it
  narrowed to a *Python* candidate over the JavaScript one. Each
  `LanguageSpec` now carries a `group`, resolved through the file a symbol is
  defined in, so HTML and CSS group with Go/TS/JS and C groups with Rust/C++
  (it previously sniffed into the Go/TS/JS group on its `/` separator). A
  qualified name with no file mapping still falls back to the old sniff, so
  unmapped names do not change behavior. No pre-existing golden digest moved —
  no corpus could reach this tier — and the new `html_js_lang_group` corpus
  closes that gap.
- **The KGLite floor moves to 0.15.8 across Cargo and Python.** The embedded
  MCP server picks up KGLite's mcp-methods 0.4.4 / rmcp 3.1.1 integration
  (0.15.7), workspace-graph producers now receive deduplicated changed-path
  hints alongside full-build requests, and a query-plan cache fix stops a write
  loop evicting unrelated graphs' cached read plans. Graph APIs, builder
  output, property encoding, and `.kgl` persistence are unchanged.
- **CI installs the engine it declares.** The Python job pinned
  `kglite==0.15.5` while the wheel required `>=0.15.6`, so the job's own
  `pip check` step was validating a dependency set the release never ships.
  The pin now tracks the floor.

## [0.1.7] - 2026-08-06

### Changed
- **The KGLite engine floor is now 0.15.6 across Cargo and Python.** The Rust
  engine, embedded MCP server, Python runtime requirement, and documentation
  move together. This picks up corrected mixed-selection vector search,
  community modularity scoring, sampled-centrality validation, persisted HNSW
  validation, and the 0.15.6 graph-algorithm and vector-search improvements.
  Release tests now enforce that the Cargo engine/MCP floors and Python runtime
  floor remain aligned, and Python acceptance tests prove they exercise an
  installed KGLite version inside the wheel's declared range.

## [0.1.6] - 2026-08-01

### Added
- **Closure-scoped TS/JS definitions are now graph nodes.** The parse walk
  only ever looked at the direct children of a file's program root, so
  everything declared inside a function body, an arrow body, a generator body
  or a TS `namespace` was invisible — on one Effect-TS codebase that is ~37 %
  of the core package's named callables, and `packages/opencode/src/mcp/`
  `index.ts` (1 004 lines) had exactly **3** `Function` nodes. It now has 36.
  A definition becomes a node when it is a named binding (`function` /
  `function*` declaration, `const|let|var x = <fn literal>`, or a
  narrowly-factory-wrapped binding) **and every enclosing scope on its chain
  is itself named**. A helper declared inside an anonymous callback
  (`useEffect(() => { const helper = … })`) is deliberately *not* a node: it
  has no addressable name, and admitting that class is what takes node growth
  past its budget. Anonymous callbacks remain non-nodes as before.
  Two new `Function` properties describe the nesting, and both are absent
  (rather than empty or zero) at top level, so graphs without closure-scoped
  definitions keep their exact property set: **`parent_scope`**, the qualified
  name of the nearest named enclosing binding, and **`nesting_depth`**, where
  1 or more means closure-scoped. Qualified names are scope-chained —
  `packages/opencode/src/mcp.layer.connectRemote` — so two `get`s in two
  closures of one module no longer collide. There is no new edge type: the
  enclosing scope is often a `Constant` or an anonymous literal, so a
  `Function`→`Function` edge would misrepresent node types; query containment
  with `WHERE f.parent_scope = '…'`.
  **Closure-scoped definitions resolve `CALLS` within their own file only.**
  A nested definition is lexically callable inside its enclosing scope unless
  it escapes, so it never joins the global name index. Without that rule the
  ~2 270 new nested names on the same codebase would have taken
  multi-candidate call names from 664 to 1 562 and turned 293 previously
  unambiguous names ambiguous. Top-level bindings, including `namespace`
  members, participate globally exactly as before.
- **Nested Python `def`s are now graph nodes.** The Python walk only ever
  looked at the direct children of a module, so a `def` inside a function body
  was invisible — decorator factories, closure factories, the
  `def wrapper(...)` at the heart of every `functools.wraps` decorator, and
  every view function of a Flask **application factory**. On `pallets/flask`
  the routes the app actually serves were among the missing; on `django/django`
  it is 391 definitions. A nested `def` becomes a `Function` node carrying the
  same **`parent_scope`** and **`nesting_depth`** properties as the TS/JS
  closure walk, with a scope-chained qualified name
  (`pkg.mod.retrying.decorate.wrapper`), and it resolves `CALLS` — and now also
  `REFERENCES_FN` and `DECORATES` — **within its own file only**, so a
  `wrapper` in one module can never be mistaken for a `wrapper` in another.
  Python's scoping rules are followed exactly rather than the TS model being
  copied: `if` / `for` / `while` / `with` / `try` / `match` blocks are **not**
  scopes in Python, so they add no name segment and no nesting level — a `def`
  inside an `if` inside `outer` is `outer.<name>` at depth 1 — while a `lambda`
  and the comprehension forms name no scope at all and keep their calls with
  the enclosing `def`. Because blocks are transparent, the
  `if`/`else` and `try`/`except` conditional-definition idiom routinely
  produces two identical qualified names in one scope; the second and
  subsequent get a `#{line}` suffix, the first is left alone. A class defined
  inside a function contributes a name segment
  (`outer.Inner.method`) without becoming a node of its own. Node growth is
  +5.5 % on flask and +1.8 % on django.
- **`.mdx` documentation is ingested by the docs pass.** `--include-docs`
  accepted `.md` and `.rst` only, so an Astro / Starlight / Docusaurus site —
  where the entire documentation set is `.mdx` — produced nothing. On one
  repository that was 627 files and 0 `:Doc` nodes; it is now 627 nodes, taking
  the repository from 117 to 744. `.mdx` is Markdown with embedded JSX/ESM and
  goes through the Markdown path unchanged, so frontmatter, heading outlines,
  backtick symbol `MENTIONS` and `[](…)` links all work exactly as they do for
  `.md`, and an `.mdx` is now a valid link *target* as well: a `.md` linking to
  `[x](./guide.mdx)` gets a `DOCUMENTS` edge. Embedded JSX and ESM are inert to
  every extractor — measured across those 627 files, 54 of 21 786 mention
  candidates (0.25 %) sat on an `import` / `export` / JSX line and **none**
  resolved to a symbol, so no edge in the graph comes from one.
  `.txt` is deliberately **not** ingested: it carries no frontmatter, heading
  or link syntax for any extractor to read, and the extension is
  indiscriminate — `requirements.txt`, `CMakeLists.txt`, licence files and test
  fixtures would all become `:Doc` nodes. Genuine `.txt` prose (agent prompt
  files, say) needs a manifest-driven opt-in rather than a widened extension
  list; that is recorded as deferred, not forgotten.
- **Top-level factory-wrapped TS/JS bindings are `Function` nodes.**
  `export const readFile = Effect.fn("Bom.readFile")(function* (…) { … })`
  bound a function but had a `call_expression` value, so it became a
  `Constant` with a 100-character `value_preview` and disappeared from the
  graph as a callable — on one Effect-TS codebase, 147 top-level exports.
  Such a binding now becomes a single `Function` node (the `Constant` it used
  to produce is gone, not duplicated) carrying a new `wrapped_by` property
  naming the factory (`Effect.fn`, `Layer.effect`, `memoize`). The property is
  only present on graphs that have at least one wrapped binding.
  The unwrap is deliberately narrow, because `const names = users.map(u =>
  u.name)` binds an array and not a function: the value's call chain must
  contain **exactly one** function literal, that literal must be a generator
  *or* its call must be curried (`f(…)(fn)`), and that call's callee must not
  be a method on a value receiver — a bare identifier (`memoize`) or a member
  on a Capitalized identifier (`Effect.fn`) qualifies, `arr.map`,
  `results.filter` and `tp.split(',').map` do not. Bindings inside a function
  or closure body are still not node-ified; that is a separate change.
- **`codingest_stats --include-docs`.** The accuracy harness built with the
  docs pass hard-coded off, so `:Doc`, `:MENTIONS` and `:DOCUMENTS` could never
  appear in a recorded measurement and no docs-pass regression could fail a
  gate. The flag opts the pass in and is off by default, since docs-off is the
  configuration the existing result history was taken at; the emitted JSON now
  states both `include_tests` and `include_docs`, and an unrecognised argument
  is a usage error instead of being silently ignored.

### Fixed
- **`function*` and `const x = function () {}` were invisible to the TS/JS
  parser.** tree-sitter-typescript emits `function_expression`,
  `generator_function` and `generator_function_declaration`, but the parser
  matched a node kind named `function` that the grammar never produces. Three
  consequences, all now fixed: `const x = function () {}` and
  `const x = function* () {}` became `Constant` nodes instead of functions,
  and a top-level `function* g() {}` — exported or not — produced **no node at
  all**. Generators are load-bearing in Effect-TS and redux-saga codebases.
- **Calls inside a nested named binding were dropped, not mis-attributed.**
  Call extraction skipped named arrow and function bindings on the theory that
  they were "node-ified elsewhere" — true only at the top level. A
  `const handler = () => { foo() }` *inside* a function body was skipped by
  the extractor **and** never node-ified, so `foo()` left no trace in the
  graph at all. Those calls now attach to the binding that contains them, and
  every call site is attributed to exactly one `Function` — the nearest
  enclosing node-ified scope — so nothing is counted twice either.
- **The same dropped-calls defect in Python.** Call extraction skipped nested
  `def`s and `@decorated` definitions with the same false justification, and
  nothing node-ified them either, so the body of every decorator's `wrapper`
  and every closure factory's inner function contributed nothing to the graph.
  Those calls now attach to the definition that contains them.
- **`REFERENCES_FN` and `DECORATES` could point across files into a
  closure-scoped definition.** Both resolve a bare identifier to a function
  that is *globally unique* by short name, and a nested definition entered
  that index — so a `wrapper`, `inner` or `decorator` declared inside one
  function could become the target of a reference or a decorator in an
  unrelated file, which no name in that file can actually refer to. Both now
  apply the same same-file-only rule `CALLS` already did, and a nested name no
  longer shadows or disambiguates an identically named top-level export for
  callers elsewhere.
- **Django routes could lose their `HANDLES` edge — or gain a wrong one — to a
  closure-scoped definition.** The `urlpatterns` view resolver is the fourth
  bare-name index over the function population, and it was the one left
  ungated. A nested `def` sharing a short name with a real view anywhere in the
  repository made that view look ambiguous, and the resolver skips rather than
  guesses, so `path('p/', views.detail)` silently emitted **no** edge whenever
  any `detail` existed inside another function. In the other direction a
  globally unique nested name — `wrapper` being the archetype — became the
  handler of a route declared in a `urls.py` that cannot name it. Both are
  gone: closure-scoped definitions are offered only to a `urlpatterns` in their
  own file, the same rule `CALLS`, `REFERENCES_FN` and `DECORATES` follow.
- **An upper-cased `README.MD` kept its extension in its `:Doc` id.** The docs
  walk has always accepted markup extensions case-insensitively, but the id was
  derived by stripping a literal lowercase `.md`, so such a file became the
  node `README.MD` while every sibling became `README`-shaped. Because doc→doc
  links are matched against extension-stripped ids, that node could never be
  linked to. Ids are now derived uniformly for every accepted markup
  extension.

## [0.1.5] - 2026-08-01

### Added
- **`CALLS` edges carry how they were resolved.** Three new properties:
  `resolution` names the tier that pinned the edge (`exact_qualified`,
  `receiver`, `inherited`, `same_owner`, `namespace_import`, `same_file`,
  `unique_name`, `lang_group`, `global_fallback`); `candidates` is how many
  targets survived the tiers, so `> 1` marks an edge as one of several guesses
  for the same call site; `import_backed` says whether the caller's file is —
  or imports — the callee's. Until now a query could not tell a receiver-pinned
  edge from a global-name guess, which is what makes "who calls X" unusable on
  a large corpus: `MATCH ()-[r:CALLS]->(f) WHERE r.import_backed AND
  r.candidates = 1` is now expressible. When several call sites between the
  same pair disagree, the edge keeps the best-precision tier and the smallest
  candidate count, by a fixed documented ranking. AGC control-transfer edges,
  which do not go through the tiers, leave all three null.
  `import_backed` is a **one-hop** check: a caller that reaches the callee
  through a barrel that re-exports it reads as `false` even though the call is
  real, so treat `false` as *unconfirmed* rather than *refuted*. It is a filter,
  not a deletion criterion — measured against a hand-labeled truth set on a
  3,293-file monorepo, filtering on `import_backed AND candidates = 1` removes
  96.9% of the false edges, and the edges it would wrongly remove are exactly
  the barrel-re-export ones. No edges are dropped from the graph.
  **Python is the exception: do not filter Python graphs on `import_backed`.**
  Absolute Python imports only resolve in the rare layout where the top-level
  package name happens to equal the repository's own directory name, so a
  Python project produces no `IMPORTS` edges at all and `import_backed` is
  `false` for *every* cross-file Python call (same-file calls are unaffected).
  See [the CALLS-property reference](docs/cli.md#interpreting-calls-edges).

### Fixed
- **TypeScript/JavaScript imports now resolve.** Relative specifiers were
  discarded at parse time (`import { x } from "./util"` was dropped before the
  builder ever saw it), and TS was not wired into any path-aware resolution, so
  a TS codebase produced essentially no `IMPORTS` edges — on a 3,293-file
  monorepo, 4 of ~13,000 specifiers. Relative specifiers are now recorded
  verbatim and resolved against the project's module set: the specifier is
  normalized against the importing file's directory, its `.ts`/`.tsx`/`.js`/
  `.jsx`/`.mjs`/`.cjs`/`.mts`/`.cts` suffix stripped, and matched first as-is
  and then with a trailing `/index` segment removed (because `a/b/index.ts`
  *is* module `a/b`). `export … from "…"` is captured too, so barrel files —
  which in a TS monorepo are the main dependency conduit and contain nothing
  but re-exports — finally contribute edges. Resolution never guesses: a
  candidate becomes an edge only when it names a module the project actually
  defines, so no edge can point at a file that does not exist. `require()` and
  dynamic `import()` remain out of scope.
- **TypeScript `paths` aliases and workspace-package specifiers resolve too.**
  `import "@/mcp/catalog"` and `import "@opencode-ai/core/foo"` are the other
  two shapes a monorepo uses, and neither is resolvable from the importing
  file's path alone. The builder now reads every `tsconfig.json`'s
  `compilerOptions.baseUrl`/`paths` and every `package.json`'s `name` once per
  build. Alias lookup is **nearest-ancestor**, not root-only, because real
  repos put `paths` in per-package configs (a root config that merely
  `extends` a base has no `paths` at all, so a root-only reader resolves
  nothing). Pattern selection is exact-match first, then longest literal
  prefix, ties broken lexicographically; every listed target is tried in
  config order. Workspace specifiers take the longest package-name prefix
  (respecting the `/` boundary, so `@scope/corely` never matches
  `@scope/core`) and probe `<pkgdir>/<rest>` then `<pkgdir>/src/<rest>`.
  `tsconfig.json` is read as JSONC — comments and trailing commas are
  stripped with a string-literal-aware scanner, so a `//` inside a string
  stays data. Deliberate limitations: `extends` chains are not followed, and
  `exports`/`imports` maps are not interpreted (the `src/` probe covers the
  `"./*": "./src/*.ts"` convention). On a 3,293-file monorepo this takes
  File→File `IMPORTS` from 73 to 8,039, for +0.014 s of discovery.

### Added
- **`codingest_stats --edge-breakdown` and `--dump-calls`** — two read-only
  reporting flags on the accuracy harness. `--edge-breakdown` appends a
  per-connection-type edge histogram to the JSON, splitting `IMPORTS` by
  endpoint node type (`IMPORTS(File->File)` vs `IMPORTS(File->Module)`) because
  the two answer different questions and only the File→File half is the
  dependency conduit. `--dump-calls name1,name2,…` emits every `CALLS` edge
  whose callee short-name is in the list as
  `{caller, callee, caller_file, callee_file, call_lines}`, sorted by
  (callee, caller) — the substrate for auditing call-resolution precision
  against source. Neither flag touches the builder or the graph.
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

  **Freshness is always checked, and warns rather than refuses.** Before
  executing, `query` runs the same sidecar check `codingest status` does and
  prints `warning: …` to stderr when the graph is stale, has no sidecar, or
  cannot be verified at all — the last being what a `.kgl` copied to another
  machine hits, where the recorded source tree is unreadable. Rows are still
  returned; refusing by default would break the copied-artifact and CI-cache
  cases the CLI exists for. `--require-fresh` upgrades any non-fresh or
  unverifiable outcome to a hard refusal.

  **Exit codes are a CI contract:** `0` success, `2` usage errors, `3` a
  `--require-fresh` refusal, `1` every other operational failure (missing
  artifact, bad Cypher, timeout, I/O). Caveat: the `pip install codingest`
  console script maps every error to `PyRuntimeError`, so a stale refusal
  surfaces there as exit `1`; use the cargo binary where the distinction
  matters.
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
