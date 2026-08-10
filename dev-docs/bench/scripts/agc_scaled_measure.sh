#!/bin/bash
# usage: measure.sh <label> <corpus-path> <outfile>
LABEL=$1; CORPUS=$2; OUT=$3
BIN=/Volumes/EksternalHome/Koding/Rust/codingest/target/release/codingest_stats
: > "$OUT"
for r in 1 2 3 4 5; do
  KGLITE_CODE_TREE_VERBOSE=1 "$BIN" "$CORPUS" > "$OUT.run$r" 2>&1 || { echo "RUN $r FAILED"; exit 1; }
  build=$(grep -o '"build_secs": [0-9.]*' "$OUT.run$r" | head -1 | awk '{print $2}')
  calls=$(grep '^\[timing\]   calls:' "$OUT.run$r" | awk '{print $3}' | tr -d 's')
  refs=$(grep '^\[timing\]   references:' "$OUT.run$r" | awk '{print $3}' | tr -d 's')
  alias=$(grep '^\[timing\]   alias_of+points_to:' "$OUT.run$r" | awk '{print $3}' | tr -d 's')
  load=$(grep '^\[timing\] load:' "$OUT.run$r" | awk '{print $3}' | tr -d 's')
  echo "$LABEL,$r,$build,$calls,$refs,$alias,$load" >> "$OUT"
done
echo "label,run,build,calls,references,alias,load"
cat "$OUT"
python3 - "$OUT" <<'PY'
import sys,statistics
rows=[l.strip().split(',') for l in open(sys.argv[1]) if l.strip()]
cols=['build','calls','references','alias','load']
for i,c in enumerate(cols):
    v=[float(r[2+i]) for r in rows]
    print(f"{c}: min={min(v):.4f} median={statistics.median(v):.4f} mean={statistics.mean(v):.4f}")
PY
