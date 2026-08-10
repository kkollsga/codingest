#!/usr/bin/env bash
# Rebuild the P12 "agc-scaled" synthetic corpus.
#
#   usage: agc_scaled_corpus.sh <dest-dir> [n-programs]
#
# Reproduces the corpus behind every `phase12-agc*` row in
# dev-docs/bench/results/results.csv. With the default n=4000 the resulting
# git-tracked content MUST hash to:
#
#   corpus_sha256 = cc4c17e675052e0b5441e103ff2517579880101698275853793311409f2d4453
#   8000 files, 4348000 bytes
#
# The script verifies that digest itself and exits non-zero on any mismatch —
# a different digest means a different corpus, and numbers taken on it are NOT
# comparable to the P12 ledger rows (perf protocol: never compare two numbers
# whose digests differ).
#
# WHY THIS FILE EXISTS: P12 (2026-08-10) built this corpus with an inline shell
# loop that was never saved, and dev-docs/plans/agc-semantic-edge-performance.md
# claimed the recipe lived "in the bench ledger" — it did not; results.csv has
# no free-text column. Reconstructed and digest-verified 2026-08-11.
set -euo pipefail

DEST="${1:?usage: agc_scaled_corpus.sh <dest-dir> [n-programs]}"
N="${2:-4000}"
EXPECTED_SHA=cc4c17e675052e0b5441e103ff2517579880101698275853793311409f2d4453

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SRC="$REPO_ROOT/tests/corpus/agc_basic/PROG"

[ -f "$SRC/MAIN.agc" ] && [ -f "$SRC/SUB.agc" ] || {
  echo "FATAL: fixture missing at $SRC (need MAIN.agc + SUB.agc)" >&2; exit 1; }

# Program identity in the graph comes from the DIRECTORY name (bench anchors are
# PROG0001.FINISH / PROG0001.INTERP), so the zero-padded 4-wide PROG%04d naming
# is load-bearing. File names stay MAIN.agc / SUB.agc in every clone, and the
# bytes are identical copies of the fixture.
rm -rf "$DEST"
mkdir -p "$DEST"
for i in $(seq -w 1 "$N"); do
  mkdir -p "$DEST/PROG$i"
  cp "$SRC/MAIN.agc" "$SRC/SUB.agc" "$DEST/PROG$i/"
done

# codingest_bench materializes git-TRACKED content, so the corpus must be a
# committed repo or the digest is computed over nothing.
git -C "$DEST" init -q
git -C "$DEST" add -A
git -C "$DEST" -c user.name=b -c user.email=b@b commit -qm "agc scaled corpus: $N programs x agc_basic"

# Same digest codingest_bench::materialize_tracked prints:
#   sha256 over, per tracked path in sorted order: <rel>\0<hex sha256 of bytes>\n
manifest="$(mktemp)"
files=0; bytes=0
while IFS= read -r rel; do
  src="$DEST/$rel"
  [ -f "$src" ] || continue
  h=$(shasum -a 256 "$src" | cut -d' ' -f1)
  printf '%s\0%s\n' "$rel" "$h" >> "$manifest"
  files=$((files + 1))
  bytes=$((bytes + $(wc -c < "$src")))
done < <(git -C "$DEST" ls-files --cached | LC_ALL=C sort)
GOT=$(shasum -a 256 "$manifest" | cut -d' ' -f1)
rm -f "$manifest"

echo "corpus_sha256=$GOT"
echo "files=$files bytes=$bytes"

if [ "$N" = 4000 ] && [ "$GOT" != "$EXPECTED_SHA" ]; then
  echo "FATAL: digest mismatch — expected $EXPECTED_SHA" >&2
  echo "       This is NOT the P12 corpus; do not compare against phase12-agc rows." >&2
  exit 1
fi
echo "OK: matches the P12 corpus digest."
