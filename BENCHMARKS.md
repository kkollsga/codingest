# code-tree benchmarks — us vs. the (former) in-tree component

> **Historical record (captured 2026-07-15).** KGLite removed its in-tree
> `kglite::code_tree` builder on 2026-07-16, so the cross-builder comparison
> below can no longer be reproduced — it is retained as the last-known-good
> parity/perf snapshot taken while the two builders were verified identical.
> The `codingest_bench` harness now builds the target **twice with the
> codingest builder** and checks query-result parity across the two builds
> (determinism); the "in-tree" columns are the historical authority side.

## Release 0.1.2 snapshot — 2026-07-22

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
consolidation); the canonical edge count is now a stable 24,317, pinned by the
release determinism gate. Every deterministic target above matched
exactly on every run.

An earlier benchmark pass ran concurrently with a large release compile and
showed +2–5% deltas on the last two targets; the quiet-machine re-run above
erased them — worth remembering when reproducing these numbers.
