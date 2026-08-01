#!/usr/bin/env bash
# Phase-1 spike driver for dev-docs/plans/closure-scoped-definitions.md.
#
# For each corpus: build the real codingest graph (release), pull the exact
# parsed File/Function/Constant population out of it, run the D1/D2 prototype
# walk (nested_spike/) over that same file list, then score the result.
#
# Corpora are named on the command line as `<label>=<abs-path>`; each must be
# a clean git checkout, because the recorded commit SHA is what makes the
# numbers quotable (CLAUDE.md: "a number is meaningless without its corpus").
#
#   ./nested_spike_run.sh OUTDIR label=/abs/repo [label=/abs/repo ...]
#
# Writes per-corpus raw artifacts + a summary table into OUTDIR.
# Reads nothing it writes; safe to re-run. Never writes into a corpus.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"

if [[ $# -lt 2 ]]; then
  echo "usage: $0 OUTDIR label=/abs/repo [label=/abs/repo ...]" >&2
  exit 2
fi
OUT="$1"; shift
mkdir -p "$OUT"

CODINGEST="$REPO_ROOT/target/release/codingest"
STATS="$REPO_ROOT/target/release/codingest_stats"
SPIKE="$HERE/nested_spike/target/release/nested_spike"

echo "== building codingest (release) =="
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml" -p codingest -p codingest-cli
echo "== building the spike walker (release) =="
cargo build --release --manifest-path "$HERE/nested_spike/Cargo.toml"

# Grammar-kind evidence: the walk's whole vocabulary depends on these, and
# `typescript.rs` currently carries kinds the pinned grammar never emits.
cat > "$OUT/grammar-probe.ts" <<'PROBE'
const asFnExpr = function () { return 1 }
const asGenExpr = function* () { yield 2 }
const asArrow = () => 3
function decl() {}
function* genDecl() { yield 4 }
namespace N { export function inNs() {} }
const wrapped = Effect.fn("w")(function* () { const inner = () => 5 })
const layer = Layer.effect(Service, Effect.gen(function* () { const svc = () => 6 }))
PROBE
"$SPIKE" --probe "$OUT/grammar-probe.ts" > "$OUT/grammar-kinds.txt"

for spec in "$@"; do
  label="${spec%%=*}"
  repo="${spec#*=}"
  echo "== $label ($repo) =="
  sha="$(git -C "$repo" rev-parse HEAD)"
  dirty="$(git -C "$repo" status --porcelain | wc -l | tr -d ' ')"
  echo "{\"label\":\"$label\",\"path\":\"$repo\",\"commit\":\"$sha\",\"dirty_paths\":$dirty}" \
    > "$OUT/$label.corpus.json"

  # One build, reused for every query. --no-tests mirrors codingest_stats'
  # default (include_tests=false), which is the configuration Phase 6 will
  # measure against; docs stay off so the node-growth denominator is the
  # SMALLER, i.e. conservative, one.
  kgl="$OUT/$label.kgl"
  "$CODINGEST" build "$repo" --no-tests -o "$kgl" --format json > "$OUT/$label.build.json"

  q() { "$CODINGEST" query -g "$kgl" "$1" --format csv; }
  q "MATCH (n) RETURN labels(n) AS node_type, count(*) AS n ORDER BY n DESC" \
    > "$OUT/$label.nodecounts.csv"
  q "MATCH (f:File) WHERE f.language = 'typescript' OR f.language = 'javascript' \
     RETURN f.path AS path, f.language AS language ORDER BY path" \
    > "$OUT/$label.files.csv"
  q "MATCH (f:Function) RETURN f.name AS name, f.qualified_name AS qualified_name, \
     f.file_path AS file_path, f.line_number AS line_number ORDER BY file_path, line_number, name" \
    > "$OUT/$label.functions.csv"
  q "MATCH (c:Constant) RETURN c.name AS name, c.file_path AS file_path, \
     c.line_number AS line_number ORDER BY file_path, line_number, name" \
    > "$OUT/$label.constants.csv"

  # Denominator sensitivity. The ceiling is stated against `codingest_stats`
  # (docs off), which is the SMALLER node total; the docs-on build adds Doc
  # nodes to the denominator without adding anything to the numerator, so it
  # can only make growth look better. Recorded so nobody has to re-litigate
  # which denominator the verdict used.
  kgl_docs="$OUT/$label.docs.kgl"
  "$CODINGEST" build "$repo" --no-tests --include-docs -o "$kgl_docs" --format json \
    > "$OUT/$label.build-docs.json"
  "$CODINGEST" query -g "$kgl_docs" \
    "MATCH (n) RETURN labels(n) AS node_type, count(*) AS n ORDER BY n DESC" \
    --format csv > "$OUT/$label.nodecounts-docs.csv"
  rm -f "$kgl_docs" "$kgl_docs.meta.json"

  python3 - "$OUT/$label.files.csv" "$OUT/$label.files.tsv" <<'PY'
import csv, sys
with open(sys.argv[1], newline="") as fh, open(sys.argv[2], "w") as out:
    for row in csv.DictReader(fh):
        out.write(f"{row['path']}\t{row['language']}\n")
PY

  # Twice, to different files: D2's anon markers are line-based and the walk
  # must not depend on filesystem or hash-map order. Byte-identical output is
  # the determinism evidence the golden_parity triple-build will demand.
  "$SPIKE" --repo "$repo" --files "$OUT/$label.files.tsv" --out "$OUT/$label.spike.json"
  "$SPIKE" --repo "$repo" --files "$OUT/$label.files.tsv" --out "$OUT/$label.spike.rerun.json"
  if cmp -s "$OUT/$label.spike.json" "$OUT/$label.spike.rerun.json"; then
    echo "determinism: OK (byte-identical across two runs)" > "$OUT/$label.determinism.txt"
  else
    echo "determinism: FAILED — two runs differ" > "$OUT/$label.determinism.txt"
    echo "DETERMINISM FAILURE on $label" >&2
    exit 1
  fi
  rm -f "$OUT/$label.spike.rerun.json"
done

echo "== scoring =="
python3 "$HERE/nested_spike_report.py" "$OUT" "$@"
echo "artifacts: $OUT"
