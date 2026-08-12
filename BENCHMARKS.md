# code-tree benchmarks — us vs. the (former) in-tree component

> **Historical record (captured 2026-07-15).** KGLite removed its in-tree
> `kglite::code_tree` builder on 2026-07-16, so the cross-builder comparison
> below can no longer be reproduced — it is retained as the last-known-good
> parity/perf snapshot taken while the two builders were verified identical.
> The `codingest_bench` harness now builds the target **twice with the
> codingest builder** and checks query-result parity across the two builds
> (determinism); the "in-tree" columns are the historical authority side.

## ⚠ The benchmark corpus changed on 2026-07-27 — earlier numbers are not comparable

**Read this before comparing any two numbers in this file.**

Every workspace-targeted measurement published below (`0.1.2`, `0.1.3`, and the
kglite 0.15.0 migration section) was taken by pointing `codingest_bench` at this
repository's **working directory**. The builder skips dot-directories,
`target/`, `node_modules/`, `venv/` and `__pycache__/` by name, but it has no
notion of `.gitignore` — so the gitignored `dev-docs/` and `inbox/` working
folders were ingested through the docs pass and counted as corpus. Those folders
are local scratch: they differ between machines, between checkouts, and between
one hour and the next on the same machine.

The size of the effect was measured, not assumed:

| Corpus | nodes | edges |
|---|---:|---:|
| clean `git worktree` of the pre-migration commit | 1,115 | 3,692 |
| working tree at that same commit | 1,170 | 3,759 |
| working tree, ~4 hours later, no commits | 1,177 | 3,867 |
| working tree, one scratch `.md` file added | 1,178 | 3,868 |

A single untracked markdown file moves the graph. The published 0.1.3 snapshot
records 3,760 edges for a tree that measures differently today with no code
change at all, and the migration capture had to freeze the tree mid-task once
the agent realised its own doc edits were changing the thing being measured.

**As of 2026-07-27 `codingest_bench` defines its own corpus.** It copies the
target's *git-tracked* files into a temporary directory and builds that, so the
input is a function of the revision (plus uncommitted edits to tracked files)
and nothing else. Every run now prints a corpus line:

```
corpus : tracked-only — 198 files, 1687711 bytes, sha256 2d081a2b…
```

`corpus_sha256` is the comparability token: **two numbers are comparable only if
their corpus digests match.** `--include-untracked` restores the old
build-the-directory-as-is behaviour for one-off measurement of a non-git tree
and prints a NOT-REPRODUCIBLE banner instead of a digest. `make gate`'s
bench-smoke step fails if the harness does not resolve a tracked-only corpus.

**Machine load is not a precondition for a capture; it is metadata on it.**
Captures run in release mode under whatever load the machine has. The validity
lives in the instrument, not in the machine's quietness: a **CONTROL** cell (a
query or corpus the change cannot have touched) is the drift meter — load moves
every cell, a real regression moves one, and a control that moves too voids the
whole capture rather than licensing a choice about which cells to believe — plus
two agreeing runs and one confirmation retake when a verdict lands near its
threshold. A stricter "quiet machine only" mandate applied here until 2026-08-09
and was retired because it cost more in deferred captures and stalled releases
than the precision it bought. Its corollary stays and is the reason the
capture-condition notes below are written out rather than smoothed over: **a
number compared across sessions records the conditions it was taken under.**
Release-time longitudinal captures therefore state their machine state (load,
concurrent builds) alongside the `corpus_sha256` — metadata, never a gate on
taking the capture. An unrecorded hot baseline reads as real drift to the next
release's comparison.

Consequences for this document:

- The **methodology below is unchanged** — same 11 queries, same alternating
  A/B timed iterations, same min-of-repetitions reporting, same 33-check
  per-side parity gate. Only the *input* changed.
- Node/edge counts and absolute timings from before this date describe a corpus
  that cannot be reconstructed. Do **not** read a delta across the 2026-07-27
  line as a builder or engine effect.
- The external-target sections (mistral.rs, KGLite, distillPDF, petekSuite,
  Apollo-11) were never affected by this: they were measured on other
  checkouts, whose own untracked state is a separate (and unrecorded) variable.
- The first tracked-only reading of this workspace, at
  `chore/harden-gate-corpus`, is **1,122 nodes / 3,800 edges**, corpus
  `2d081a2bd90a58e2…` (198 files, 1,687,711 bytes). No timings were captured
  with it: the machine was not idle, and the performance protocol then in force
  required a quiet machine for a release-time timing baseline. **That mandate
  was retired on 2026-08-09** (see the capture-conditions note above) — a
  present-day capture in this situation runs anyway, under a CONTROL cell and
  with its machine state recorded. The next release capture starts the new
  comparable series.

## Release 0.2.1 — 2026-08-12 (kglite 0.15.13 engine move: anchored traversals 5.7–327× faster)

**No codingest source changed this release** — `git diff v0.2.0..HEAD -- crates/codingest/src`
is empty. The movement is entirely kglite 0.15.11 → 0.15.13, whose planner fix
(a join filtered on a type's title/id field was estimated as excluding nothing,
present since 0.11.9) reaches us through the canonical `title` / `id` aliases
code_tree nodes carry.

**Corpus:** KGLite tracked-only, `corpus_sha256`
`69b2872a4c6ab70628f6fadf30e0f2db87f8f2030804df33059e9228056e9559` — 1,251
files / 20,756,050 bytes → 8,691 nodes / 43,868 edges. Release build
(`lto=thin`, `codegen-units=1`), `codingest_bench`, WARMUP 3 / ITERS 20,
per-query median.

**Design: A/B/A.** The first A→B read showed every non-anchored cell moving
+2…+13 % one way, which reads as machine drift rather than eight independent
regressions. 0.15.11 was therefore rebuilt and re-measured *after* the 0.15.13
arm; it landed within **−3.8…+4.5 %** of its own earlier run. The machine was
steady, so both the wins and the regressions below are real. `Δ` compares
0.15.13 against the **adjacent** late-0.15.11 run.

| Query | 0.15.11 (late) | 0.15.13 | Δ |
|---|---:|---:|---:|
| `two_hop_into_hot` (2-hop traversal) | 4.389 | 0.013 | **−99.7 %** |
| `reverse_callees_of_hub` (reverse 1-hop) | 0.983 | 0.021 | **−97.9 %** |
| `anchored_callers` (anchored 1-hop, in-hub) | 1.161 | 0.204 | **−82.4 %** |
| `calls_edge_scan` (1-hop edge scan) | 1.042 | 1.038 | −0.4 % |
| `contains_new` (CONTAINS filter) | 0.096 | 0.098 | +2.1 % |
| `varlen_callers_1_3` (`[:CALLS*1..3]`) | 2.934 | 3.023 | +3.0 % |
| `method_calls_mix` (3-type join) | 0.101 | 0.104 | +3.5 % |
| `eq_filter_pub` (equality property filter) | 0.160 | 0.178 | **+11.2 %** |
| `defs_per_file` (grouped aggregation) | 0.083 | 0.094 | **+13.3 %** |
| `top20_by_branch_count` (ORDER BY + LIMIT) | 2.503 | 2.881 | **+15.1 %** |

Median ms. `count_functions` omitted — 0.000 ms on both arms, below timer
resolution and a dead cell in this harness.

**Validity.** Row counts identical on every query on both arms (259/259, 61/61,
20/20, …) and the hot anchors resolved to the same two nodes — a shrinking
result set is the obvious way to fake a 327×, and it is ruled out. min/median
0.88–1.00, so no heavy tails and the medians sit on real floors. An independent
corpus (codingest `crates/codingest/src`, digest `5ab22bc61c70…`, 1,238 nodes —
never pooled with the above) agrees on direction: −65.1 %, −99.1 %, −92.6 % on
the same three queries. Parity goldens unmoved.

**The three regressions are real, not noise** — they sit outside the ±4.5 %
control band and reproduce on both corpora. They land on cheap cells, so the net
is decisively positive, but they are a genuine cost of the planner change.

**The perf anchor returned VOID (4) in docs-off mode and was not overridden.**
Its control query is `anchored_callers`, one of the three the bump improves, so
it moved −71.43 % per row — identically across three consecutive re-measures.
A deterministic move is not the instrument wandering: the control's premise,
that nothing we change can touch it, is void once the engine floor moves under
it. docs-on saw the same −69.23 % and returned PASS only because its raw delta
fell 0.001 ms under the 0.01 ms jitter floor, so that PASS is luck rather than
evidence. No baseline was regenerated to make either verdict go away; the perf
question is answered by the controlled capture above instead. **A control query
a dependency bump can improve is not a control** — the gate needs one the
engine cannot reach.

## Release 0.2.0 — 2026-08-11 (backlog program: parse dispatch, fingerprint, AGC frames; kglite 0.15.11)

Perf-sensitive paths changed this release (single-dispatch parse worklist,
fingerprint rescope, control-edge frames, kglite engine 0.15.8→0.15.11), so
the record refreshes. All numbers release-build; full rows in the local bench
ledger (`phase9-*`, `phase11-lpt-parsewall`, `phase12-*`, `clitimer-*`).

- **Parse wall (single-dispatch worklist, P9):** KGLite corpus
  `0882abb4c2b1` 0.320→0.281 s (**−12.0%**), mistral.rs `8c44399b4047`
  0.294→0.239 s (**−18.9%**); whole build −9.6% / −13.3%. CONTROL query
  medians moved ±4.3% with no systematic direction. Byte-identical graphs
  (15/15 goldens + 5-run determinism soak).
- **Freshness fingerprint (ingestibility scoping + parallel hash, P10):**
  KGLite checkout, warm min-of-5 **0.299 s → 0.013 s**; first-run
  1.440 → 0.105 s; hashed set 231.7 MB / 3250 files → 16.8 MB / 1150 files.
  Paid on every `build`/`status`/`query` freshness check.
- **LPT dispatch: measured and rejected** (pre-registered ≥10 ms bar):
  −2/+4 ms head-of-line saving; naive size-sorted dispatch REGRESSES +245 ms
  under rayon's contiguous chunking. Decision recorded; do not re-attempt
  without new evidence.
- **AGC control-edge insertion (kglite 0.15.10 index+intern, our ask):**
  engine-side `add_connections` 0.0173→0.0097 s (**−44%**) on the frozen
  synthetic AGC corpus `cc4c17e6…4453`; builder share now 11.2%. Reported
  upstream; their second-look trigger fires (engine still 88.8%).
- **Drift anchor (first live release run):** PASS in both docs modes vs the
  0.1.7 baseline on settled, twice-agreeing captures (control +7.69%). Two
  earlier load-contaminated captures were caught by the gate itself — one
  VOID, one FAIL that did not survive settling — and the run surfaced+fixed a
  comparator defect (control voided on sub-floor jitter, contradicting its
  documented floor contract; red-first tested). 0.2.0 baselines captured
  post-release per `tests/benchmarks/README.md`.

## Release 0.1.6 — 2026-08-01 (closure-scoped definitions + `.mdx` docs)

**Not comparable to the 0.1.5 section below.** That capture used opencode at
`1e17856b`, `corpus_sha256 04a90c5d…`; this one uses opencode at `32f278b4`,
`corpus_sha256 312f41cf7b3e8facf41245a86779528d97cbe07fdd606f736d1e36c362703ad5`
(6,351 files / 127,414,487 bytes). Different digest, different corpus, no delta
may be read across the two.

Five corpora were measured rather than one, because the change is a parser-walk
change and the risk was that it fitted the Effect-TS idiom that motivated it.
Each row is `main` (`ed8f3ae`) vs the release commit, release build, identical
staged tree on both sides, **min over 7**. Both configurations are published:
docs-off is the historically comparable series, docs-on is what the builder
actually does for a user who passes `--include-docs`.

| corpus (`corpus_sha256`) | nodes before → after | build before → after (docs off) |
|---|---|---|
| opencode `312f41cf…` | 28,097 → **31,145** (+10.85 %) | 0.464 s → **0.524 s** (+12.93 %) |
| TanStack/query `3fe142b7…` | 5,069 → **5,200** (+2.58 %) | 0.120 s → 0.122 s (+1.67 %) |
| fastify `ec82f718…` | 864 → **936** (+8.33 %) | 0.046 s → 0.045 s (−2.17 %) |
| flask `467098f4…` | 641 → **676** (+5.46 %) | 0.017 s → 0.018 s (+5.88 %) |
| **codingest (Rust control) `0364afb9…`** | **1,210 → 1,210 (0)** | **0.045 s → 0.045 s (0 %)** |

The Rust control is the load-bearing row: no Rust parser changed, and the delta
is **exactly zero on every counter** — nodes, edges, 901 Function nodes, and all
11 queries. A non-zero number there would have meant the walk was reaching
further than intended.

**Two ceilings are exceeded in the docs-on configuration and are published as
exceeded**: opencode nodes +13.03 % against a ≤12 % budget, and build time
+20.16 % against ≤15 %. Both budgets were set against the docs-off denominator
before the `.mdx` work was quantified. The excess decomposes exactly: docs-on
`+3,675` nodes = `+3,048` code nodes (the same growth that passes at 10.85 %
docs-off) `+ 627` Doc nodes, and 627 is precisely the `.mdx` file count. The
docs pass goes 0.027 s → 0.066 s for those 627 files — **per-doc cost falls from
231 µs to 89 µs**. The budgets are being restated per configuration rather than
the result being reinterpreted to fit them.

**Per-query medians.** Raw, opencode is 6 of 11 queries over a +10 % ceiling
(worst +69.5 %). That ceiling is unachievable by construction here: the call
graph got denser on purpose — Function nodes 7,813 → 11,184 (+43.1 %), CALLS
edges 14,136 → 23,319 (+65.0 %), the latter because the D4 repair recovered call
sites that were previously **dropped entirely**. Normalized per row returned the
engine is flat (−0.1 % on `calls_edge_scan`, a pure edge scan and the cleanest
available signal). **One genuine regression survives normalization:**
`eq_filter_pub`, a full-Function-frame equality filter, at **+18.4 % per row**,
with a clean dose-response across corpora (Rust control 0 %, TanStack ~8 %,
opencode 18 %) pointing at the widened Function frame — the scan pays for sparse
columns it does not read. Linear rather than compounding, ~0.1 ms absolute. It
is engine-side and has been routed to KGLite rather than absorbed silently here.

Cross-build query-result parity: **0 mismatches in 330 comparisons** (11 queries
× 3 repeats × 5 corpora × 2 independent builds).

The full report — the precision gate and every per-query row — was written to
local working state and is not part of this committed record.

## Release 0.1.5 — 2026-08-01 (Track C: TS/JS import resolution + CALLS metadata)

**The first release since the goldens were frozen that changes builder output**,
so unlike 0.1.4 the bench was mandatory rather than skipped.

All numbers below share one corpus — the **opencode** monorepo pinned at
`1e17856b`, tracked-only, **6,346 files / 127,343,975 bytes**,
`corpus_sha256 04a90c5d45cf620a3d85473ae8f660d5ef3e4af1c6d55666b333f53108c7dd31`.
Quote that digest with any figure taken from here; a number without it is not
comparable to anything.

| | before | after |
|---|---|---|
| build (release, **min over 16**) | 0.468 s | **0.476 s** (+1.7 %) |
| graph | 43,038 edges | **59,522 edges** (28,179 nodes) |
| `IMPORTS` File→File | 73 | **8,039** |
| `IMPORTS` File→Module | 77 | **8,595** |
| labeled CALLS precision (`import_backed AND candidates = 1`) | 5.0 % | **87.5 %** |

+1.7 % build time for a graph 38 % larger, with dependency edges going from
"effectively absent" to complete, is the trade this release makes. It sits
inside the +5 % budget the plan set before the work started, rather than a
budget chosen afterwards to fit the result.

A first cut of the alias work measured **+8 %**. The cause was found rather than
absorbed — `package_targets` allocated a probe string per package per specifier
(~650 k allocations per build) — and the boundary check is now allocation-free.

**Release-time verification** (`codingest_bench`, same corpus, two independent
codingest builds, 3 warmup + 20 timed iterations alternating A/B): both builds
produced **identical** graphs at 28,179 nodes / 59,522 edges, and query parity
was **11 queries, 11 OK, 0 MISMATCH**. Per-query medians agreed within 0.5 %
across builds. `make determinism-soak` was stable at 58,992 edges during the
work itself (a different corpus scope — do not compare it with the figures
above).

## kglite 0.15.0 engine migration — 2026-07-27

> **Corpus caveat (2026-07-27):** measured on the working tree, which then
> included the gitignored `dev-docs/` and `inbox/` folders. Not reproducible;
> see the corpus-change notice at the top of this file.

Matched before/after capture isolating the engine bump (kglite 0.14.5 ->
0.15.0). **No detectable regression, and no change in graph output.**

Methodology deviates from the sections below in one deliberate way, and it
matters. Both binaries were pointed at a **single fixed target directory** (this
workspace) rather than each at its own checkout. The builder ingests gitignored
local content — `dev-docs/`, `inbox/` — through the docs pass, so a `git
worktree` of the pre-migration commit scores 1,115 nodes / 3,692 edges while the
working tree scores 1,170 / 3,759. Giving each engine its own checkout would
have booked that ~5% tree-content difference as an engine effect. Runs also
alternated BEFORE/AFTER/BEFORE/AFTER rather than three-then-three, so background
load is shared across both arms instead of loading one.

All six runs (3 repetitions x 2 engines) produced **1,170 nodes / 3,759 edges**
and **11 queries, 11 OK, 0 MISMATCH** — 33 checks per side, 66 total. The two
engine versions produce byte-identical graphs and identical query results.

Per-query minimum over 6 samples per side (3 repetitions x 2 independent builds),
milliseconds:

| Query | before | after | delta |
|---|---:|---:|---:|
| count_functions (full-label scan + count) | ~0.000 | ~0.000 | n/a¹ |
| eq_filter_pub (equality property filter) | 0.026 | 0.027 | +3.8% |
| contains_new (CONTAINS string filter) | 0.018 | 0.018 | +0.0% |
| top20_by_branch_count (ORDER BY + LIMIT) | 0.320 | 0.307 | −4.1% |
| defs_per_file (grouped aggregation) | 0.017 | 0.017 | +0.0% |
| calls_edge_scan (1-hop edge scan + count) | 0.083 | 0.085 | +2.4% |
| anchored_callers (anchored 1-hop, in-hub) | 0.156 | 0.155 | −0.6% |
| two_hop_into_hot (2-hop traversal + count) | 0.432 | 0.431 | −0.2% |
| varlen_callers_1_3 (`[:CALLS*1..3]` + count DISTINCT) | 0.374 | 0.373 | −0.3% |
| reverse_callees_of_hub (reverse-direction 1-hop) | 0.094 | 0.095 | +1.1% |
| method_calls_mix (Struct-`HAS_METHOD`→Fn-`CALLS`→Fn) | 0.015 | 0.015 | +0.0% |

Deltas are un-signed and scattered (three negative, four positive, three at the
measurement floor), with the largest movement in *either* direction near 4%.
Per the "Reading the results" note below, a genuine divergence would appear as a
MISMATCH or as a systematic same-signed delta across queries; neither is present.

Build-time minimum moved 0.046 s → 0.048 s. **This is not reported as a
regression**: `build_secs` has 1 ms resolution and the workload is ~47 ms, so the
difference is two ticks, at the harness's floor — the same caveat footnote ¹ in
the build-times section describes. It is also uncorroborated by the query
workload above, which moved un-signed.

**Capture conditions were not ideal and are stated rather than smoothed over.**
Three repository migrations were running on this machine concurrently. Load
average held flat across the window (1-min 2.87 → 2.88; 5-min 3.54 → 3.53) and
`syspolicyd` was ~14–20% throughout (Gatekeeper assessing freshly linked
binaries). The absolute milliseconds therefore run hot and should not be compared
against the quiet-machine numbers in the sections below; the **before/after
delta** is the load-bearing result, because shared contention lifts both arms
together and the alternating layout distributes it evenly. This is the same
confound the closing note of this document already records from the other
direction.

## Release 0.1.3 snapshot — 2026-07-22

> **Corpus caveat (2026-07-27):** measured on the working tree, which then
> included the gitignored `dev-docs/` and `inbox/` folders. Not reproducible;
> see the corpus-change notice at the top of this file.

Release build on Apple M4 / macOS. Three `codingest_bench` repetitions against
this workspace produced independent build pairs of 0.079/0.047, 0.071/0.050,
and 0.071/0.048 seconds. The 0.047 s minimum is within noise of 0.1.2's 0.046 s
minimum (+2.2%). Every repetition returned identical results for all 11
queries: 33 OK, 0 mismatches. The workspace graph now contains 1,170 nodes /
3,760 edges; the increase is expected from the new AGC semantic builder code
and bundled MCP installation surface.

The pinned Apollo-11 corpus produced 14,756 nodes / 46,825 edges (including 74
documentation nodes) with a 0.107 s minimum across three independent build
pairs. The previous model produced 14,682 nodes / 54,987 edges at 0.052 s. The
roughly 2.1x build cost is a known regression from richer per-site semantic
metadata; the large edge reduction is intentional removal of false
cross-program references, partly offset by explicit jump, branch, alias, and
data-point relationships. All 33 Apollo query comparisons matched. A dedicated
performance follow-up remains tracked in the local backlog.

## Release 0.1.2 snapshot — 2026-07-22

> **Corpus caveat (2026-07-27):** measured on the working tree, which then
> included the gitignored `dev-docs/` and `inbox/` folders. Not reproducible;
> see the corpus-change notice at the top of this file.

Release build on Apple M4 / macOS. Three `codingest_bench` repetitions against
this workspace produced independent build pairs of 0.075/0.046, 0.048/0.050,
and 0.048/0.050 seconds. The minimum is 0.046 s versus the pre-refresh 0.047 s
minimum (−2.1%, flat-to-improved). Every repetition returned identical results
for all 11 queries: 33 OK, 0 mismatches.

The workspace graph contained 1,057 nodes / 3,516 edges. The three-edge
reduction from the 3,519-edge dependency-refresh baseline is expected: Phase 1
removed three unused direct manifest dependencies that were represented as
dependency edges. Frozen corpus output did not move.

Apollo-11 validation retained exactly 14,682 nodes / 54,987 edges and the
pinned call-resolution shape (10,566 total, 1,761 excluded, 945 no candidate,
0 ambiguous, 7,860 resolved sites / 6,741 edges, rate 0.8927). Repeated release
builds reached 0.052 s versus the 0.053 s baseline (−1.9%, flat-to-improved).

This document compares the **standalone `codingest` crate** (this workspace) against
the **in-tree `kglite::code_tree` module** (the equivalent builder that then lived
inside the `kglite` dependency). Both crates depend on the *same* `kglite` engine, so the
`DirGraph` / `NodeData` / `EdgeData` / `Value` types are identical and a graph
produced by either builder is read through the same `kglite::api` accessors.

Two things are compared:

- **Cypher queries & traversals** (this file, below): an identical query workload
  run against a graph built by each builder, through the *same* kglite Cypher
  engine. This isolates any difference attributable to the graphs the two builders
  emit — there is no separate query engine "per side".
- **Build times** (placeholder section at the bottom): raw wall-time to parse a repo
  into a graph with each builder.

## Methodology (Cypher queries & traversals)

- **Same engine, two graphs.** For each target repo we build the directory twice —
  once with `kglite::code_tree::builder::run_with_options(...)` (in-tree) and once
  with `codingest::builder::run_with_options(...)` (standalone) — using identical
  arguments (`verbose=false, include_tests=false, save_to=None, max_loc=None,
  docs=true`). Every query then runs against *both* graphs through kglite's
  canonical read path, `kglite::api::session::execute_read` with
  `ExecuteOptions::eager` — the exact pipeline a kglite-cli / kglite-mcp-server user
  hits.
- **Alternating, warm-cache iterations.** Each query gets 3 warmup runs per graph,
  then 20 timed iterations executed **alternating** in-tree / standalone so any
  machine-wide jitter is shared evenly across both sides. We report the median and
  the min (ms) per side, and the standalone-vs-in-tree delta on the medians.
- **Result parity.** Before timing, each query's result is compared across the two
  graphs: row count plus an order-insensitive value digest (each row canonicalized
  with `{:?}` per cell, sorted, then hashed with the fixed-key `DefaultHasher`).
  Sorting the rows before hashing means query-ordering nondeterminism cannot produce
  a false mismatch — only a genuine difference in the returned row *multiset* would.
  A mismatch would skip timing for that query and be reported as `MISMATCH`.
- **Anchors are discovered at runtime.** The anchored/traversal queries pin to the
  CALLS in-degree hub (most-called function) and out-degree hub (function that calls
  the most), found per-repo with a first exploratory query, so the traversal
  workload is meaningful on whatever repo is passed.
- **Harness:** `crates/codingest/src/bin/codingest_bench.rs`
  (`cargo run -p codingest --bin codingest_bench --release -- <path> [--json]`).
- **Hardware / environment:** Apple M4, 16 GB RAM, macOS (Darwin 25.3.0); kglite
  0.13.4. **Date:** 2026-07-15.

The graphs were provably equivalent at capture time (parity was then enforced by
`crates/codingest/tests/parity.rs`; that authority is now frozen into the golden
digests it checks — see `PARITY.md`), so the expectation going in is: **identical query results, and
per-query timings equal within measurement noise.** That is what we observe — every
delta below is small (single-digit percent) and un-signed across queries, i.e.
measurement jitter rather than a systematic edge to either side.

## Cypher queries & traversals

Medians are of 20 warm alternating iterations; `delta` = standalone median vs.
in-tree median (positive = standalone slower). All queries returned identical
results from both graphs (`parity = OK`).

### Target: mistral.rs (12,249 nodes / 52,507 edges)

Anchors — in-hub: `…utils::unvarbuilder::UnVarBuilder::pp`;
out-hub: `…pipeline::multimodal::MultimodalLoader::load_model_from_path`.

| Query | rows | in-tree median (ms) | standalone median (ms) | delta | parity |
|---|---:|---:|---:|---:|:--:|
| count_functions (full-label scan + count) | 1 | ~0.000 | ~0.000 | n/a¹ | OK |
| eq_filter_pub (equality property filter) | 1 | 0.364 | 0.370 | +1.7% | OK |
| contains_new (CONTAINS string filter) | 1 | 0.231 | 0.229 | −0.6% | OK |
| top20_by_branch_count (ORDER BY + LIMIT) | 20 | 3.423 | 3.438 | +0.4% | OK |
| defs_per_file (grouped aggregation) | 20 | 0.122 | 0.117 | −4.1% | OK |
| calls_edge_scan (1-hop edge scan + count) | 1 | 1.009 | 0.974 | −3.5% | OK |
| anchored_callers (anchored 1-hop, in-hub) | 542 | 0.970 | 0.961 | −1.0% | OK |
| two_hop_into_hot (2-hop traversal + count) | 1 | 3.089 | 3.078 | −0.4% | OK |
| varlen_callers_1_3 (`[:CALLS*1..3]` + count DISTINCT) | 1 | 0.990 | 0.992 | +0.2% | OK |
| reverse_callees_of_hub (reverse-direction 1-hop) | 110 | 0.649 | 0.653 | +0.6% | OK |
| method_calls_mix (Struct-`HAS_METHOD`→Fn-`CALLS`→Fn) | 1 | 0.277 | 0.269 | −3.1% | OK |

**11 queries, 11 OK, 0 MISMATCH.**

### Target: KGLite (8,884 nodes / 42,543 edges)

Anchors — in-hub: `…datasets::sec::layout::StorageMode::as_str`;
out-hub: `…cypher::executor::match_clause::fused_match::CypherExecutor::execute_fused_match_return_aggregate`.

| Query | rows | in-tree median (ms) | standalone median (ms) | delta | parity |
|---|---:|---:|---:|---:|:--:|
| count_functions (full-label scan + count) | 1 | ~0.000 | ~0.000 | n/a¹ | OK |
| eq_filter_pub (equality property filter) | 1 | 0.278 | 0.289 | +4.1% | OK |
| contains_new (CONTAINS string filter) | 1 | 0.149 | 0.149 | +0.4% | OK |
| top20_by_branch_count (ORDER BY + LIMIT) | 20 | 2.567 | 2.532 | −1.4% | OK |
| defs_per_file (grouped aggregation) | 20 | 0.091 | 0.090 | −1.3% | OK |
| calls_edge_scan (1-hop edge scan + count) | 1 | 1.156 | 1.162 | +0.5% | OK |
| anchored_callers (anchored 1-hop, in-hub) | 331 | 1.144 | 1.137 | −0.6% | OK |
| two_hop_into_hot (2-hop traversal + count) | 1 | 4.229 | 4.187 | −1.0% | OK |
| varlen_callers_1_3 (`[:CALLS*1..3]` + count DISTINCT) | 1 | 2.116 | 2.115 | −0.0% | OK |
| reverse_callees_of_hub (reverse-direction 1-hop) | 55 | 0.938 | 0.935 | −0.3% | OK |
| method_calls_mix (Struct-`HAS_METHOD`→Fn-`CALLS`→Fn) | 1 | 0.105 | 0.106 | +0.2% | OK |

**11 queries, 11 OK, 0 MISMATCH.**

¹ `count_functions` is a metadata-only count that resolves in sub-microsecond time;
its median rounds to 0.000 ms, so a percentage delta is meaningless (measurement
floor). It is retained as a coverage case, not a timing signal.

### Reading the results

Across both repos every query returned byte-identical results from the two builders'
graphs, and every timing delta is small and un-signed (some queries nominally faster
in-tree, some standalone, none consistently). This is the expected outcome: the two
builders emit equivalent graphs and the same kglite engine executes the workload, so
there is no structural reason for one side to be faster. The benchmark's value is as
a **regression guard** — a future divergence in the standalone builder would surface
here either as a `MISMATCH` (graph shape changed) or as a systematic, same-signed
timing delta across queries (graph got heavier/lighter to traverse).

## Build times

Both `codingest_stats` binaries built `--release` in their own workspaces (both
with `lto = "thin"`, `codegen-units = 1`). Per target: 1 warmup per side, then
**7 timed runs per side, alternating** in-tree / standalone, on an otherwise
quiet machine. Timing is the binary's internal `build_secs` (parse + graph
construction, excluding process startup and JSON emit). Same hardware/date as
above. `+tests` = `--include-tests`.

| Target | nodes | edges | in-tree median (s) | min | standalone median (s) | min | delta | graph parity |
|---|---:|---:|---:|---:|---:|---:|---:|:--:|
| las-rs | 270 | 935 | 0.012 | 0.010 | 0.012 | 0.011 | +0.0% | OK |
| sonara | 1,152 | 2,995 | 0.034 | 0.031 | 0.035 | 0.033 | +2.9%¹ | OK |
| KGLite | 8,725 | 40,571 | 0.284 | 0.281 | 0.286 | 0.284 | +0.7% | OK |
| KGLite +tests | 15,129 | 71,821 | 0.459 | 0.455 | 0.460 | 0.442 | +0.2% | OK |
| mistral.rs | 12,122 | 52,088 | 0.368 | 0.358 | 0.376 | 0.357 | +2.2% | OK |
| mistral.rs +tests | 12,122 | 52,088 | 0.367 | 0.356 | 0.370 | 0.361 | +0.8% | OK |
| distillPDF | 13,555 | ~24.3k² | 0.466 | 0.443 | 0.459 | 0.451 | −1.5% | see ² |
| petekSuite | 14,848 | 41,786 | 0.504 | 0.500 | 0.508 | 0.499 | +0.8% | OK |

All deltas are within run-to-run jitter (the min-times frequently favor the
standalone side even where the median doesn't), with no consistent sign across
targets: **build-time parity**.

¹ Sub-40 ms workloads sit near the 1 ms reporting resolution of `build_secs`, so
single-millisecond jitter shows up as a large-looking percentage.

² **distillPDF exposed a pre-existing nondeterminism in the builder itself** —
inherited from kglite's in-tree `code_tree`, not a standalone divergence: two
consecutive runs then disagreed on total edge count (observed values 24,317 /
24,449 / 24,464 across runs), while `nodes` (13,555) and `resolved_edges`
(1,099) were stable. The root cause (randomized HashMap iteration over DEFINES
pairs) has since been **fixed** in codingest (BTreeMap + within-pair
consolidation). The edge count observed on distillPDF at the time was 24,317;
that number is **not** a pin and is not gated — distillPDF is an externally
owned checkout that has since been refactored and now yields 24,173 from the
same builder. Determinism is enforced instead by
`crates/codingest/tests/parity.rs::golden_parity`, which builds each committed
in-repo corpus three times and requires all three digests to match each other
and the frozen golden (`tests/corpus/dup_minified_assets` is the reproducer for
this exact bug). Every deterministic target above matched exactly on every run.

An earlier benchmark pass ran concurrently with a large release compile and
showed +2–5% deltas on the last two targets; the quiet-machine re-run above
erased them — worth remembering when reproducing these numbers.
