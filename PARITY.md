# Parity Verification: in-tree `kglite::code_tree` vs standalone `codingest`

Date: 2026-07-15. Compared the in-tree module (`/Volumes/EksternalHome/Koding/Rust/KGLite/crates/kglite/src/code_tree/`)
against the standalone crate (`crates/codingest/`). Both link the same `kglite`
engine crate, so graphs from either builder are read through identical
`kglite::api` types (`DirGraph` / `NodeData` / `EdgeData` / `Value`).

**Verdict: full feature parity, full performance parity. Zero graph discrepancies found. No fixes required.**

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
Tracked in `dev-docs/plans/python-imports-never-resolve.md`.

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

**Known and deliberately shipped:** Python absolute imports never produce
`IMPORTS` edges, so `py_basic` pins that behaviour — a golden that currently
freezes a defect. It predates every release and 0.1.5 does not worsen it; the
fix will move that golden *with* a recorded reason, which is exactly what the
protocol is for. The consequence for users is documented in the CHANGELOG and
`docs/cli.md`: the `import_backed AND candidates = 1` filter removes
essentially all true cross-file Python call edges. Tracked in
`dev-docs/plans/python-imports-never-resolve.md`.

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
byte-identical to `v0.1.3`. The engine floor moved from kglite 0.15.0 to 0.15.3,
which is the one change that *could* have shifted output — it did not, and that
was confirmed twice independently: the goldens did not move, and a matched
before/after bench capture (varying only the linked engine, against two
digest-identical corpora) reported identical node/edge counts on both, 991/3,518
for `crates/codingest/src` and 7,291/36,719 for the KGLite checkout.

Cross-build query parity: 0 mismatches in 220 checks (11 Cypher queries × 20
runs across two independent builds).

Per the performance protocol the release bench was **skipped deliberately**: no
perf-sensitive path changed since `v0.1.3`, so there is nothing to re-measure.
The engine-bump capture is recorded at
`dev-docs/bench/out/phase1-kglite-0153-engine-bump.md` (verdict: flat; the large
corpus agrees to 0.1%).

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

`crates/codingest/tests/parity.rs` — run with `cargo test -p codingest --test parity`.
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
  captured from the in-tree authority (first 12 hex):
  `py_basic 83c20d86fa6c`, `py_inheritance d27d37313d02`,
  `rust_xfile a44952b16301`, `ts_callback ea30ba202d55`,
  `cross_ts_py 16abbe05f4bc`, `dup_minified_assets 5a0799382c3b`.
  The additive AGC corpus was reviewed and captured with the AGC parser on
  2026-07-21, then intentionally refreshed for the 0.1.3 semantic model on
  2026-07-22: `agc_basic 4e0c3d4aad2c`. It supplements the six historical
  authority digests without changing them.
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
