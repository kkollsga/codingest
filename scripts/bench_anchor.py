#!/usr/bin/env python3
"""Release perf anchor — compare a `codingest_bench --json` capture to a
committed baseline and decide whether the release may proceed.

WHY THIS FILE EXISTS
--------------------
Release 0.1.6 published two perf breaches that nothing caught (see
`BENCHMARKS.md`, the 0.1.6 section): opencode nodes +13.03 % against a <=12 %
budget and build time +20.16 % against <=15 %. The budgets were not wrong so
much as *unattached* — they were written against a docs-off denominator and
then read against docs-on rows, by a human, at release time, from prose. A
budget that lives only in prose is not a gate.

Two lessons from that release shape everything here:

  1. **A raw per-query ceiling is unachievable by construction.** 0.1.6 was 6
     of 11 queries over a +10 % raw ceiling, worst +69.5 %, and every one of
     those was *correct*: the call graph got denser on purpose (CALLS edges
     +65.0 %), so the engine returned more rows for more time. Normalized per
     row returned, the same capture is flat. This comparator therefore judges
     queries on **ms per row**, never raw ms.

  2. **The comparison must refuse rather than mislead.** The 0.1.6 write-up had
     to open with "not comparable to the 0.1.5 section below" because the
     corpus digest changed underneath. A tool that silently prints a delta
     across two different corpora is worse than no tool. Digest mismatch and
     mode mismatch REFUSE here; they never produce a number.

WHAT IT IS AND IS NOT
---------------------
This is a **cumulative-drift** backstop, not a precision instrument. It
compares against a baseline roughly three releases back at +30 %, so it is
deliberately blind to a 5 % release-over-release slide and deliberately loud
about a 2x. Its job is to make "we shipped a 30 % regression over three
releases and nobody noticed" impossible. Fine-grained perf work is still
`codingest_bench` read by a human, recorded in `BENCHMARKS.md`.

THE FOUR VERDICTS (exit codes are the interface — see EXIT_* below)
-------------------------------------------------------------------
  REFUSE (3)  corpus_sha256 or include_docs differs from the baseline. No
              delta is computed or printed. Per the performance protocol, a
              number whose corpus digest differs from its comparand is not a
              number.
  VOID   (4)  the CONTROL query moved more than +/-15 % per row. The
              instrument moved, so the capture says nothing about the code.
              Re-measure; do NOT bisect, and do NOT read the other rows —
              this verdict deliberately reports no per-row verdicts at all.
  FAIL   (1)  a real trip: some row drifted past +30 %.
  PASS   (0)  nothing tripped.

WHY THE ANCHOR CORPUS MUST BE FROZEN
------------------------------------
The obvious corpus is this repo's own sources (`crates/codingest/src`, what
`make bench-smoke` uses). It is the wrong one, and the reason is structural
rather than a matter of taste: the anchor compares against a baseline roughly
three releases back, and `crates/codingest/src` **is the code under test**, so
almost every release changes it. A changed corpus changes `corpus_sha256`,
which correctly REFUSES — and a gate that REFUSES every release is a gate that
cannot fail. It also conflates the two things the gate must separate: "the
builder got slower" and "the builder has more source to parse".

The anchor therefore runs on `tests/corpus`, the frozen polyglot fixture tree
(60 files, TS/PY/RS/HTML plus `.md`/`.mdx`). It is committed, needs no network
or sibling checkout, and its digest moves only when someone deliberately
changes a fixture. It also makes the two docs modes genuinely independent
measurements — 279 nodes / 394 edges docs-on versus 272 / 374 docs-off —
which the all-Rust bench-smoke corpus could not do, having no docs at all.

WHY A TRIP NEEDS BOTH A RELATIVE **AND** AN ABSOLUTE MOVE
---------------------------------------------------------
Percentages lie about small numbers. Two back-to-back release-build captures
of `crates/codingest/src`, with nothing changed between them but the clock,
disagreed by +134 %, +106 % and +140 % on sub-millisecond cells (largest
absolute swing 0.165 ms). A bare +30 % relative gate fires on those every
release, and a gate that cries wolf is a gate someone switches off — the
failure mode this whole file exists to prevent.

So a row trips only when it is **both** >30 % per row **and** at least
`floors.query_abs_ms` slower in raw terms. The floor is recorded per baseline
rather than fixed here, because it is a property of the corpus: three runs per
mode on the frozen fixture corpus held every cell to a **0.004 ms** maximum
spread, so its floor is set at 0.010 ms — roughly 2.5x the observed noise.

The consequence is worth stating plainly rather than discovering later: the
floor sets a per-cell trip threshold that is *higher* than 30 % for the
smallest cells. On the fixture corpus a cell at 0.048 ms trips at +30 % as
intended, one at 0.026 ms needs ~+38 %, and one at 0.007 ms cannot trip below
+143 %. That is the honest reading of what a 1 us-resolution measurement can
support — where the cell's own noise exceeds the threshold, the right verdict
is *no verdict*, not a coin flip. It is the "judge a heavy-tailed cell by
median/mean, not min" doctrine applied at gate level.

**Build time does not gate on this corpus, and is recorded rather than
enforced.** At 0.006-0.025 s for 27 KB the measurement is dominated by
one-time grammar initialization — a once-per-event cost with no steady state
(hence `build_secs` being a mean, not a min). The build-side signal is carried
instead by node and edge counts, which are perfectly deterministic here (zero
spread across three runs) and are precisely what breached in 0.1.6.

CHOICE OF CONTROL
-----------------
The control is named in the baseline (`"control": true`), not hardcoded,
because the right control depends on the corpus — and this is not
hypothetical. `calls_edge_scan` is an excellent control on a large corpus
(1.6 % spread there, and `BENCHMARKS.md` calls it "the cleanest available
signal"), but on the fixture corpus it measures 0.007 ms and swings 14.3 %
run-to-run — under the 15 % void threshold by a hair, i.e. a control that
would void captures at random. Hardcoding it would have shipped exactly that.

The fixture corpus therefore designates `anchored_callers`: zero observed
spread across three runs in both modes, 7 rows (so per-row normalization is
actually exercised), and 13-14 us against a 1 us resolution.

The first and simplest query, `count_functions`, is rejected outright on both
corpora: it measures **0.000 ms**, below the recorded field's rounding
resolution. A control that cannot resolve its own value cannot detect that the
instrument moved, and 0.000 -> 0.001 is an infinite percentage.

The control is checked on the same per-row normalization as everything else,
deliberately: it must answer "did the machine change speed", not "did the
graph change shape". A builder change legitimately moves row counts, and that
must not void a capture.

Invoke either way:
    scripts/bench_anchor.sh compare --current C.json --baseline B.json
    import bench_anchor                     # from the unit test

Stdlib only, on purpose: the `release-gates` CI job installs pytest and
nothing else, and never builds the workspace, so this suite must run with no
cargo artifacts and no third-party imports.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Iterable, NamedTuple

# --------------------------------------------------------------------------
# Exit codes — this is the tool's contract with the release skill. Each
# non-zero code names a DIFFERENT operator action, which is the whole point of
# not collapsing them into a single 1: "re-measure" and "you have a
# regression" are opposite instructions.
# --------------------------------------------------------------------------
EXIT_PASS = 0
EXIT_FAIL = 1  # real drift; blocks the tag
EXIT_USAGE = 2  # bad arguments / unreadable input
EXIT_REFUSE = 3  # not comparable; no delta computed
EXIT_VOID = 4  # control moved; capture says nothing

# Thresholds. Defaults are the policy; the CLI can override so a test can
# drive a boundary without fabricating a huge fixture.
DRIFT_THRESHOLD_PCT = 30.0
CONTROL_THRESHOLD_PCT = 15.0

# Absolute floors, derived from measured run-to-run jitter (see module
# docstring). A row must clear its floor AND the relative threshold to trip.
#
# These are only the FALLBACKS. A baseline records its own `floors`, because
# the floor is a property of the corpus that was measured, not of this tool —
# the numbers here are sized for a millisecond-scale corpus and would make a
# microsecond-scale one ungateable. A baseline without floors gets these.
QUERY_ABS_FLOOR_MS = 0.25
BUILD_ABS_FLOOR_SECS = 0.05

SCHEMA_VERSION = 1


class Verdict(NamedTuple):
    """The outcome of one comparison.

    `status` is the machine-readable word, `code` the process exit code, and
    `lines` everything to print. `rows` carries the per-row detail so a test
    can assert on structure rather than on message text — and so a VOID can be
    asserted to carry NO rows.
    """

    status: str
    code: int
    lines: list[str]
    rows: list[dict[str, Any]]


# --------------------------------------------------------------------------
# Loading / normalizing
# --------------------------------------------------------------------------


def read_json(path: str | Path) -> Any:
    """Read a JSON file, raising ValueError with the path on any problem."""
    p = Path(path)
    try:
        return json.loads(p.read_text())
    except FileNotFoundError:
        raise ValueError(f"no such file: {p}") from None
    except json.JSONDecodeError as e:
        raise ValueError(f"{p}: not valid JSON: {e}") from None


def normalize_bench(obj: dict[str, Any]) -> dict[str, Any]:
    """Reduce a raw `codingest_bench --json` object to the baseline shape.

    codingest_bench runs TWO independent builds and reports `a_*` and `b_*`
    per query, which is a cross-build parity check, not two samples of one
    thing. Collapsing them needs a rule per quantity, and the rules differ:

      * per-query medians are a repeatable inner loop (WARMUP + ITERS), so the
        floor is the meaningful summary -> `min` of the two builds' medians.
      * build time is a ONCE-PER-EVENT cost. It has no steady state to find a
        floor of, so `min` over builds reports the luckiest moment of the
        machine -> `mean` of the two build times. (Performance protocol, the
        "trust min does not hold for a once-per-event cost" rule.)
    """
    missing = [
        k
        for k in ("path", "corpus_sha256", "include_docs", "nodes", "edges", "queries")
        if k not in obj
    ]
    if missing:
        raise ValueError(f"bench JSON is missing required field(s): {', '.join(missing)}")
    if obj["corpus_sha256"] is None:
        raise ValueError(
            "bench JSON has a null corpus_sha256 — the run did not resolve a "
            "tracked-only corpus, so it is not reproducible and must not be "
            "recorded as a baseline or compared against one"
        )

    build_a = float(obj.get("build_a_secs", 0.0))
    build_b = float(obj.get("build_b_secs", 0.0))

    queries = []
    for q in obj["queries"]:
        queries.append(
            {
                "name": q["name"],
                "rows": int(q["rows"]),
                "median_ms": min(float(q["a_median_ms"]), float(q["b_median_ms"])),
                "min_ms": min(float(q["a_min_ms"]), float(q["b_min_ms"])),
            }
        )

    return {
        "target": obj["path"],
        "corpus_sha256": obj["corpus_sha256"],
        "include_docs": bool(obj["include_docs"]),
        "corpus_files": obj.get("corpus_files"),
        "corpus_bytes": obj.get("corpus_bytes"),
        "nodes": int(obj["nodes"]),
        "edges": int(obj["edges"]),
        "build_secs": round((build_a + build_b) / 2.0, 4),
        "queries": queries,
    }


def select_corpus_entry(
    baseline: dict[str, Any], target: str, include_docs: bool
) -> dict[str, Any] | None:
    """Find the baseline entry for the same target AND docs mode, or None.

    Keyed on both because a baseline may record more than one corpus, and a
    number from one corpus read against another is exactly what the digest
    guard exists to prevent — matching on mode alone would let two different
    targets meet if their digests ever coincided.
    """
    for entry in baseline.get("corpora", []):
        if (
            entry.get("target") == target
            and bool(entry.get("include_docs")) == include_docs
        ):
            return entry
    return None


def control_name(entry: dict[str, Any]) -> str | None:
    """The query the baseline designates as its CONTROL, if any."""
    for q in entry.get("queries", []):
        if q.get("control"):
            return q["name"]
    return None


# --------------------------------------------------------------------------
# The comparison itself
# --------------------------------------------------------------------------


def per_row_ms(median_ms: float, rows: int) -> tuple[float, bool]:
    """Per-row cost, and whether normalization actually applied.

    A query returning 0 rows cannot be normalized. Rather than divide by zero
    or silently substitute 1, fall back to the raw median and say so, so the
    report never presents an un-normalized number as a normalized one.
    """
    if rows <= 0:
        return median_ms, False
    return median_ms / rows, True


def pct_change(base: float, cur: float) -> float | None:
    """Percent change, or None when the baseline is zero (no ratio exists)."""
    if base == 0.0:
        return None
    return (cur - base) / base * 100.0


def _mode(include_docs: bool) -> str:
    return "docs-on" if include_docs else "docs-off"


def compare(
    current: dict[str, Any],
    baseline: dict[str, Any],
    *,
    baseline_label: str = "<baseline>",
    drift_pct: float = DRIFT_THRESHOLD_PCT,
    control_pct: float = CONTROL_THRESHOLD_PCT,
    query_floor_ms: float = QUERY_ABS_FLOOR_MS,
    build_floor_secs: float = BUILD_ABS_FLOOR_SECS,
) -> Verdict:
    """Compare a normalized current capture against a whole baseline file.

    `current` is the output of `normalize_bench`. `baseline` is the parsed
    baseline document (with its `corpora` list); the matching docs mode is
    selected here so that a missing mode is a REFUSE rather than a crash.
    """
    lines: list[str] = []
    mode = _mode(current["include_docs"])
    target = current["target"]
    lines.append(
        f"bench anchor: current {target} {mode} capture vs baseline "
        f"{baseline_label} (version {baseline.get('version', '?')})"
    )

    # ---- (b) target / mode mismatch -> REFUSE ---------------------------
    # Checked before the digest so the message can name the real problem: with
    # no entry for this pair there is no digest to compare against either.
    entry = select_corpus_entry(baseline, target, current["include_docs"])
    if entry is None:
        have = ", ".join(
            sorted(
                f"{e.get('target')} {_mode(bool(e.get('include_docs')))}"
                for e in baseline.get("corpora", [])
            )
        )
        lines.append(
            f"REFUSE: baseline {baseline_label} records no {target} {mode} "
            f"capture (it has: {have or 'nothing'}). Budgets written for one "
            f"docs mode and read against the other is exactly the 0.1.6 breach "
            f"recorded in BENCHMARKS.md; this tool will not do it. Re-run "
            f"codingest_bench against {target} in the {mode} mode, or add that "
            f"pair to the baseline."
        )
        return Verdict("REFUSE", EXIT_REFUSE, lines, [])

    # Noise floors are recorded PER BASELINE, because they are a property of
    # the corpus, not of the tool: the same +30% means different things on a
    # 27 KB fixture corpus and on a 127 MB one. An explicit CLI override still
    # wins, so a test can drive a boundary exactly.
    floors = entry.get("floors", {})
    if query_floor_ms == QUERY_ABS_FLOOR_MS:
        query_floor_ms = float(floors.get("query_abs_ms", query_floor_ms))
    if build_floor_secs == BUILD_ABS_FLOOR_SECS:
        build_floor_secs = float(floors.get("build_abs_secs", build_floor_secs))

    # ---- (a) corpus digest mismatch -> REFUSE ---------------------------
    if entry.get("corpus_sha256") != current["corpus_sha256"]:
        lines.append(
            "REFUSE: corpus_sha256 differs — these two captures did not measure "
            "the same bytes, so no delta between them means anything."
        )
        lines.append(f"  baseline : {entry.get('corpus_sha256')}")
        lines.append(f"  current  : {current['corpus_sha256']}")
        lines.append(
            "  The bench corpus is the target's git-tracked files; it moves "
            "when those files change. Capture a fresh baseline for the new "
            "corpus (see tests/benchmarks/README.md) rather than reading a "
            "cross-corpus number."
        )
        return Verdict("REFUSE", EXIT_REFUSE, lines, [])

    lines.append(f"corpus {current['corpus_sha256'][:16]}… ({mode}) — comparable")

    base_q = {q["name"]: q for q in entry.get("queries", [])}
    cur_q = {q["name"]: q for q in current["queries"]}

    # ---- (c) CONTROL moved -> VOID --------------------------------------
    # Deliberately BEFORE any per-row verdict, and returning none of them: if
    # the instrument moved, the other rows are not evidence, and publishing
    # them invites someone to bisect a machine hiccup.
    ctl = control_name(entry)
    if ctl is None:
        lines.append(
            f"REFUSE: baseline {baseline_label} designates no CONTROL query "
            f'(no entry with "control": true) for the {mode} capture. Without '
            f"a control there is nothing to distinguish a code regression from "
            f"a machine that got slower, so no verdict is available."
        )
        return Verdict("REFUSE", EXIT_REFUSE, lines, [])
    if ctl not in cur_q:
        lines.append(
            f"REFUSE: the baseline's CONTROL query {ctl!r} is absent from the "
            f"current capture. The query set changed; recapture the baseline."
        )
        return Verdict("REFUSE", EXIT_REFUSE, lines, [])

    cb, cc = base_q[ctl], cur_q[ctl]
    cb_pr, _ = per_row_ms(float(cb["median_ms"]), int(cb["rows"]))
    cc_pr, _ = per_row_ms(float(cc["median_ms"]), int(cc["rows"]))
    ctl_delta = pct_change(cb_pr, cc_pr)
    if ctl_delta is None:
        lines.append(
            f"VOID: CONTROL {ctl!r} has a zero baseline per-row cost, so no "
            f"movement can be measured against it. Re-capture the baseline "
            f"with a control that resolves above the measurement floor."
        )
        return Verdict("VOID", EXIT_VOID, lines, [])
    ctl_abs_ms = float(cc["median_ms"]) - float(cb["median_ms"])
    if abs(ctl_delta) > control_pct:
        if abs(ctl_abs_ms) >= query_floor_ms:
            lines.append(
                f"VOID: CONTROL {ctl!r} moved {ctl_delta:+.2f}% per row "
                f"({cb_pr:.4f} -> {cc_pr:.4f} ms/row, {ctl_abs_ms:+.3f} ms raw), "
                f"past the +/-{control_pct:.0f}% void threshold."
            )
            lines.append(
                "  The instrument moved, not necessarily the code. RE-MEASURE on a "
                "settled machine. Do NOT bisect and do NOT read the other rows — "
                "this capture reports none, on purpose."
            )
            return Verdict("VOID", EXIT_VOID, lines, [])
        # Same standard as a trip row (module docstring: a sub-floor movement
        # "must not void a capture"): below floors.query_abs_ms the ratio is
        # sub-resolution jitter, not evidence the machine changed speed.
        lines.append(
            f"CONTROL {ctl}: {ctl_delta:+.2f}% per row but only "
            f"{ctl_abs_ms:+.3f} ms raw — under the {query_floor_ms} ms floor, "
            f"sub-floor jitter; instrument treated as steady"
        )
    else:
        lines.append(
            f"CONTROL {ctl}: {ctl_delta:+.2f}% per row — instrument steady"
        )

    # ---- (d) per-row drift, build time, node/edge counts ----------------
    rows: list[dict[str, Any]] = []
    trips: list[str] = []

    def record(
        kind: str,
        name: str,
        base_v: float,
        cur_v: float,
        delta: float | None,
        tripped: bool,
        note: str = "",
    ) -> None:
        rows.append(
            {
                "kind": kind,
                "name": name,
                "baseline": base_v,
                "current": cur_v,
                "delta_pct": delta,
                "tripped": tripped,
                "note": note,
            }
        )

    # Counts and build time: compared raw. The corpus is pinned by digest, so
    # growth here is the builder doing more work — which is exactly the
    # cumulative signal the 0.1.6 node-count breach needed and did not have.
    for kind, key, floor, unit in (
        ("count", "nodes", 0.0, ""),
        ("count", "edges", 0.0, ""),
        ("build", "build_secs", build_floor_secs, " s"),
    ):
        bv = float(entry.get(key, 0.0))
        cv = float(current[key])
        d = pct_change(bv, cv)
        note = ""
        tripped = False
        if d is None:
            note = "zero baseline — no ratio"
        elif d > drift_pct:
            if (cv - bv) >= floor:
                tripped = True
            else:
                note = f"over {drift_pct:.0f}% but under the {floor}{unit} absolute floor"
        record(kind, key, bv, cv, d, tripped, note)
        if tripped:
            trips.append(f"{key} {d:+.2f}% ({bv:g}{unit} -> {cv:g}{unit})")

    # Queries: per-row normalized. This is the 0.1.6 lesson made mechanical —
    # more rows for more time is not a regression.
    for name, b in base_q.items():
        c = cur_q.get(name)
        if c is None:
            record("query", name, float(b["median_ms"]), float("nan"), None, False, "absent from current capture")
            continue
        b_pr, b_norm = per_row_ms(float(b["median_ms"]), int(b["rows"]))
        c_pr, c_norm = per_row_ms(float(c["median_ms"]), int(c["rows"]))
        d = pct_change(b_pr, c_pr)
        abs_ms = float(c["median_ms"]) - float(b["median_ms"])
        note = "" if (b_norm and c_norm) else "0 rows — raw median, not normalized"
        tripped = False
        if d is None:
            note = (note + "; " if note else "") + "zero baseline — no ratio"
        elif d > drift_pct:
            if abs_ms >= query_floor_ms:
                tripped = True
            else:
                note = (note + "; " if note else "") + (
                    f"over {drift_pct:.0f}% per row but only {abs_ms:+.3f} ms raw — "
                    f"under the {query_floor_ms} ms floor, within measured jitter"
                )
        record("query", name, b_pr, c_pr, d, tripped, note)
        if tripped:
            trips.append(
                f"{name} {d:+.2f}% per row ({b_pr:.4f} -> {c_pr:.4f} ms/row, "
                f"{abs_ms:+.3f} ms raw)"
            )

    for r in rows:
        d = "  n/a  " if r["delta_pct"] is None else f"{r['delta_pct']:+7.2f}%"
        flag = "TRIP" if r["tripped"] else "ok  "
        extra = f"  [{r['note']}]" if r["note"] else ""
        lines.append(f"  {flag} {d}  {r['name']}{extra}")

    if trips:
        lines.append(
            f"FAIL: {len(trips)} row(s) drifted past +{drift_pct:.0f}% against "
            f"baseline {baseline_label}:"
        )
        for t in trips:
            lines.append(f"  - {t}")
        lines.append(
            "  This blocks the release tag. Either the regression is real and "
            "gets fixed, or the growth is intended and gets argued in "
            "BENCHMARKS.md and re-baselined in the same commit — never "
            "re-baselined to silence an unexplained diff."
        )
        return Verdict("FAIL", EXIT_FAIL, lines, rows)

    lines.append(
        f"PASS: no row past +{drift_pct:.0f}% vs baseline {baseline_label}. "
        f"Cumulative drift is within budget."
    )
    return Verdict("PASS", EXIT_PASS, lines, rows)


# --------------------------------------------------------------------------
# Baseline selection / pruning
# --------------------------------------------------------------------------


def parse_version(text: str) -> tuple[int, ...]:
    """`"0.1.7"` -> `(0, 1, 7)`. Raises ValueError on anything else."""
    parts = text.split(".")
    if not parts or not all(p.isdigit() for p in parts):
        raise ValueError(f"not a numeric dotted version: {text!r}")
    return tuple(int(p) for p in parts)


def baseline_versions(paths: Iterable[Path]) -> list[tuple[tuple[int, ...], Path]]:
    """Sorted (version, path) for every `<version>.json` in `paths`."""
    out = []
    for p in paths:
        try:
            out.append((parse_version(p.stem), p))
        except ValueError:
            continue
    return sorted(out)


def select_anchor(directory: str | Path, window: int = 3) -> Path | None:
    """The baseline to compare against: the OLDEST within the last `window`.

    Comparing against the immediately preceding release would only ever see
    one release of drift, which is precisely how 0.1.6's slide went unnoticed
    — each step looked fine. Anchoring `window` releases back is what makes
    the gate cumulative.

    With fewer than `window` baselines on disk this returns the oldest one
    available. That is a LIVE BUT DEGRADED window: the gate works, it just
    spans less history than intended, and it tightens automatically as
    baselines accumulate.
    """
    found = baseline_versions(sorted(Path(directory).glob("*.json")))
    if not found:
        return None
    return found[-window:][0][1]


def prunable(directory: str | Path, keep: int = 4) -> list[Path]:
    """Baselines beyond the newest `keep` — safe to delete after a release."""
    found = baseline_versions(sorted(Path(directory).glob("*.json")))
    return [p for _, p in found[:-keep]] if len(found) > keep else []


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def cmd_compare(args: argparse.Namespace) -> int:
    try:
        current = normalize_bench(read_json(args.current))
        baseline = read_json(args.baseline)
    except (ValueError, KeyError, TypeError) as e:
        print(f"::error::bench_anchor: {e}", file=sys.stderr)
        return EXIT_USAGE

    if baseline.get("schema") != SCHEMA_VERSION:
        print(
            f"::error::bench_anchor: baseline {args.baseline} has schema "
            f"{baseline.get('schema')!r}, expected {SCHEMA_VERSION} — refusing to "
            f"guess at a layout it may not have.",
            file=sys.stderr,
        )
        return EXIT_USAGE

    v = compare(
        current,
        baseline,
        baseline_label=Path(args.baseline).name,
        drift_pct=args.drift_pct,
        control_pct=args.control_pct,
        query_floor_ms=args.query_floor_ms,
        build_floor_secs=args.build_floor_secs,
    )
    stream = sys.stdout if v.code == EXIT_PASS else sys.stderr
    for line in v.lines:
        print(line, file=stream)
    return v.code


def cmd_select_baseline(args: argparse.Namespace) -> int:
    chosen = select_anchor(args.directory, window=args.window)
    if chosen is None:
        print(
            f"::error::bench_anchor: no baseline files in {args.directory}",
            file=sys.stderr,
        )
        return EXIT_USAGE
    n = len(baseline_versions(sorted(Path(args.directory).glob("*.json"))))
    if n < args.window:
        print(
            f"bench_anchor: only {n} baseline(s) on disk, window is "
            f"{args.window} — comparing against {chosen.name}. The gate is "
            f"LIVE BUT DEGRADED: it spans {n} release(s) of history, not "
            f"{args.window}. It tightens on its own as baselines accumulate.",
            file=sys.stderr,
        )
    print(chosen)
    return EXIT_PASS


def cmd_prune(args: argparse.Namespace) -> int:
    stale = prunable(args.directory, keep=args.keep)
    for p in stale:
        if args.delete:
            p.unlink()
        print(p)
    return EXIT_PASS


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(prog="bench_anchor", description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("compare", help="compare a bench capture to a baseline")
    c.add_argument("--current", required=True, help="codingest_bench --json output")
    c.add_argument("--baseline", required=True, help="tests/benchmarks/baselines/<v>.json")
    c.add_argument("--drift-pct", type=float, default=DRIFT_THRESHOLD_PCT)
    c.add_argument("--control-pct", type=float, default=CONTROL_THRESHOLD_PCT)
    c.add_argument("--query-floor-ms", type=float, default=QUERY_ABS_FLOOR_MS)
    c.add_argument("--build-floor-secs", type=float, default=BUILD_ABS_FLOOR_SECS)
    c.set_defaults(func=cmd_compare)

    s = sub.add_parser("select-baseline", help="print the baseline to anchor against")
    s.add_argument("directory")
    s.add_argument("--window", type=int, default=3)
    s.set_defaults(func=cmd_select_baseline)

    p = sub.add_parser("prune", help="list (or delete) baselines beyond --keep")
    p.add_argument("directory")
    p.add_argument("--keep", type=int, default=4)
    p.add_argument("--delete", action="store_true")
    p.set_defaults(func=cmd_prune)
    return ap


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    sys.exit(main())
