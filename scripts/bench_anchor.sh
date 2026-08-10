#!/usr/bin/env bash
#
# Release perf anchor — entry point.
#
# WHY THIS FILE EXISTS SEPARATELY FROM bench_anchor.py
# ----------------------------------------------------
# `scripts/release_gates.sh` established the shape: release-path logic lives in
# a script that branch CI can drive on every push, so "a new gate is not
# trusted until you have seen it fail" is satisfiable. This is that entry
# point for the perf anchor; the comparison logic is in `bench_anchor.py`
# beside it, because the inputs are JSON and parsing JSON in bash is how a
# gate is born dead. `scripts/verify_wheel.py` is the existing precedent for
# Python in this project's release tooling — no new dependency either way, the
# module is stdlib-only.
#
# The split is also what makes the logic unit-testable two ways, mirroring
# `test_release_gates.py`'s own `run_shell` (source the real shell) /  `call`
# (drive the CLI) duality: `tests/release/test_bench_anchor.py` imports the
# module for the comparison rules AND drives this script end to end for the
# exit codes.
#
# Usage:
#   scripts/bench_anchor.sh compare --current CUR.json --baseline BASE.json
#   scripts/bench_anchor.sh select-baseline tests/benchmarks/baselines [--window 3]
#   scripts/bench_anchor.sh prune tests/benchmarks/baselines [--keep 4] [--delete]
#
# Exit codes are the contract and are passed through UNCHANGED — each names a
# different operator action, which is why they are not collapsed into 1:
#   0  PASS    nothing drifted
#   1  FAIL    real drift past the threshold — this blocks the release tag
#   2  USAGE   bad arguments or unreadable input
#   3  REFUSE  not comparable (corpus digest or docs mode differs); no delta
#   4  VOID    the control query moved — re-measure, do not bisect
#
# NOTE ON `set -e` AND THIS WRAPPER: the python invocation is the last command
# and its status is this script's status, so a non-zero verdict must NOT be
# swallowed. It is deliberately NOT inside a `$( )` and NOT in a pipeline — a
# pipeline reports its last stage, which is exactly the mask that made
# release.yml's version extraction unfailable.

set -euo pipefail

# Indirected so the unit test can point at a specific interpreter, the same way
# release_gates.sh indirects curl through CODINGEST_RELEASE_CURL.
BENCH_ANCHOR_PYTHON="${BENCH_ANCHOR_PYTHON:-python3}"

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

main() {
  local cmd="${1:-}"
  if [ -z "$cmd" ]; then
    printf 'usage: bench_anchor.sh <compare|select-baseline|prune> [args...]\n' >&2
    return 2
  fi
  # Allowlisted, like release_gates.sh's dispatch: a typo (or an injected
  # argument) must not reach argparse as something else entirely.
  case "$cmd" in
    compare | select-baseline | prune) ;;
    *)
      printf '::error::unknown bench_anchor.sh command: %s\n' "$cmd" >&2
      return 2
      ;;
  esac
  "$BENCH_ANCHOR_PYTHON" "$here/bench_anchor.py" "$@"
}

if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  main "$@"
fi
