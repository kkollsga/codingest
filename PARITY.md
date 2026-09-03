# Parity Verification: in-tree `kglite::code_tree` vs standalone `codingest`

Date: 2026-07-15. Compared the in-tree module (`/Volumes/EksternalHome/Koding/Rust/KGLite/crates/kglite/src/code_tree/`)
against the standalone crate (`crates/codingest/`). Both link the same `kglite`
engine crate, so graphs from either builder are read through identical
`kglite::api` types (`DirGraph` / `NodeData` / `EdgeData` / `Value`).

**Verdict: full feature parity, full performance parity. Zero graph discrepancies found. No fixes required.**

## Release 0.2.16 — 2026-09-03: 30 corpora, all green across the kglite 0.16.22 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency` both ok), and green again in the full
`make gate VENV=.venv` (ALL 9 STEPS PASSED) at the pre-bump commit. The engine
floor moved kglite 0.16.20 → 0.16.22 — two upstream releases at once, since
codingest never took 0.16.21 — and **no builder source changed this release**:
`git diff v0.2.15..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move.** None of the
persistence entries the builder calls (`prepare_kgl_write`, `write_kgl`,
`save_graph`) moved upstream, which is why every digest holds.

**Neither release reaches the builder.** The two struct-literal breaks upstream
declared are unreachable here: the blueprint spec structs (`Blueprint`,
`Settings`, `NodeSpec`, `FkEdge`, `JunctionEdge`) gained fields and
`JunctionEdge::target` became `Vec<String>`, but codingest constructs no
blueprint and calls neither `from_blueprint` nor `from_records`, so the
now-honoured `on_missing_endpoint` spec key is unreachable too; `ServerExtensions`
gained a field and `read_only()`, and codingest-mcp composes through
`ServerExtensions::default().with_workspace_graph(hooks)`, a builder chain.
What does reach users is the embedded MCP server refusing to boot on a
declared-but-missing `skills:` pack (it used to boot with *every* skill gone
while `--selftest` printed PASSED), `save_graph(force: true)` requiring the
write opt-in, and a fired query deadline raising `CypherTimeoutError` instead of
`CypherExecutionError` — which `codingest query --timeout` surfaces through
kglite's own error class. The Python acceptance suite (31 tests) ran against the
kglite 0.16.22 wheel reading Rust-0.16.22-written bytes.

**The perf anchor is VOID this release, and the instrument is proven to be the
mover.** `BENCHMARKS.md` was not refreshed (no perf-sensitive path changed — the
builder diff is empty), and the release-gate anchor against the 0.2.12 baseline
returned VOID in **both** modes: the control `top20_by_branch_count` read
+122 % to +141 % per row (0.0009 → 0.0020 ms/row). It was re-measured, and then
measured a second independent way, per the control doctrine:

- The control is **bimodal** on this machine — every capture lands at either
  ~0.018 ms or ~0.041 ms, never between — and the 0.2.12 baseline captured the
  low mode. Twelve consecutive captures landed in the high mode.
- **The previous engine VOIDs too.** A `codingest_bench` built against kglite
  **0.16.20** — the exact engine that produced the green 0.2.15 capture — VOIDed
  2 of 3 times against the same baseline, on the same corpus, minutes apart.
- **Interleaved A/B, 12 samples each, alternating under identical load:**
  0.16.20 median 0.041 ms (low mode 1/12, min 0.018), 0.16.22 median 0.040 ms
  (low mode 2/12, min 0.017). The two engines are indistinguishable; both reach
  both modes.

The machine could not be settled — eight unrelated processes held ~98 % CPU each
throughout (load average 11–24), and they were not this release's to stop. The
verdict is therefore recorded as VOID-by-instrument rather than re-read as a
regression, exactly as the tool instructs, and the 0.2.16 baseline capture is
deferred to a settled machine. No row was read past the control, on purpose.

## Release 0.2.15 — 2026-09-02: 30 corpora, all green across the kglite 0.16.20 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency` both ok). The engine floor moved kglite
0.16.19 → 0.16.20 and **no builder source changed this release**:
`git diff v0.2.14..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move**, corroborated
independently by the perf anchor: `nodes` and `edges` compare at
+0.00 % / +0.00 % (docs-on) and +0.00 % / +0.00 %
(docs-off) against the 0.2.12 baseline, control steady.

**0.16.20 changes what the embedded MCP server says, not what it computes.**
The `<active_graph>` header and the `— active graph:` footer now report
`load="N"` / `· load N ·` where they said `generation`, and gain a preceding
`file_saved` field carrying the served artifact's publish time; `reload_graph`
answers "Load N on this server". A write-enabled server's `save_graph` with
nothing unsaved is a no-op instead of republishing identical bytes (`force:
true` rewrites on purpose), `extensions.writable: true` matches `--writable`,
and `builtins.save_graph: true` alone registers only `save_graph`. codingest
embeds that server unchanged via `codingest-mcp` and parses none of its output
(`git grep -n 'generation="'` and `git grep -n '· generation'` are both empty
here), so no codingest source changed this release. On the core side the move
is additive and unused: `GraphFileIdentity::modified()` is new, a released
`GraphWriterLease` appends `released=<rfc3339>` to `<path>.lock-owner`, and
codingest names neither type. None of the persistence entries the builder calls
(`prepare_kgl_write`, `write_kgl`, `save_graph`) moved — which is why every
digest holds. The Python acceptance suite (31 tests) ran against the kglite
0.16.20 wheel reading Rust-0.16.20-written bytes. `BENCHMARKS.md` was not
refreshed: no perf-sensitive path changed (the builder diff is empty), and the
release-gate anchor carries the perf evidence.

## Release 0.2.14 — 2026-09-01: 30 corpora, all green across the kglite 0.16.19 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency` both ok). The engine floor moved kglite
0.16.18 → 0.16.19 and **no builder source changed this release**:
`git diff v0.2.13..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move**, corroborated
independently by the perf anchor: `nodes` and `edges` compare at
+0.00 % / +0.00 % (docs-on) and +0.00 % / +0.00 %
(docs-off) against the 0.2.11 baseline, control steady.

**0.16.19 is the lazy-writer-lease + automatic-refresh release.** A
write-enabled `kglite-mcp-server` takes the served `.kgl`'s writer lease at
its first unsaved change rather than at boot, a `--graph` server re-reads
the file when its on-disk identity changes, `save_graph` refuses a lost
update, and `extensions.graph_watch` is retired. codingest embeds that server
unchanged via `codingest-mcp`, so the behaviour ships here, but none of the
persistence entries the builder calls (`prepare_kgl_write`, `write_kgl`,
`save_graph`) moved — which is why every digest holds. The one codingest
source change is in `codingest-cli`, not the builder: the working-tree
`set_instructions` write now goes through 0.16.19's
`kglite::api::make_dir_graph_mut_preserving_lineage` (a configuration write,
the accessor's published purpose); on a uniquely owned freshly built graph it
is the same bytes as the `Arc::make_mut` it replaced, and the CLI exit-code
suite proves the artifact unchanged. The Python acceptance suite (31 tests)
ran against the kglite 0.16.19 wheel reading Rust-0.16.19-written bytes.
`BENCHMARKS.md` was not refreshed: no perf-sensitive path changed (the
builder diff is empty), and the release-gate anchor carries the perf
evidence.

## Release 0.2.13 — 2026-08-31: 30 corpora, all green across the kglite 0.16.18 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency` both ok). The engine floor moved kglite
0.16.17 → 0.16.18 and **no builder source changed this release**:
`git diff v0.2.12..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move**, corroborated
independently by the perf anchor: `nodes` and `edges` compare at +0.00 %
against the 0.2.10 baseline in both docs modes, control steady.

**0.16.18 contains zero engine changes.** Every fix lives in
`kglite-mcp-server` boot/extension handling (OS-assigned CSV port restored,
failed peripherals degrade instead of killing boot, mode-dependent
`bundled: repo_management` overrides tolerated; mcp-methods 0.4.6 → 0.4.7).
codingest embeds that server via `codingest-mcp`, so the fixes ship here, but
no `kglite::api`, Cypher, or `.kgl` storage surface moved — which is why every
digest holds. The Python acceptance suite (31 tests) ran against the kglite
0.16.18 wheel reading Rust-0.16.18-written bytes. `BENCHMARKS.md` was not
refreshed: no perf-sensitive path changed (the builder diff is empty), and
the release-gate anchor carries the perf evidence.

## Release 0.2.12 — 2026-08-31: 30 corpora, all green across the kglite 0.16.17 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`, `kgl_bytes_are_stable_across_builds` and
`reloaded_graph_renders_identically` alongside). The engine floor moved kglite
0.16.15 → 0.16.17 (over 0.16.16) and **no builder source changed this
release**: `git diff v0.2.11..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move**, corroborated
independently by the perf anchor: `nodes` and `edges` compare at +0.00 %
against the 0.2.9 baseline in both docs modes.

**0.16.16 lands on a flag codingest documents.** The deadline set by
`codingest query --timeout` was previously observed only inside the pattern
matcher; the MATCH row loops downstream of it ran to completion however long
past the deadline, and variable-length expansion ignored cooperative
cancellation. Both now poll. The timeout *contract* is unchanged (a fired
deadline errors, no partial rows), so error handling holds — and nothing in
the release touches the build path, which is why every digest holds. Its one
removal, `QueryDiagnostics::timed_out`, had no writer anywhere in the engine
and is a symbol codingest names nowhere.

**0.16.17 is the compile-footprint release** — kglite pulls `geo` without its
default features (no API or Cypher change; codingest has no direct `geo`
dependency), cut the same day as codingest's footprint request after
unbounded target dirs filled the shared build-cache disk to zero. The same
coordination changed this repo's gate *invocations* (parity and the
bench/soak bins run with `--workspace` package selection; `make gate` builds
`--all-targets`; dependency debuginfo capped at `line-tables-only`) — build
plumbing only, verified by this release's full gate: the graph, the parity
digests and the query results are computed identically. The Python acceptance
suite (31 tests) ran against the kglite 0.16.17 wheel reading
Rust-0.16.17-written bytes.

## Release 0.2.11 — 2026-08-30: 30 corpora, all green across the kglite 0.16.15 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`, `kgl_bytes_are_stable_across_builds` and
`reloaded_graph_renders_identically` alongside). The engine floor moved kglite
0.16.13 → 0.16.15 (over 0.16.14) and **no builder source changed this
release**: `git diff v0.2.10..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move**, corroborated
independently by the perf anchor: `nodes` and `edges` compare at +0.00 %
against the 0.2.8 baseline in both docs modes.

**This move is not a pure lockstep refresh — two 0.16.14 fixes land on bytes
codingest ships.** Two saves of one graph write identical `.kgl` bytes again
(four persisted lists — the type-connectivity triples and the
property/composite/range index-key snapshots — were written in hash-map
iteration order). That is *our* output files gaining byte-determinism, and it
could not have moved the goldens even in principle: the goldens digest the
**in-memory canonical rendering** (`canonical_graph_string`), never `.kgl`
bytes, and no format changed — the same reader loads files written either way.
And a reloaded `.kgl` no longer reports every relationship type as having
**zero edges**: the load derived the authoritative type-connectivity cache
from fabricated 0-count triples, so `describe()` and planner selectivity over
a *loaded* codingest graph saw zeros over a graph full of edges; the fix also
distrusts files already written with the zeros, repairing existing graphs on
read. This is a load-path fix — the build path never had the defect, which is
why every digest holds.

0.16.14's **breaking** `max_rows` → `max_work_units` rename (no alias) does
not reach us: every codingest `ExecuteOptions` is built via
`ExecuteOptions::eager(&params)`, and the tree spells `max_rows` nowhere.
0.16.15 (LoadOptions, `estimate_load_memory` / `max_load_mb`, the `row_limit`
cap, spill-dir and loader-error fixes) is additive at every surface codingest
names — `load_file` is unchanged and *is* the default `LoadOptions`, and
`defer_index_rebuild` would defer nothing on our graphs, since codingest
creates no index. The Python acceptance suite (31 tests) ran
against the kglite 0.16.15 wheel reading Rust-0.16.15-written bytes.

## Release 0.2.10 — 2026-08-27: 30 corpora, all green across the kglite 0.16.13 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`, `kgl_bytes_are_stable_across_builds` and
`reloaded_graph_renders_identically` alongside). The engine floor moved kglite
0.16.12 → 0.16.13 as a **lockstep refresh** and **no builder source changed
this release**: `git diff v0.2.9..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move**, corroborated
independently by the perf anchor: `nodes` and `edges` compare at +0.00 % in
both docs modes.

**The unreachability argument is different this time, and the difference
matters.** 0.16.10–0.16.12 could not reach us because they add *opt-in
declared* surface (the ontology layer, the BM25 text index) that codingest
declares nothing into. 0.16.13's ontology follow-ups are unreachable the same
way — but its engine fixes are not that shape. Each corrects an **index**
answering wrongly: an index on `name` shadowing the title fallback and silently
dropping rows from `{name: …}` lookups; indexed equality for an absent value
scanning the type; a `WHERE`-equality index pre-filter pruning rows of the
wrong type; a fluent `where()` over a mixed-type node set taking the first
indexed type's hits as the whole answer. All are unreachable here for exactly
one reason: **codingest creates no index at all** — zero hits across `crates/`,
`tests/` and the docs recipes for `create_index` / `create_range_index` /
`create_composite_index` / `build_text_index` / `build_vector_index` /
`CREATE INDEX`.

That is *unreachable*, **not** *unused*, and the distinction is recorded here
because it expires. The untyped `WHERE n.is_external = false` shape those fixes
repair is one codingest documents and ships; the day anyone adds an index, this
paragraph stops being true and the fixes become load-bearing. The release's
Rust API note does not reach us either — `kglite::api::RelationshipDecl` gained
public `exempt` / `ancestry` fields, and codingest constructs no
`RelationshipDecl` (zero struct literals). The Python acceptance suite ran
against the kglite 0.16.13 wheel reading Rust-0.16.13-written bytes.

## Release 0.2.9 — 2026-08-26: 30 corpora, all green across the kglite 0.16.12 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`, `kgl_bytes_are_stable_across_builds` and
`reloaded_graph_renders_identically` alongside). The engine floor moved kglite
0.16.9 → 0.16.12 as a **lockstep refresh** and **no builder source changed this
release**: `git diff v0.2.8..HEAD -- crates/codingest/src` is empty (the one
source edit is codingest-py's import-failure message, which named a floor six
patches stale).

**Every golden digest is byte-identical across the move**, and the perf anchor
corroborates it independently: `nodes` and `edges` compare at +0.00 % against
the 0.2.6 baseline in both docs modes, so the engine move changed nothing about
the graph the builder produces. The three upstream releases are opt-in surface
codingest declares nothing into — 0.16.12 and 0.16.11 are the ontology
declaration layer (nothing here calls `define_ontology`, so there is no
declaration to audit and no `<ontology>` section to render), and 0.16.10 adds
the BM25 text index whose `.kgl` section is a rebuildable cache that a graph
declaring no text index never writes. 0.16.11's one broad fix — `vacuum()`
corrupting secondary labels — cannot reach a builder that adds no secondary
label and never vacuums (grep: zero hits for `add_label` / `secondary_label` /
`vacuum` / `set_parent_type` in `crates/`). The Python acceptance suite ran
against the kglite 0.16.12 wheel reading Rust-0.16.12-written bytes.

## Release 0.2.8 — 2026-08-24: 30 corpora, all green across the kglite 0.16.9 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`, `kgl_bytes_are_stable_across_builds` and
`reloaded_graph_renders_identically` alongside). The engine floor moved kglite
0.16.7 → 0.16.9 as a **lockstep refresh** and **no builder source changed this
release**: `git diff v0.2.7..HEAD -- crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move** — and 0.16.9 makes
that claim in both directions itself: its CRC change (`crc32fast` replacing the
software table in `.kgl` section verification) computes the same CRC-32/IEEE
values, so a file written on either side verifies on the other; upstream pins
this with `crc32_matches_known_vector` plus cross-wheel flip tests. 0.16.8's
correctness fixes are query-semantics and index/storage repairs that reach no
query codingest ships (its one `count(DISTINCT …)` counts a node variable, not
a relationship). The Python acceptance suite ran against the kglite 0.16.9
wheel reading Rust-0.16.9-written bytes — and inherits upstream's ~1.6x load
speedup for digest-carrying files without a byte of the written format moving.

## Release 0.2.7 — 2026-08-23: 30 corpora, all green across the kglite 0.16.7 engine move

Released state: unchanged corpus set (**30 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`, `kgl_bytes_are_stable_across_builds` and
`reloaded_graph_renders_identically` alongside). The engine floor moved kglite
0.16.6 → 0.16.7 as a **lockstep refresh** — the 0.16.5 pattern — and **no
builder source changed this release**: `git diff v0.2.6..HEAD --
crates/codingest/src` is empty.

**Every golden digest is byte-identical across the move.** 0.16.7's changes are
Python-facing error-typing (`as_dict` removal, collision-raising `degrees()` /
`embeddings()` / `to_networkx()`, `compare()` errors as `kglite.ArgumentError`)
plus an opt-in strict mode and an HNSW engagement fix for whole-graph
`vector_search`; grep confirms codingest calls none of the changed APIs, no
file-format change is reported upstream, and the frozen goldens plus the `.kgl`
byte-stability test turn that from a changelog claim into a verified one. The
Python acceptance suite ran against the kglite 0.16.7 wheel reading
Rust-0.16.7-written bytes (the lockstep the floor exists for).

## Release 0.2.6 — 2026-08-22: 30 corpora (21 → 30), all green on kglite 0.16.6; four intended golden moves, each with a recorded reason

Released state: **30 corpora**, all green in the release-mode gate
(`cargo test --workspace --release`; `golden_parity` + `rev_self_consistency`,
`kgl_bytes_are_stable_across_builds` and `reloaded_graph_renders_identically`
alongside). This release is the kglite 0.16.6 engine move **plus** a 13-fix
builder correctness program, so unlike 0.2.4/0.2.5 the goldens were expected to
move — and exactly four did, each regenerated in the same commit as its fix
with the reason recorded there: `julia_basic` (EXTENDS endpoint typing — the
phantom `_provisional` Class stub is gone), `dup_minified_assets` (CSS/HTML ids
gained the start column; the corpus exists to pin that collision),
`html_js_lang_group` (pure id-string identity from the same change, every count
unchanged), `docs_mdx` (one new DOCUMENTS edge from the case-fold fix, via a
fixture link added for it). Nine corpora were added: `dart_part_of`,
`rust_inline_mod`, `py_src_layout`, `cpp_extern_c`, `web_served_root`, and the
first-ever `go_interface`, `java_javadoc`, `csharp_using_alias`,
`php_group_use`, `swift_basic` — all captured additively (five of them pin
fixes whose pre-fix graphs were measured and recorded in the commit messages).

**The engine move itself cannot move goldens** — verified, not assumed: the
0.16.6 Rust API delta is purely additive for every symbol codingest names and
`add_nodes`/`add_connections` behavior is unchanged upstream. What 0.16.6 DOES
change is query *answers* on cyclic graphs (breaking trail-semantics fix). The
parity gate is blind to that by construction, so this release adds
`crates/codingest/tests/traversal_semantics.rs`: hand-derived reachable-caller
sets over `rust_import`'s real 2-cycle, red on 0.16.5 with the old
distance-semantics answer and green on 0.16.6 — verified in both directions.

The fixture corpus now builds **567 nodes / 880 edges** docs-on (was 405/616) —
the growth is the nine new corpora, and the bench anchor correctly REFUSEd the
cross-corpus comparison (fresh baseline captured for the new corpus).

## Release 0.2.5 — 2026-08-20: 21 corpora, all green across the kglite 0.16.5 engine move

Released state: unchanged corpus set (**21 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`, with `kgl_bytes_are_stable_across_builds` alongside
them). The engine floor moved kglite 0.16.2 → 0.16.5 and **no builder source
changed this release** — `git diff v0.2.4..HEAD -- crates/codingest/src` is
empty.

**Every golden digest is byte-identical across that move**, and here that is a
stronger statement than at 0.2.4: 0.16.5 carries a breaking Rust API change —
node, relationship and map properties became a `kglite::datatypes::PropMap`
instead of a `BTreeMap<String, Value>` — so the claim under test was that a
change to how properties are *held in memory* leaves both the property values
and the serialized bytes alone. It does. codingest is not exposed to the API
break itself (it names neither `NodeValue::properties` nor
`RelValue::properties`, and its one `Value::Map` site matches a wildcard), and
upstream reports no file-format change, postcard's map framing being identical
either way. The frozen goldens and the `.kgl` byte-stability test are what turn
both of those from a changelog claim into a verified one.

The fixture corpus builds the same **405 nodes / 616 edges** docs-on and
**398 / 596** docs-off on 0.16.2 and on 0.16.5.

## Release 0.2.4 — 2026-08-16: 21 corpora, all green across the kglite 0.16.2 engine move

Released state: unchanged corpus set (**21 corpora**), all green in the
release-mode gate (`cargo test --workspace --release`; `golden_parity` +
`rev_self_consistency`). The engine floor moved kglite 0.16.1 → 0.16.2, whose
one change reaching codingest is the `add_connections` edge-property **type
registry** fix — every edge property we write (`CALLS.call_count`,
`IMPORTS.import_count`, …) was recorded as `Unknown` and now carries its
observed type. **Every golden digest is byte-identical across that move.** That
is the expected result and worth stating precisely: the fix changes edge
property *type metadata*, which the canonical digest does not cover, and not
the property *values*, which it does. So a green `golden_parity` here confirms
the values did not move; it is not evidence about the schema, which was verified
separately by querying `CALL db.schema.relTypeProperties()` on graphs built by
each engine (`["Unknown"]` → `["Long"]`/`["Boolean"]`). No builder source
changed this release.

## Post-016 program, closing reconciliation — 2026-08-15: 21 corpora

Three golden events post-date the entry below and are recorded here per this
file's completeness charter: **py_routes_dup** regenerated (multi-line route
decorators — the dominant real FastAPI style — produced no Route node; a
multi-line registration joined the corpus and Route went 5→6), and the two
additive language captures **r_basic** and **julia_basic** (languages 16 and
17; each verified strictly additive by `git status` after `capture_goldens`).
This also corrects the entry below on two counts: the corpus set is now
**21**, and "the 14 pre-program goldens never moved" held only until
py_routes_dup's deliberate, reasoned move — every other pre-program golden
remained untouched through program end.

## Post-016 program — 2026-08-15: 19 corpora at entry time; six deliberate golden moves, each with its recorded reason

Corpus set grew 15 → 19 (`rust_import`, `py_import`, `cpp_include`,
`dart_import` — each added FIRST to pin the pre-fix broken output, so the fix
commit's golden diff is the record of exactly what it changed). Golden moves,
one per phase commit, reasons in the commit messages: rust_import (File→File
0→4, File→Module off the bare crate root), py_import (IMPORTS 4→16),
dart_import (IMPORTS 3→5, the a/x-vs-b/x collision split), cpp_include
(corpus pin only — an angle include naming a real project file, proving no
edge forms), cross_ts_py (Route 4→2: flask twins of fastapi registrations
removed). The 14 pre-program goldens never moved at any phase — isolation
verified by canonical dump at every integration, twice per agent-implemented
phase (agent's check + coordinator's independent check). kglite 0.16.0 → 0.16.1
moved no golden (verified before the program's own changes began).

## Release 0.2.1 — 2026-08-12: 15 corpora, all green across the kglite 0.15.13 engine move

Released state: unchanged corpus set (**15 corpora**), all green in the
release-mode gate. The engine floor moved kglite 0.15.11 → 0.15.13, which
includes a query-planner estimate fix that makes anchored traversals 5.7–327×
faster (see `CHANGELOG.md` `[0.2.1]` and
`BENCHMARKS.md`). **Every golden digest is byte-identical across that move —
verified, not assumed:** `golden_parity` (three builds per corpus),
`rev_self_consistency` and `kgl_bytes_are_stable_across_builds` all pass with no
golden regenerated. Independent evidence that it is a planning change and not a
result change: the bench harness returned identical row counts for all eleven
queries on both engine versions, on both a 8,691-node and a 1,238-node corpus.

## Release 0.2.0 — 2026-08-11: 15 corpora, all green

Released state: **15 corpora** (12 pre-program + `docs_ext_collide`,
`py_routes_dup`, `html_js_lang_group`, each captured additively), all green in
the release-mode gate (`golden_parity` three builds per corpus +
`rev_self_consistency` + `kgl_bytes_are_stable_across_builds`). Golden
movements this release, each regenerated in its own phase commit with a
recorded reason: `py_basic` + `py_nested_defs` (Python absolute imports now
resolve), `cross_ts_py` (route identity → registration model), the 13-of-14
bulk regen below (manifestless anchoring; `rust_xfile` frozen as the
additivity proof). The kglite 0.15.8 → 0.15.11 engine move and the
NodeView/`set_node_property` API migration left every digest byte-identical —
verified, not assumed.

## Inferred `:Project` for manifestless repositories — 2026-08-10 (branch `feat/backlog-2026-08`)

**The first bulk golden regeneration since the extraction: 13 of 14 digests
moved.** Every prior capture was additive by construction (a new corpus, with
the pre-existing digests byte-identical); the largest earlier movement of a
pre-existing digest was one file.

Only `pyproject.toml` and `Cargo.toml` were recognised as manifests, so every
other repository — all JS/TS, Go, Java and C++ trees, and any plain directory —
built with **no `:Project` node at all**: no `HAS_SOURCE` owner for its files,
and docs anchored to nothing structural. Manifestless repositories now get an
inferred project (name = the project root's directory name, languages
reconciled from the files actually parsed, `manifest` = the sentinel
`(inferred)`), which flows through the two existing `Some(info)` guards
unchanged and so emits the `:Project` node and one `HAS_SOURCE` per source
file. A new `Project HAS_DOC Doc` edge anchors every ingested doc; the semantic
`MENTIONS` / `DOCUMENTS` edges are untouched.

Thirteen of the fourteen corpora are manifestless, so thirteen digests moved.
**`rust_xfile` — the only manifest-backed corpus — came back byte-identical,
and that is the proof the change is additive for manifest-backed graphs.** It
holds because the inferred project adds **no new property column** to the
`:Project` dataframe: "inferred" is carried by the existing `manifest`
property, since the per-node property sweep would otherwise move every
manifest-backed digest too. Verified exclusive by `git status` after
`capture_goldens`: exactly thirteen `.sha256` files modified, `rust_xfile`
absent. `HAS_DOC` on a manifest-backed project is corpus-uncovered (no corpus
has both a manifest and docs) and is pinned by a unit test instead. Determinism
of the new node and edges is covered by `golden_parity`'s three builds per
corpus.

One defect was found and fixed alongside it: the single-rev path extracted into
its randomly-named tempdir, so the build root's basename — which is
user-visible, as Python fallback module names derive from it — differed per
build (`kglite-rev-xiPP2N.app.compute`). Two builds of the same revision
therefore disagreed on ids; the inferred project would have inherited the same
randomness in its name. Single-rev now extracts into the same fixed `snapshot`
basename the multi-rev path already used, which also aligns single-rev ids with
multi-rev ids. Regression test: `single_rev_builds_of_one_revision_agree_on_ids`.

## Track D — Python absolute imports — 2026-08-10 (branch `feat/backlog-2026-08`)

**Deliberate digest movement, two corpora.** The Python parser prefixes every
module path with the source root's directory name (`pkg/app.py` →
`py_basic.pkg.app`) while import specifiers are root-relative (`pkg.util`), so
the resolver's prefix walk could never match and Python absolute imports
produced **zero** `IMPORTS` edges — the carve-out recorded under Release 0.1.5
below. The fix is resolver-side and additive (`builder/other_edges.rs`): after
the raw prefix walk misses, Python specifiers are retried under the recovered
root prefix, extended by each of the importing file's ancestor directories
(covering `src/` layouts). The parser is untouched.

Movement, verified exclusive by `git status` after `capture_goldens`: **only**
`py_basic` and `py_nested_defs` moved. Measured edge deltas — `py_basic`
0 → 1 `IMPORTS(File→Module)` + 1 `IMPORTS(File→File)`; `py_nested_defs`
0 → 2 + 2; its CALLS edges become `import_backed`. Every other corpus,
including `cross_ts_py` and all TS/JS corpora, is byte-identical: a clone
layout (`xarray/core/dataset.py` → `xarray.core.dataset`) has no prefix to
recover, so it generates no new candidates at all. Gate mutated to confirm it
can fail: disabling the new branch turns three new unit tests, the
`codingest_stats` `import_backed` test and both regenerated goldens red.

## Release 0.1.6 verification — 2026-08-01

The release-mode gate is green: `golden_parity`, `rev_self_consistency` and
`kgl_bytes_are_stable_across_builds` all pass inside the
`cargo test --workspace --release` run, now across **12** corpora.

**Four corpora were added and no existing golden moved** — the two facts belong
together, because the second is what makes the first necessary. The closure-walk
work changes TS/JS and Python parser output substantially, and the eight-corpus
net did not notice: verified before the work started, **not one committed corpus
contained a single `const` fn-literal binding, a `function*`, a factory-wrapped
binding, a nested `def`, or an `.mdx` file.** The whole class could have been
changed, or silently broken, with every digest staying green. That is a hole in
the net, not evidence of safety, and each phase closed its own part of it in the
same commit as the change:

- `ts_hof_binding` — factory-wrapped bindings and the three grammar-vocabulary
  defects (`const x = function(){}`, `const x = function*(){}`, and a top-level
  `function* g(){}` that produced **no node at all**).
- `ts_closure_scope` — the Effect-style `Layer.effect(S, Effect.gen(…))` shape,
  an IIFE module factory, a React-hook factory, a nested named arrow whose calls
  must attach to it, **a binding under an anonymous callback that must not
  become a node**, `const x = arr.map(f)` at depth > 0 staying a `Constant`, and
  two same-named nested fns in sibling blocks (the `#{line}` tie-break).
- `py_nested_defs` — decorator factory, closure factory, nested helper, and the
  conditional-definition duplicate that makes the tie-break routine in Python.
- `docs_mdx` — an `.mdx` with frontmatter and a code-symbol mention, plus
  `README.MD` and a markdown-shaped `NOTES.txt` that pins the `.txt` rejection.

**Every new gate was mutation-tested, not merely read.** For each corpus the
thing it guards was broken, `golden_parity` was confirmed **red**, the change was
restored, and green was confirmed — and each probe was diffed against a saved
copy first to prove the edit had actually landed, because a probe that silently
edits the wrong text makes a working gate look broken and an unchanged file makes
a dead gate look alive. Twenty-one probes across the release. Two are recorded as
deliberate **null results** rather than dressed up as passes: dropping `.MDX`
from `strip_doc_ext` moves only a unit test (no corpus uses the uppercase form),
and removing Python's anonymous-scope prune changes nothing at all — which is the
evidence that D1's clause 5 is *structurally* vacuous in Python rather than merely
untested.

All eleven pre-existing digests came back byte-identical at every phase;
`capture_goldens` was run additively each time and `git status` confirmed only
the new `.sha256` appeared.

Cross-build query parity at release: **0 mismatches in 330 comparisons**
(11 queries × 3 repeats × 5 corpora × 2 independent builds), across opencode,
TanStack/query, fastify, flask and this repo. The Rust control corpus produced
an **exactly zero** delta on every counter — no Rust parser changed, and nothing
moved.

**Known and deliberately shipped, unchanged from 0.1.5:** Python absolute
imports still never produce `IMPORTS` edges, and `py_basic` still pins that
defect. This release did not touch import resolution — the Python phase was
explicitly fenced off from it — so the golden that freezes it is untouched.
It is tracked in the local backlog, not here.

## Release 0.1.5 verification — 2026-08-01

The release-mode gate is green: `golden_parity`, `rev_self_consistency` and the
new `kgl_bytes_are_stable_across_builds` all pass in the
`cargo test --workspace --release` run, across **8** corpora.

**The goldens did not move during this release, and that is the point.** The
Track C section below records the one deliberate regeneration, which happened on
the feature branch with its evidence captured at the time. By release time that
decision is closed: a red `golden_parity` here would have been a regression to
diagnose, never a regen. It stayed green.

The regeneration was additionally re-verified independently before tagging, by a
reviewer that did not trust this file: v0.1.4 was extracted via `git archive`
and built with its own locked dependencies, canonical renderings were dumped
from both versions, **both ends were anchored** (the v0.1.4 dumps hash to the
old goldens 7/7; HEAD's hash to the committed goldens 8/8), and the two were
section-diffed with an independent parser. Result: sections 1–4 byte-identical,
edge key sets identical in every corpus, **0 removals and 0 mutations** — the
only change is the three added properties on touched CALLS edges.

Cross-build query parity at release: **11 queries, 11 OK, 0 MISMATCH**, with
both builds producing identical 28,179-node / 59,522-edge graphs
(`corpus_sha256 04a90c5d…`, opencode pinned at `1e17856b`).

**Known and deliberately shipped (fixed after 0.1.7 — see Track D):** through
0.1.7, Python absolute imports never produced `IMPORTS` edges, so `py_basic`
pinned that behaviour — a golden that froze a defect. It predated every release
and 0.1.5 did not worsen it; the fix moved that golden *with* a recorded reason,
which is exactly what the protocol is for.

## Track C — graph resolution precision — 2026-08-01 (branch `feat/graph-resolution-precision`, shipped in 0.1.5)

The first builder-behaviour work since the goldens were frozen, so it is the
first entry that records **deliberate** digest movement rather than the absence
of it. `golden_parity` and `rev_self_consistency` are green, and a new sibling
gate `kgl_bytes_are_stable_across_builds` joins them.

**Corpus added.** `ts_monorepo` (13 files: two packages, a barrel, a
`.tsx` importer, a JSONC per-package `tsconfig.json` with a `paths` alias, two
named `package.json`s, and a deliberately dangling specifier). It exists
because the seven-corpus net was **blind to TS/JS import resolution** — not one
of them contained a single TypeScript `import`, so the whole subsystem could be
changed, or silently broken, with zero golden movement. Its digest is additive
and does not touch the historical authority digests.

**Two conscious regenerations, both verified rather than asserted.**

1. *TS import resolution* (Phases 2–3) — `ts_monorepo` only. Verified the
   strict way: `capture_goldens` rewrites every golden file, and `git status`
   afterwards reported only `ts_monorepo.sha256` as changed, so the seven
   pre-existing digests came back byte-identical.
2. *CALLS resolution metadata* (Phase 4) — `resolution` / `candidates` /
   `import_backed` on every tier-resolved CALLS edge. **6 of 8** goldens moved:
   `py_basic`, `py_inheritance`, `rust_xfile`, `ts_callback`,
   `dup_minified_assets`, `ts_monorepo`. `agc_basic` and `cross_ts_py` did not,
   and the mechanism was checked, not assumed — `agc_basic`'s four CALLS edges
   all come from the AGC semantic pass (they never touch the tiers, so the
   three properties stay null and the conditional columns are absent), and
   `cross_ts_py` has no CALLS edges at all.

   Because a change to the *edge set* hiding inside a properties-only
   regeneration would be blessed permanently, the canonical rendering was
   dumped before and after and diffed section by section (new `dump_canonical`
   diagnostic in `tests/parity.rs`). For every one of the eight corpora,
   `node_type_counts`, `edge_type_counts`, `node_identities` and `node_props`
   are **identical**; only `edge_props` differs, by exactly +3 lines per
   tier-resolved CALLS edge (`ts_monorepo`: +21 = 3 × 7).

**New gate: `.kgl` byte determinism.** `golden_parity` renders the graph from
sorted maps, so property *insertion* order is invisible to it — the bug class
that once produced identical in-memory digests from `.kgl` files differing
byte-for-byte. Three more properties per CALLS edge widens that exposure, so
`kgl_bytes_are_stable_across_builds` now builds `ts_monorepo` and `agc_basic`
three times each with `save_to` and compares the files. It was proven live:
removing the resolver's deterministic row sort leaves `golden_parity` **green**
and turns the byte test **red**.

`make determinism-soak REPO=<opencode> SOAK_RUNS=5` stable at 58,992 edges;
`make bench-smoke` green, 0 query mismatches in 11 queries × 2 builds.

**Performance.** Release build, min over 16 samples, opencode pinned at
`1e17856b`, `corpus_sha256`
`04a90c5d45cf620a3d85473ae8f660d5ef3e4af1c6d55666b333f53108c7dd31`:
0.468 s before → **0.476 s after (+1.7 %)**, inside the plan's +5 % budget,
while the graph grew from 43,038 to 59,522 edges. The tsconfig/package.json
discovery walk itself is 0.014 s. A first cut measured +8 %; the cause was
`package_targets` allocating a probe string per package per specifier
(~650k allocations/build) and the boundary check is now allocation-free.

## Release 0.1.4 verification — 2026-07-30

The frozen-record gate passes: `golden_parity` and `rev_self_consistency` both
green in the release-mode workspace run, all seven corpus digests matching.

This release changed **no builder code**, and the record reflects that rather
than re-deriving it. The only `crates/codingest/src` changes since `v0.1.3` are
the `codingest_bench` harness (which defines the measured corpus, not the graph)
and a comment in `rev.rs`; all seven `tests/goldens/*.sha256` files are
byte-identical to `v0.1.3`. The engine floor moved up two kglite patch releases
(the exact pair is in that release's notes),
which is the one change that *could* have shifted output — it did not, and that
was confirmed twice independently: the goldens did not move, and a matched
before/after bench capture (varying only the linked engine, against two
digest-identical corpora) reported identical node/edge counts on both, 991/3,518
for `crates/codingest/src` and 7,291/36,719 for the KGLite checkout.

Cross-build query parity: 0 mismatches in 220 checks (11 Cypher queries × 20
runs across two independent builds).

Per the performance protocol the release bench was **skipped deliberately**: no
perf-sensitive path changed since `v0.1.3`, so there is nothing to re-measure.
The engine-bump capture was taken in local working state (verdict: flat; the
large corpus agrees to 0.1%).

## Release 0.1.3 verification — 2026-07-22

The frozen-record gate passes: all seven corpus digests match, and
`rev_self_consistency` passes. The AGC semantic-fidelity work intentionally
changed only `agc_basic` (now `4e0c3d4aad2`); all six historical authority
digests remain byte-identical. Three release benchmark repetitions returned
identical results for all 11 Cypher queries in both independent builds (33/33,
zero mismatches).

The pinned Apollo-11 acceptance test also passes at commit
`911e5c0283c629c50cb97666f34065e8c07d71a5`: 737 direct inter-bank trampoline
sites resolve to their real program-local destinations, no semantic control
edge targets a trampoline helper, and no control or reference edge crosses an
AGC program boundary.

## Release 0.1.2 verification — 2026-07-22

The current frozen-record gate passes: all seven corpus digests match, and
`rev_self_consistency` passes. The release ran `cargo test --workspace` plus
`cargo test -p codingest --test parity` repeatedly through the feature,
hardening, dependency, and final release gates with zero unexplained golden
movement.

The release benchmark built the current workspace twice per run and returned
identical results for all 11 Cypher queries in three repetitions (33/33, zero
mismatches). The minimum build time was 0.046 s versus the dependency-refresh
baseline of 0.047 s. Apollo-11 at
`911e5c0283c629c50cb97666f34065e8c07d71a5` retained exactly 14,682 nodes /
54,987 edges and its pinned call-resolution counters; its minimum was 0.052 s
versus 0.053 s before the refresh. Both timing deltas are flat-to-improved.

## Update 2026-07-16: in-tree builder removed — parity now enforced by frozen record

KGLite deleted its in-tree `kglite::code_tree` builder on 2026-07-16 (the
planned handover — codingest is now the only builder). **Cross-builder
comparison is therefore no longer possible, and no longer needed.** The live
two-builder tests (`corpus_parity`, `rev_path_parity`) were removed from
`crates/codingest/tests/parity.rs`. Parity is now enforced by three surviving,
single-builder mechanisms:

1. **Golden digests + determinism** (`golden_parity`) — per-corpus SHA-256s
   captured 2026-07-16 from the last in-sync in-tree authority, while the two
   builders were still verified byte-for-byte identical (§1 below was green).
   Each corpus is rebuilt with the codingest builder **three times**; every
   build's canonical digest must equal every other build's (determinism —
   randomized `HashMap` iteration order is what the `dup_minified_assets`
   corpus reproduces) and must equal the frozen golden (behaviour). The two
   failure modes are reported separately because they call for opposite
   responses: a behaviour change may legitimately be regenerated,
   nondeterminism never may.
2. **Rev self-consistency** (`rev_self_consistency`) — the multi-rev fixture
   can't be frozen (fresh commit SHAs leak into `revs`), so it builds the same
   2-commit repo twice with the codingest builder and asserts equivalence,
   including the stamped `revs`/`rev_fp` provenance.
3. **The bench query-parity harness** (`codingest_bench`) —
   builds the target twice with the codingest builder and asserts identical
   Cypher query results across the two builds (a determinism check; any MISMATCH
   fails the gate).

Sections §1–§4 below are the historical parity/perf record captured while both
builders still existed — retained as the evidence behind the frozen goldens.

## 1. Corpus parity test (permanent regression test)

`crates/codingest/tests/parity.rs` — run with `cargo test --workspace --test parity`.
Result: **2 passed, 0 failed**.

- `corpus_parity`: for each of `tests/corpus/{py_basic, py_inheritance, rust_xfile,
  ts_callback, cross_ts_py}`, builds the same directory with
  `kglite::code_tree::builder::run_with_options` and
  `codingest::builder::run_with_options` using identical arguments
  (`verbose=false, include_tests=true, save_to=None, max_loc_per_file=None,
  include_docs=true` — docs pass compiled on both sides: the standalone `docs`
  feature is default-on and enables `kglite/okf`). Asserts:
  - identical node-type → count maps
  - identical edge-type → count maps
  - identical sorted sets of `(node_type, id)` (id = qualified_name/path/title id)
  - identical per-node property maps — full sweep, every property, canonicalized
    via `Value`'s `Debug` form, both sides sorted
  - identical per-edge property maps — full sweep, keyed `(conn, src_id, tgt_id)`

- `rev_path_parity`: creates a throwaway git repo in a tempdir (2 commits: rev2
  removes a function, adds one, widens `foo`'s signature — a fingerprint change —
  and changes call edges), then runs `build_code_tree_revs` from BOTH sides over
  the same two revs and applies the same full equivalence check, **including the
  `revs` and `rev_fp` list properties on every node and the `revs` property on
  every edge**. This directly validates the one bridged internal-API gap in the
  standalone transform (`rev.rs` multi-rev stamping: `node.properties.insert(...)`
  → `node.set_property(...)` with a throwaway interner, and `get_or_intern` →
  `try_get_or_intern().expect()`). A sanity assertion confirms the merged graph
  actually carries stamped `revs` lists before comparing. Skips with a clear
  message if `git` is unavailable (it was available: git 2.48.1).

**Property exclusions: none.** No nondeterministic property was found — file
paths are stored relative to the project root, so even the two rev builds
(distinct tempdir snapshots) produce identical property maps.

## 1b. Golden-digest oracle (the survivor)

Added 2026-07-16. The two tests in §1 are the *live* cross-check: they build
each input with BOTH builders and will be **deleted together with KGLite's
in-tree builder**. To keep the authority after that deletion, it was frozen
while the builders were still verified-identical (§1 green) into per-corpus
SHA-256 golden digests at `crates/codingest/tests/goldens/<corpus>.sha256`.

- `golden_parity` (in `tests/parity.rs`) builds each of the 7 corpora with
  **only** `codingest::builder::run_with_options`, renders the graph to a
  deterministic exhaustive string (`canonical_graph_string` — the same
  node/edge count maps, identity set, and full property sweeps that §1
  compares), SHA-256s it, and asserts it equals the stored golden. It never
  references `kglite::code_tree`, so it outlives the in-tree deletion. Digests
  **as first captured from the in-tree authority** (first 12 hex) — a historical
  record, not the current file contents; several have since been moved by
  recorded, deliberate builder changes, so read
  `crates/codingest/tests/goldens/*.sha256` for what is in force:
  `py_basic 83c20d86fa6c`, `py_inheritance d27d37313d02`,
  `rust_xfile a44952b16301`, `ts_callback ea30ba202d55`,
  `cross_ts_py 16abbe05f4bc`, `dup_minified_assets 5a0799382c3b`.
  The additive AGC corpus was reviewed and captured with the AGC parser on
  2026-07-21, then intentionally refreshed for the 0.1.3 semantic model on
  2026-07-22: `agc_basic 4e0c3d4aad2c`. It supplements the six historical
  authority digests without changing them.
  `py_basic`'s lineage since: `83c20d86fa6c` (in-tree authority) →
  `c362f2a87ed4` (commit `8094244`, which added `resolution`, `candidates` and
  `import_backed` properties to CALLS edges) → `11478e9dded5` (Track D, Python
  absolute imports now resolve). Each step is a deliberate builder change with
  its reason in the moving commit.
- `capture_goldens` (`#[ignore]`) regenerates the goldens; while the in-tree
  builder exists it captures from that authority, and retargets to the
  codingest builder once the in-tree builder is deleted (documented at the
  call site and in `tests/goldens/README.md`).
- **Rev fixture not frozen.** The multi-rev tempdir repo gets fresh commit
  SHAs each run, and those SHAs are stored in the `revs` node/edge property, so
  its canonical digest is unstable across from-scratch runs (verified: two
  fresh repos of identical content produced different commit SHAs). Instead of
  a golden, `rev_self_consistency` builds the same repo twice with the
  codingest builder and asserts the two graphs are equivalent (including the
  stamped `revs`/`rev_fp` provenance) — a post-deletion-safe determinism check.

## 2. Real-repo stats diff

Both `codingest_stats` bins built `--release` in their own workspaces
(`cargo build -p kglite --bin code_tree_stats --release`,
`cargo build -p codingest --bin codingest_stats --release`). Source-level diff of
the two bins: only the doc-comment usage lines and the crate path
(`kglite::code_tree::builder` vs `code_tree::builder`) differ.

JSON outputs diffed with `jq -S` (sorted keys):

| Target | Result |
|---|---|
| `KGLite/crates/kglite/src` | identical after excluding `build_secs` |
| `codingest/crates/codingest` | **byte-identical including `build_secs`** (both `0.031`) |
| `KGLite` repo root (default) | identical after excluding `build_secs` |
| `KGLite` repo root (`--include-tests`) | identical after excluding `build_secs` |

**Excluded field: `build_secs` only** — it is the measured wall-clock build
time, inherently run-dependent. Every other field (nodes, edges, total_calls,
excluded_noise, no_candidate, ambiguous_dropped, resolved_call_sites,
resolved_via_inheritance, resolved_edges, resolution_rate, path,
include_tests) matched exactly on all targets. Reference figures on
`kglite/src`: 7045 nodes, 33028 edges, resolution_rate 0.505.

## 3. Performance

Largest target: the KGLite repo root. 5 runs each, alternating in-tree ↔
standalone, warm filesystem cache, `/usr/bin/time -p` wall time plus the bin's
internal `build_secs` (excludes process startup + JSON emit). hyperfine not
installed.

| Workload | in-tree median | standalone median | delta |
|---|---|---|---|
| KGLite root, default (build_secs) | 0.289 s | 0.288 s | −0.3 % |
| KGLite root, default (wall, time -p) | 0.29 s | 0.29 s | 0 % |
| KGLite root, `--include-tests` (build_secs) | 0.467 s | 0.461 s | −1.3 % |

Raw samples — default: in-tree 0.284/0.285/0.289/0.290/0.301, standalone
0.286/0.287/0.288/0.291/0.295. `--include-tests`: in-tree
0.462/0.466/0.467/0.467/0.478, standalone 0.445/0.457/0.461/0.470/0.475.

Both within noise (±5 %); the standalone is marginally faster if anything.
Profile parity verified: the standalone workspace `Cargo.toml` carries
`[profile.release] lto = "thin", codegen-units = 1, strip = "symbols"`,
mirroring KGLite's workspace profile, and both bins were built from their
workspace roots so the profiles applied.

## 4. Discrepancies

None. No graph-content difference of any kind was observed across the 5
corpus dirs, the multi-rev merge case, or the 3 real-repo stats targets.
No fixes to the standalone transform were needed.
