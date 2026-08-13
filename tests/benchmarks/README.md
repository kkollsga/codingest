# Release perf anchor — committed bench baselines

`baselines/<version>.json` is the committed record of how fast codingest was at
each release. `scripts/bench_anchor.sh` compares a fresh capture against one of
them and blocks the release tag on cumulative drift.

## Why this exists

Release 0.1.6 published two perf breaches that nothing caught (`BENCHMARKS.md`,
the 0.1.6 section): opencode nodes **+13.03 %** against a ≤12 % budget and build
time **+20.16 %** against ≤15 %. The budgets were not wrong so much as
*unattached* — written against a docs-off denominator, then read against
docs-on rows, by a human, from prose. A budget that lives only in prose is not
a gate.

This is a **cumulative-drift backstop**, not a precision instrument. It anchors
~3 releases back at **+30 %**, so it is deliberately blind to a 5 %
release-over-release slide and deliberately loud about a 2×. Fine-grained perf
work is still `codingest_bench` read by a human and recorded in
`BENCHMARKS.md`.

## The four verdicts

| exit | verdict | meaning | what you do |
|---|---|---|---|
| 0 | PASS | nothing drifted | proceed |
| 1 | FAIL | a row drifted past +30 % | **blocks the tag** — fix it, or argue the growth in `BENCHMARKS.md` and re-baseline in the same commit |
| 2 | USAGE | bad arguments / unreadable input | fix the invocation |
| 3 | REFUSE | corpus digest or docs mode differs | **no delta is computed** — recapture, never read across |
| 4 | VOID | the control query moved >15 % | re-measure on a settled machine; do **not** bisect |

REFUSE and VOID are separate codes on purpose: "re-measure" and "you have a
regression" are opposite instructions, and collapsing them into one failure is
how an operator learns to ignore the gate.

## What is compared, and how

- **Queries: per row returned (`median_ms / rows`), never raw ms.** 0.1.6 was 6
  of 11 queries over a raw +10 % ceiling, worst +69.5 %, and every one was
  *correct* — the call graph got denser on purpose (CALLS edges +65 %).
  Normalized per row, that same capture is flat. A gate that fires on intended
  growth is a gate someone switches off.
- **Node count, edge count, build time: raw.** The corpus is pinned by digest,
  so growth here is the builder doing more work per byte — exactly the signal
  the 0.1.6 node-count breach needed and did not have.
- **A row trips only if it is both >30 % *and* past an absolute floor.**
  Percentages lie about small numbers: two back-to-back captures of
  `crates/codingest/src`, with nothing changed between them but the clock,
  disagreed by **+134 %, +106 % and +140 %** on sub-millisecond cells. Without
  a floor this gate would be red every release, and a gate that cries wolf
  gets switched off.

  The floor is recorded **per baseline** (`floors`), not fixed in the tool,
  because it is a property of the corpus measured. Three runs per mode on the
  fixture corpus held every cell to a **0.004 ms** spread, so its floor is
  `0.010 ms` — about 2.5× the observed noise. Each baseline records the jitter
  it was captured with under `stability_across_3_runs`.

  The consequence, stated plainly: the floor makes the effective trip
  threshold *higher* than 30 % for the smallest cells. A 0.048 ms cell trips
  at +30 % as intended; a 0.026 ms cell needs ~+38 %; a 0.007 ms cell cannot
  trip below +143 %. That is the honest reading of what a 1 µs-resolution
  measurement supports — where a cell's own noise exceeds the threshold, the
  right verdict is *no verdict*.

- **Build time is recorded but does not gate on this corpus.** At 27 KB the
  build (0.006–0.025 s) is dominated by one-time grammar initialization — a
  once-per-event cost with no steady state, which is also why `build_secs` is
  a mean rather than a min. The build-side signal is carried by node and edge
  counts, which are perfectly deterministic here (zero spread across three
  runs) and are exactly what breached in 0.1.6. `build_gates: false` records
  this rather than leaving it to be rediscovered.

## Why the corpus is frozen, and is not this repo's own source

The obvious corpus is `crates/codingest/src` (what `make bench-smoke` uses).
It is the wrong one, for a structural reason:

- It **is the code under test**, so almost every release changes it. A changed
  corpus changes `corpus_sha256`, which correctly REFUSEs — and a gate that
  REFUSEs every release is a gate that cannot fail.
- It conflates the two things the anchor must separate: *the builder got
  slower* and *the builder has more source to parse*.
- It is all Rust and contains **no docs at all**, so docs-on and docs-off
  produce byte-identical graphs there and the second capture proves nothing.

`tests/corpus` — the frozen polyglot fixture tree, 60 files of TS/PY/RS/HTML
plus `.md`/`.mdx` — has none of those problems. It is committed, needs no
network or sibling checkout, and its digest moves only when someone
deliberately changes a fixture. Its two docs modes are genuinely independent
measurements: **279 nodes / 394 edges** docs-on versus **272 / 374** docs-off.

Its one real cost is scale: at 27 KB the absolute timings are small, which the
per-baseline floor accounts for. `test_committed_baseline_gate_is_not_vacuous`
asserts the gate can still trip there — doubling the slowest query in either
mode must FAIL — so "too small to gate" cannot creep in unnoticed.

## The control query

Each baseline names its own control (`"control": true`) rather than the tool
hardcoding one. If the control moves more than ±15 % **per row**, the capture
is VOID and no other row is even reported — if the instrument moved, the other
rows are not evidence.

Naming it per baseline is not ceremony — the right control genuinely differs
by corpus. `calls_edge_scan` is an excellent control on a large corpus (1.6 %
spread, and `BENCHMARKS.md` calls it "the cleanest available signal"), but on
the fixture corpus it measures 0.007 ms and swings **14.3 %** run-to-run,
under the 15 % void threshold by a hair — a control that would void captures
at random. Hardcoding it would have shipped exactly that.

The fixture corpus therefore designates **`varlen_callers_1_3`**: 28 µs against
floors of 5–7.5 µs, so **3.7–5.6× margin**, and measured flat (+2.2 %, +3.0 %)
across the kglite 0.15.11 → 0.15.13 engine move — the exact change that
disqualified its predecessor.

**It used to designate `anchored_callers`, and that choice failed twice over.**
It was picked for a good-looking reason — zero observed spread across three runs
in both modes, 7 rows so per-row normalization is exercised — but at 13–14 µs
against a 10–13 µs floor it only ever had ~1.0× margin. Two independent things
then went wrong:

- **The margin ran out.** The 0.2.0 capture landed at 12 µs against a 13 µs
  floor, so `test_committed_baseline_is_well_formed` went red and stayed red
  through the 0.2.0 release.
- **The premise expired.** A control is chosen because the work under test
  cannot touch it — but the thing that moved was a **dependency**, not our
  source. kglite 0.15.13's planner fix made that exact query 5.7× faster
  (4 µs on this corpus now), so docs-off VOIDed three times running while
  docs-on returned PASS purely because its raw delta fell 0.001 ms under the
  jitter floor. Two uninformative verdicts, neither legible as such.

So a control now has two requirements, both enforced:

1. **≥2× its own noise floor** — checked by
   `test_committed_baseline_is_well_formed` against `CONTROL_FLOOR_MARGIN`.
   `>= floor` is a tie, and a tie is one noise tick from measuring nothing.
2. **Measured stable across the last dependency move.** Prefer the slowest
   query that did not move; a query the upstream release notes describe as
   improved is disqualified on sight. This one cannot be asserted in a unit
   test — it is a judgement made when the control is chosen, and it is why the
   control is named per baseline rather than hardcoded.

**A control that moves *deterministically* is not the instrument wandering.**
VOID says "re-measure on a settled machine", but a repeated re-measure that
returns the same delta every time has refuted that reading: the control's
premise is void and the gate needs a new control. Three identical VOIDs is a
finding about the gate, not about the machine.

The first and simplest query, `count_functions`, is **rejected on every
corpus**: it measures 0.000 ms, below the recorded field's rounding
resolution. A control that cannot resolve its own value cannot detect that the
instrument moved, and 0.000 → 0.001 is an infinite percentage.
`test_committed_baseline_is_well_formed` enforces that a control sits above
its own noise floor.

The control is checked on the **same per-row normalization** as everything
else, deliberately: it must answer "did the machine change speed", not "did the
graph change shape". A builder change legitimately moves row counts and must
not void a capture.

## Capturing a new baseline

At release time, from a **release build** (a debug build's numbers are not
comparable to anything here):

```sh
cargo build --release -p codingest --bin codingest_bench
# discard a warmup run per mode — see below, this is not optional
./target/release/codingest_bench tests/corpus --json           > /dev/null 2>&1
for i in 1 2 3; do
  ./target/release/codingest_bench tests/corpus --json         > /tmp/on$i.json  2>/dev/null
done
./target/release/codingest_bench tests/corpus --no-docs --json > /dev/null 2>&1
for i in 1 2 3; do
  ./target/release/codingest_bench tests/corpus --no-docs --json > /tmp/off$i.json 2>/dev/null
done
```

**Discard a warmup run per mode.** The first run after a build reads high in
*every cell at once* — measured 2026-08-13 over six runs: `top20` 0.054 then
0.043/0.044/0.043/0.045/0.044, `varlen` 0.034 then 0.028/0.028/0.029/0.029/0.029,
with `defs_per_file`, `calls_edge_scan` and `reverse_callees_of_hub` all showing
the same one-run step. A whole-capture step is a one-time cost (page-cache fill
for the fixture tree), not per-cell noise, and folding it into the spread
inflated the 2.5× floor from 0.005 to 0.0275 — high enough that **no query on
this corpus could clear the 2× control margin**, i.e. the un-warmed procedure
made a well-formed baseline unconstructible. This is the same shape as the
"trust min" caveat: a once-per-event cost has no steady state to find a floor
of.

Run it **three times per mode after the warmup** and confirm the runs agree
within noise before recording — if they do not agree, find out why rather than
averaging over it. Store the **min** per query (a repeatable inner loop has a
floor worth finding) and the **mean** for `build_secs` (a once-per-build cost
does not), and record the worst spread under `stability_across_3_runs`. Set
`floors.query_abs_ms` to roughly 2.5× the worst observed spread — then check the
designated control still clears **2×** that floor; if it does not, the control
is wrong, not the floor. A capture whose `corpus_sha256` is null is not
tracked-only, is not reproducible, and is rejected by the tool.

Note that `codingest_bench` writes warnings to stderr before the JSON, so
redirect the two streams separately — `> out.json 2>/dev/null`, never `2>&1`,
which produces a file that is not JSON at all.

Name the file for the version being released (`0.1.7.json`) and set
`captured_at_commit` to the full SHA it was taken from.

## Retention

`scripts/bench_anchor.sh select-baseline baselines --window 3` picks the anchor:
the **oldest** baseline within the last 3 releases. Anchoring to the previous
release would only ever see one step of drift — which is exactly how 0.1.6's
slide went unnoticed, each step looking fine on its own.

With fewer than 3 baselines on disk the tool anchors to the oldest available
and says so on stderr: the gate is **live but degraded**, spanning less history
than intended, and it tightens on its own as baselines accumulate. As of 0.1.7
there is exactly one baseline, so the window is degraded to one release.

`scripts/bench_anchor.sh prune baselines --keep 4 --delete` drops baselines
beyond the newest four.
