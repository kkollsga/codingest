# Golden parity digests

Each `<corpus>.sha256` here is a single lowercase-hex SHA-256 line: the digest
of a **canonical, exhaustive rendering** of the knowledge graph built from
`tests/corpus/<corpus>/`. The rendering (see
`canonical_graph_string` in `../parity.rs`) covers exactly what the
two-builder equivalence check sweeps — node-type counts, edge-type counts, the
sorted `(node_type, id)` identity set, the full per-node property sweep, and
the full per-edge property sweep — so two graphs share a digest iff they are
graph-equivalent.

## Provenance and authority

**Captured 2026-07-16 from the last in-sync in-tree builder** — i.e. from
`kglite::code_tree::builder::run_with_options`, KGLite's in-tree component,
while the `corpus_parity` test verified it was byte-for-byte identical to the
`codingest` builder (parity suite green, all 6 original corpora). **These
digests remain the anchor.** The additive `agc_basic` digest was reviewed and
captured with the new AGC parser on 2026-07-21, then deliberately refreshed on
2026-07-22 for its architecture-aware control/data relationship model. It
supplements the six historical authority digests without rewriting them.

KGLite deleted its in-tree builder on 2026-07-16, so `corpus_parity` (the live
in-tree vs codingest check) is gone. `golden_parity` — which builds each corpus
with only the `codingest` builder and compares to these frozen digests — is now
the sole guardian that codingest still produces the historically-correct graph.

`golden_parity` builds each corpus **three times** (`BUILDS_PER_CORPUS`), so it
is also the builder-determinism gate: hash iteration order is randomized per
`HashMap` instance, and `dup_minified_assets` reproduces the DEFINES-edge
ordering bug that once flapped edge totals run-to-run. A disagreement *between
builds* is nondeterminism and is never a reason to regenerate; agreement between
builds that differs from the golden is a behaviour change and may be. The test
reports the two cases distinctly.

## Regenerating (deliberate builder-behavior changes only)

Do **not** regenerate to make a red `golden_parity` go green. A digest change
means the graph a corpus produces changed; regenerate only when that change is
intended (a parser fix, a new edge kind, a property-shape change). The capture
path is:

```bash
cargo test -p codingest --test parity -- --ignored capture_goldens
```

This rewrites every `<corpus>.sha256`. Review the diff, and land the digest
change in the same commit as the builder change that caused it.

**Post-deletion note (done 2026-07-16).** KGLite's in-tree builder has been
deleted, and `capture_goldens` (in `../parity.rs`) has been retargeted from
`kglite::code_tree::builder::run_with_options` to
`codingest::builder::run_with_options` — codingest is now its own oracle.
These frozen digests, captured 2026-07-16 from the in-tree authority's
last-known-good output, remain the anchor a regeneration is reviewed against.

## Why there is no rev-fixture golden

The multi-rev fixture (`rev_self_consistency`) builds a throwaway 2-commit git
repo in a tempdir. Its canonical digest is **not stable across from-scratch
runs**: each run creates fresh commits whose SHAs depend on the commit
timestamp, and those SHAs are stored verbatim in the `revs` node and edge
property. Identical file content therefore yields different `revs` values
run-to-run, so the digest flaps. (`rev_fp`, a content-shape hash, is stable;
`revs` is not.)

Rather than freeze an unstable digest, the multi-rev path is guarded by
`rev_self_consistency`: it builds the **same** repo (identical SHAs) twice with
the `codingest` builder and asserts the two graphs are equivalent, including
the stamped `revs`/`rev_fp` provenance — a determinism check that needs no
second builder.
