"""Unit tests for the release perf anchor — `scripts/bench_anchor.{sh,py}`.

WHY THIS SUITE EXISTS
---------------------
The anchor runs only at release time, from the `release` skill, against a
release build. That is the same "can never be exercised by branch CI" problem
`scripts/release_gates.sh` was extracted to solve, and it gets the same answer:
the logic lives in a script that this suite drives through every verdict on
every push (the `release-gates` job in `.github/workflows/ci.yml` runs
`pytest tests/release`, so this file rides that job with no workflow change).

A perf gate is unusually easy to be born dead, in ways specific to it:

  1. **It can be unfailable.** A comparator that swallows its own non-zero
     status, or whose threshold nothing can exceed, passes forever and looks
     healthy. Every verdict here is asserted through the real script's real
     exit code.
  2. **It can be un-passable**, which is worse, because the fix is to switch it
     off. `test_measured_jitter_does_not_trip` pins the actual measured
     run-to-run noise of this repo's bench corpus and asserts it stays quiet.
  3. **It can measure the wrong thing.** `test_more_rows_for_more_time_*` is
     the 0.1.6 lesson made executable: a query that returns 2x the rows in 2x
     the time is FLAT per row and must not trip.

That third one is why this gate exists at all — see `BENCHMARKS.md` §0.1.6 and
the `scripts/bench_anchor.py` module docstring.

The suite needs no cargo artifacts and no third-party imports beyond pytest,
because the `release-gates` job installs pytest and never builds the
workspace. `test_module_is_stdlib_only` enforces that so a future import
cannot silently break that job.
"""

from __future__ import annotations

import ast
import json
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "bench_anchor.sh"
MODULE = REPO / "scripts" / "bench_anchor.py"
BASELINE_DIR = REPO / "tests" / "benchmarks" / "baselines"
BENCH_RS = REPO / "crates" / "codingest" / "src" / "bin" / "codingest_bench.rs"

sys.path.insert(0, str(REPO / "scripts"))
import bench_anchor as ba  # noqa: E402

CONTROL = "calls_edge_scan"
DIGEST = "a" * 64
TARGET = "tests/corpus"


# --------------------------------------------------------------------------
# fixtures — synthetic, so a threshold is exercised exactly and not
# approximately. Shaped exactly like real `codingest_bench --json` output.
# --------------------------------------------------------------------------


def make_current(
    *,
    target: str = TARGET,
    digest: str = DIGEST,
    include_docs: bool = True,
    nodes: int = 1000,
    edges: int = 2000,
    build: float = 0.500,
    control_ms: float = 1.000,
    slow_ms: float = 2.000,
    slow_rows: int = 10,
) -> dict:
    """A raw bench capture. `a_*` and `b_*` are the two independent builds."""

    def q(name, rows, ms):
        return {
            "name": name,
            "rows": rows,
            "parity": True,
            "a_median_ms": ms,
            "a_min_ms": ms,
            "b_median_ms": ms,
            "b_min_ms": ms,
        }

    return {
        "path": target,
        "corpus_mode": "tracked-only",
        "corpus_sha256": digest,
        "corpus_files": 39,
        "corpus_bytes": 100000,
        "include_docs": include_docs,
        "nodes": nodes,
        "edges": edges,
        "build_a_secs": build,
        "build_b_secs": build,
        "queries": [q(CONTROL, 1, control_ms), q("slow_one", slow_rows, slow_ms)],
    }


def make_baseline(
    *,
    target: str = TARGET,
    digest: str = DIGEST,
    include_docs: bool = True,
    control: bool = True,
    control_ms: float = 1.000,
    floors: dict | None = None,
) -> dict:
    return {
        "schema": 1,
        "version": "0.1.0",
        "captured_at_commit": "0" * 40,
        "corpora": [
            {
                "target": target,
                "mode": "docs-on" if include_docs else "docs-off",
                "include_docs": include_docs,
                **({"floors": floors} if floors else {}),
                "corpus_sha256": digest,
                "nodes": 1000,
                "edges": 2000,
                "build_secs": 0.500,
                "queries": [
                    {
                        "name": CONTROL,
                        **({"control": True} if control else {}),
                        "rows": 1,
                        "median_ms": control_ms,
                        "min_ms": control_ms,
                    },
                    {"name": "slow_one", "rows": 10, "median_ms": 2.000, "min_ms": 2.000},
                ],
            }
        ],
    }


def write(tmp_path: Path, name: str, obj: dict) -> Path:
    p = tmp_path / name
    p.write_text(json.dumps(obj))
    return p


def run_anchor(tmp_path: Path, current: dict, baseline: dict, *args: str):
    """Drive the REAL shell entry point, so exit codes are the shipped ones."""
    cur = write(tmp_path, "current.json", current)
    base = write(tmp_path, "baseline.json", baseline)
    return subprocess.run(
        [str(SCRIPT), "compare", "--current", str(cur), "--baseline", str(base), *args],
        capture_output=True,
        text=True,
        cwd=str(REPO),
    )


def verdict(current: dict, baseline: dict, **kw) -> ba.Verdict:
    """Call the comparison logic directly, for structural assertions."""
    return ba.compare(ba.normalize_bench(current), baseline, **kw)


# --------------------------------------------------------------------------
# the four verdicts, through the real script
# --------------------------------------------------------------------------


def test_identity_passes(tmp_path):
    r = run_anchor(tmp_path, make_current(), make_baseline())
    assert r.returncode == ba.EXIT_PASS, r.stderr
    assert "PASS" in r.stdout


def test_two_times_slowdown_trips(tmp_path):
    """The headline gate: 2x on a row above the noise floor blocks the tag."""
    r = run_anchor(tmp_path, make_current(slow_ms=4.000), make_baseline())
    assert r.returncode == ba.EXIT_FAIL
    assert "FAIL" in r.stderr
    assert "slow_one" in r.stderr
    assert "+100.00%" in r.stderr


def test_digest_mismatch_refuses_and_prints_no_delta(tmp_path):
    """A cross-corpus number is not a number — so none may be printed."""
    r = run_anchor(tmp_path, make_current(digest="b" * 64, slow_ms=99.0), make_baseline())
    assert r.returncode == ba.EXIT_REFUSE
    assert "REFUSE" in r.stderr
    # The 99ms row would be a screaming +4850% if it were ever compared.
    assert "%" not in r.stderr.split("REFUSE")[1].split("baseline :")[0]
    assert "slow_one" not in r.stderr


def test_mode_mismatch_refuses(tmp_path):
    """docs-off measured against a docs-on baseline is the 0.1.6 breach."""
    r = run_anchor(tmp_path, make_current(include_docs=False), make_baseline(include_docs=True))
    assert r.returncode == ba.EXIT_REFUSE
    assert "docs-off" in r.stderr


def test_target_mismatch_refuses(tmp_path):
    """A number from one corpus read against another is the thing the digest
    guard exists to prevent; matching on docs mode alone would let two
    different targets meet."""
    r = run_anchor(
        tmp_path,
        make_current(target="crates/codingest/src"),
        make_baseline(target="tests/corpus"),
    )
    assert r.returncode == ba.EXIT_REFUSE
    assert "crates/codingest/src" in r.stderr


def test_control_move_voids(tmp_path):
    r = run_anchor(tmp_path, make_current(control_ms=1.500), make_baseline())
    assert r.returncode == ba.EXIT_VOID
    assert "VOID" in r.stderr
    assert "re-measure" in r.stderr.lower()
    assert "bisect" in r.stderr.lower()


def test_void_reports_no_per_row_verdicts():
    """A void capture is not evidence, so it must not publish row verdicts.

    The fixture moves the control AND doubles a row that would otherwise trip:
    if the void did not short-circuit, `slow_one` would appear as a FAIL and
    somebody would go bisect a machine hiccup.
    """
    v = verdict(make_current(control_ms=1.500, slow_ms=4.000), make_baseline())
    assert v.status == "VOID"
    assert v.rows == []
    assert not any("slow_one" in line for line in v.lines)


def test_control_dip_also_voids():
    """A machine that got FASTER invalidates the capture just as much."""
    v = verdict(make_current(control_ms=0.500), make_baseline())
    assert v.status == "VOID"


def test_missing_control_designation_refuses():
    """No control means no way to tell a regression from a slow machine."""
    v = verdict(make_current(), make_baseline(control=False))
    assert v.status == "REFUSE"


# --------------------------------------------------------------------------
# what the gate must measure — the 0.1.6 lessons
# --------------------------------------------------------------------------


def test_more_rows_for_more_time_does_not_trip():
    """THE 0.1.6 LESSON. 2x rows in 2x time is flat per row.

    Release 0.1.6 was 6 of 11 queries over a raw +10% ceiling, worst +69.5%,
    and every one was correct: CALLS edges grew 65% on purpose. A gate that
    fires on that is a gate that gets disabled.
    """
    v = verdict(make_current(slow_ms=4.000, slow_rows=20), make_baseline())
    assert v.status == "PASS"
    row = next(r for r in v.rows if r["name"] == "slow_one")
    assert row["delta_pct"] == pytest.approx(0.0)


def test_same_rows_for_more_time_does_trip():
    """The converse — holding rows fixed, the same 2x is a real regression."""
    v = verdict(make_current(slow_ms=4.000, slow_rows=10), make_baseline())
    assert v.status == "FAIL"


def test_sub_floor_relative_move_does_not_trip():
    """>30% per row but a trivial absolute move is below measurement quality.

    2.000 -> 2.200 ms is +10% and quiet; the floor case is a cell where the
    percentage is large only because the numbers are tiny.
    """
    cur = make_current(slow_ms=2.000)
    # 0.02 -> 0.04 ms: +100% per row, +0.02 ms raw — pure jitter territory.
    for q in cur["queries"]:
        if q["name"] == "slow_one":
            for k in ("a_median_ms", "b_median_ms", "a_min_ms", "b_min_ms"):
                q[k] = 0.040
    base = make_baseline()
    base["corpora"][0]["queries"][1]["median_ms"] = 0.020
    v = ba.compare(ba.normalize_bench(cur), base)
    assert v.status == "PASS"
    row = next(r for r in v.rows if r["name"] == "slow_one")
    assert row["delta_pct"] > 30.0 and not row["tripped"]
    assert "floor" in row["note"]


def test_measured_jitter_does_not_trip(tmp_path):
    """Pins the REAL noise this repo's bench corpus showed, so the gate that
    must not cry wolf provably does not.

    These are the two worst back-to-back deltas measured on 39 tracked `.rs`
    files with nothing changed between runs but the clock (recorded in
    `stability_run2_vs_run1` in the committed baseline).
    """
    base = make_baseline()
    base["corpora"][0]["queries"][1].update({"rows": 20, "median_ms": 0.118})
    cur = make_current(slow_rows=20)
    for q in cur["queries"]:
        if q["name"] == "slow_one":
            for k in ("a_median_ms", "b_median_ms", "a_min_ms", "b_min_ms"):
                q[k] = 0.283  # +139.8%, but only +0.165 ms
    v = ba.compare(ba.normalize_bench(cur), base)
    assert v.status == "PASS"


def test_baseline_supplies_its_own_floor():
    """The floor is a property of the corpus, not of the tool.

    A microsecond-scale corpus needs a microsecond-scale floor; the module
    default (0.25 ms, sized for a millisecond-scale corpus) would make it
    ungateable. Same fixture, two floors, two verdicts.
    """
    cur = make_current(slow_ms=0.100, slow_rows=10)  # 0.050 -> 0.100 ms: +100%
    base = make_baseline()
    base["corpora"][0]["queries"][1]["median_ms"] = 0.050

    coarse = ba.compare(ba.normalize_bench(cur), base)
    assert coarse.status == "PASS", "0.05 ms move must not clear a 0.25 ms floor"

    base["corpora"][0]["floors"] = {"query_abs_ms": 0.010}
    fine = ba.compare(ba.normalize_bench(cur), base)
    assert fine.status == "FAIL", "it must clear a 0.010 ms floor"


def test_explicit_override_beats_the_baseline_floor():
    """So a test (or an operator) can drive a boundary exactly."""
    cur = make_current(slow_ms=0.100, slow_rows=10)
    base = make_baseline(floors={"query_abs_ms": 0.010})
    base["corpora"][0]["queries"][1]["median_ms"] = 0.050
    v = ba.compare(ba.normalize_bench(cur), base, query_floor_ms=5.0)
    assert v.status == "PASS"


def test_node_count_growth_trips():
    """The other half of the 0.1.6 breach: node count, not just time."""
    v = verdict(make_current(nodes=1400), make_baseline())
    assert v.status == "FAIL"
    assert any("nodes" in line for line in v.lines if "TRIP" in line)


def test_build_time_growth_trips():
    v = verdict(make_current(build=0.900), make_baseline())
    assert v.status == "FAIL"


def test_build_secs_is_the_mean_not_the_min():
    """A once-per-build cost has no steady state, so `min` over builds reports
    the luckiest machine moment (performance protocol, doctrine R11)."""
    cur = make_current()
    cur["build_a_secs"], cur["build_b_secs"] = 0.400, 0.600
    assert ba.normalize_bench(cur)["build_secs"] == pytest.approx(0.500)


def test_query_median_is_the_min_of_the_two_builds():
    """A repeatable inner loop DOES have a floor — opposite rule, on purpose."""
    cur = make_current()
    cur["queries"][1]["a_median_ms"] = 3.0
    cur["queries"][1]["b_median_ms"] = 2.0
    assert ba.normalize_bench(cur)["queries"][1]["median_ms"] == pytest.approx(2.0)


def test_null_corpus_digest_is_rejected():
    """An untracked-corpus run is not reproducible and must never anchor."""
    cur = make_current()
    cur["corpus_sha256"] = None
    with pytest.raises(ValueError, match="tracked-only"):
        ba.normalize_bench(cur)


def test_zero_rows_falls_back_to_raw_and_says_so():
    cur = make_current(slow_rows=0)
    base = make_baseline()
    base["corpora"][0]["queries"][1]["rows"] = 0
    v = ba.compare(ba.normalize_bench(cur), base)
    row = next(r for r in v.rows if r["name"] == "slow_one")
    assert "not normalized" in row["note"]


# --------------------------------------------------------------------------
# baseline selection window
# --------------------------------------------------------------------------


def _mk(tmp_path: Path, *versions: str) -> Path:
    d = tmp_path / "baselines"
    d.mkdir(exist_ok=True)
    for v in versions:
        (d / f"{v}.json").write_text("{}")
    return d


def test_anchor_is_the_oldest_within_the_window(tmp_path):
    """Comparing to the previous release only ever sees one step of drift."""
    d = _mk(tmp_path, "0.1.4", "0.1.5", "0.1.6", "0.1.7")
    assert ba.select_anchor(d, window=3).name == "0.1.5.json"


def test_versions_sort_numerically_not_lexically(tmp_path):
    """`0.1.10` is newer than `0.1.9`; a string sort gets that backwards."""
    d = _mk(tmp_path, "0.1.9", "0.1.10", "0.2.0")
    assert ba.select_anchor(d, window=1).name == "0.2.0.json"
    assert ba.select_anchor(d, window=2).name == "0.1.10.json"


def test_degraded_window_uses_the_only_baseline(tmp_path):
    """Today's state: one baseline. Live, just spanning less history."""
    d = _mk(tmp_path, "0.1.7")
    assert ba.select_anchor(d, window=3).name == "0.1.7.json"
    r = subprocess.run(
        [str(SCRIPT), "select-baseline", str(d), "--window", "3"],
        capture_output=True,
        text=True,
    )
    assert r.returncode == 0
    assert "DEGRADED" in r.stderr


def test_prune_keeps_the_newest(tmp_path):
    d = _mk(tmp_path, "0.1.3", "0.1.4", "0.1.5", "0.1.6", "0.1.7")
    assert [p.name for p in ba.prunable(d, keep=4)] == ["0.1.3.json"]
    assert ba.prunable(d, keep=5) == []


# --------------------------------------------------------------------------
# wiring / hygiene guards
# --------------------------------------------------------------------------


def test_unknown_command_is_rejected(tmp_path):
    r = subprocess.run([str(SCRIPT), "rm-rf"], capture_output=True, text=True)
    assert r.returncode == 2
    assert "unknown" in r.stderr


def test_script_is_executable():
    assert SCRIPT.stat().st_mode & 0o111, "bench_anchor.sh must be executable"


def test_module_is_stdlib_only():
    """The `release-gates` CI job installs pytest and nothing else."""
    tree = ast.parse(MODULE.read_text())
    roots = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            roots.update(a.name.split(".")[0] for a in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module and node.level == 0:
            roots.add(node.module.split(".")[0])
    assert roots <= set(sys.stdlib_module_names), f"non-stdlib import(s): {roots}"


def test_no_working_folder_citation_in_committed_files():
    """The local working folder is gitignored, so a committed file that cites a
    path inside it becomes a dangling instruction the moment the folder is not
    there — every failure message here must name a committed path instead.

    The needle is assembled at runtime rather than written out: spelled
    literally, this guard would flag its own source and be permanently red.
    """
    needle = "dev-" + "docs/"
    targets = [MODULE, SCRIPT, Path(__file__), *committed_baselines()]
    for p in targets:
        assert needle not in p.read_text(), f"{p.name} cites the gitignored working folder"


def test_bench_json_still_emits_every_field_the_anchor_reads():
    """Guards the seam: if `codingest_bench` renames a JSON field, this suite
    would keep passing on synthetic fixtures while the real gate broke."""
    src = BENCH_RS.read_text()
    for field in (
        "corpus_sha256",
        "include_docs",
        "nodes",
        "edges",
        "build_a_secs",
        "build_b_secs",
        "a_median_ms",
        "b_median_ms",
        "a_min_ms",
        "b_min_ms",
        "rows",
        "queries",
    ):
        assert f'"{field}"' in src, f"codingest_bench.rs no longer emits {field!r}"


# --------------------------------------------------------------------------
# the committed baseline itself
# --------------------------------------------------------------------------


def committed_baselines():
    return sorted(BASELINE_DIR.glob("*.json"))


def test_at_least_one_committed_baseline_exists():
    assert committed_baselines(), "the anchor has nothing to compare against"


@pytest.mark.parametrize("path", committed_baselines(), ids=lambda p: p.stem)
def test_committed_baseline_is_well_formed(path):
    doc = json.loads(path.read_text())
    assert doc["schema"] == ba.SCHEMA_VERSION
    assert doc["version"] == path.stem, "file name must be its version"
    assert len(doc["captured_at_commit"]) == 40
    modes = {e["include_docs"] for e in doc["corpora"]}
    assert modes == {True, False}, "both docs modes must be recorded"
    for entry in doc["corpora"]:
        assert len(entry["corpus_sha256"]) == 64
        assert entry["target"], "an entry must name the corpus it measured"
        controls = [q for q in entry["queries"] if q.get("control")]
        assert len(controls) == 1, f"{entry['mode']}: need exactly one control"
        # A control below the recorded resolution cannot detect anything —
        # this is why `count_functions` (0.000 ms) is rejected on every corpus.
        floor = entry["floors"]["query_abs_ms"]
        assert controls[0]["median_ms"] >= floor, "control is below its own noise floor"
        assert controls[0]["rows"] >= 1


@pytest.mark.parametrize("path", committed_baselines(), ids=lambda p: p.stem)
def test_committed_baseline_anchors_a_frozen_corpus(path):
    """The anchor corpus must NOT be the code under test.

    `crates/codingest/src` changes almost every release, so its digest moves,
    so the comparison REFUSEs — a gate that cannot fail. It also conflates
    "the builder got slower" with "the builder has more source to parse".
    """
    doc = json.loads(path.read_text())
    for entry in doc["corpora"]:
        assert not entry["target"].startswith("crates/"), (
            f"{entry['target']} is the code under test; an anchor corpus must "
            "be frozen independently of it"
        )


@pytest.mark.parametrize("path", committed_baselines(), ids=lambda p: p.stem)
def test_committed_baseline_docs_modes_are_independent(path):
    """Both modes are only worth capturing if they measure different things.

    On an all-Rust corpus docs-on and docs-off produce byte-identical graphs,
    and the second row is decoration. This asserts the chosen corpus actually
    contains docs.
    """
    doc = json.loads(path.read_text())
    by_mode = {e["include_docs"]: e for e in doc["corpora"]}
    assert by_mode[True]["nodes"] != by_mode[False]["nodes"], (
        "docs-on and docs-off are identical — the anchor corpus has no docs, "
        "so capturing both modes proves nothing"
    )


@pytest.mark.parametrize("path", committed_baselines(), ids=lambda p: p.stem)
def test_committed_baseline_gate_is_not_vacuous(path):
    """A gate nothing can trip is the defect this whole file guards against.

    For each mode, doubling the slowest query must FAIL. If the corpus were
    too small for its own floor — every cell under `query_abs_ms` — this
    passes silently and the anchor is decoration.
    """
    doc = json.loads(path.read_text())
    for entry in doc["corpora"]:
        slowest = max(entry["queries"], key=lambda q: q["median_ms"])
        assert not slowest.get("control"), "the control cannot be the trip witness"
        current = _capture_from(entry)
        for q in current["queries"]:
            if q["name"] == slowest["name"]:
                for k in ("a_median_ms", "b_median_ms"):
                    q[k] *= 2
        v = ba.compare(ba.normalize_bench(current), doc)
        assert v.status == "FAIL", (
            f"{entry['mode']}: doubling {slowest['name']} "
            f"({slowest['median_ms']} ms) does not trip — the gate is vacuous"
        )


def _capture_from(entry: dict) -> dict:
    """Reconstruct the raw bench capture a baseline entry was made from."""
    return {
        "path": entry["target"],
        "corpus_sha256": entry["corpus_sha256"],
        "include_docs": entry["include_docs"],
        "nodes": entry["nodes"],
        "edges": entry["edges"],
        "build_a_secs": entry["build_secs"],
        "build_b_secs": entry["build_secs"],
        "queries": [
            {
                "name": q["name"],
                "rows": q["rows"],
                "a_median_ms": q["median_ms"],
                "b_median_ms": q["median_ms"],
                "a_min_ms": q["min_ms"],
                "b_min_ms": q["min_ms"],
            }
            for q in entry["queries"]
        ],
    }


@pytest.mark.parametrize("path", committed_baselines(), ids=lambda p: p.stem)
def test_committed_baseline_compares_clean_against_itself(path):
    """Round-trip: a baseline must PASS against a capture identical to it, in
    both modes. A baseline that cannot pass its own capture is unusable."""
    doc = json.loads(path.read_text())
    for entry in doc["corpora"]:
        v = ba.compare(ba.normalize_bench(_capture_from(entry)), doc)
        assert v.status == "PASS", f"{path.name} {entry['mode']}: {v.lines}"


def test_subfloor_control_move_does_not_void():
    """A control living below the absolute floor cannot void a capture.

    Row trips require BOTH >30% AND >= floors.query_abs_ms raw (module
    docstring: "must not void a capture") — the control is held to the same
    standard. 0.002 -> 0.003 ms is +50% relative but +0.001 ms raw, an order
    of magnitude under the 0.010 ms floor: sub-resolution jitter, not an
    instrument move. Live instance: the 0.2.0 release's docs-on capture
    void-looped on a 0.0019 -> 0.0024 ms/row control (+30.77%) that two
    agreeing re-measures reproduced exactly.
    """
    v = verdict(make_current(control_ms=0.003), make_baseline(control_ms=0.002))
    assert v.status != "VOID"
    assert any("sub-floor" in line or "under the" in line for line in v.lines)


def test_control_move_above_floor_still_voids():
    """The floor must not defang the control: a real move still voids."""
    v = verdict(make_current(control_ms=1.500), make_baseline(control_ms=1.000))
    assert v.status == "VOID"
