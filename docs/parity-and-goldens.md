# Parity & goldens

codingest did not start as fresh code — it is a **behavior-preserving
extraction** of the `code_tree` component that lived inside kglite. This page
explains how that extraction was verified correct, and why the frozen goldens in
this repo now carry that authority forward.

## The extraction

The source is a byte-minimal transform of kglite's former in-tree
`crates/kglite/src/code_tree/`: import paths re-targeted at the public
`kglite::api` facade, two `pub(crate)` OKF helpers inlined verbatim, and the
`rev.rs` property-stamping internals bridged through public accessors. Because
the transform only rewrote import paths and bridged internals, the copied inline
unit tests — byte-identical to kglite's — pass against the transformed code,
proving the port is behavior-preserving.

## Authority transfer (2026-07-16)

While kglite still had its in-tree copy, a live two-builder equivalence check
(`corpus_parity` / `rev_path_parity`) verified that the codingest builder
produced **byte-for-byte identical output** across every corpus. KGLite then
deleted its in-tree builder, so that live cross-check is no longer possible.
Before the deletion, the authority was **frozen** into per-corpus digests.

## The frozen oracles

`crates/codingest/tests/parity.rs` is the standing gate (`cargo test
--workspace` runs it; goldens are committed fixtures, so it is fully offline):

- **`golden_parity`** builds each corpus under `crates/codingest/tests/corpus/`
  with the codingest builder, renders a **canonical exhaustive** graph string
  (node-type counts, edge-type counts, the sorted `(node_type, id)` identity
  set, the full per-node and per-edge property sweep), SHA-256s it, and compares
  to the frozen digest in `crates/codingest/tests/goldens/<corpus>.sha256`. Two
  graphs share a digest iff they are graph-equivalent. Captured 2026-07-16 from
  the last in-sync in-tree authority.
- **`rev_self_consistency`** guards the multi-rev path, which can't be frozen
  (fresh commit SHAs leak into the `revs` property): it builds the same repo
  twice and asserts the two graphs are equivalent.

Complementing the Rust gate:

- **`codingest_bench`** builds a target twice with the codingest builder and
  asserts identical Cypher query results across the two builds (determinism).
- The **DEFINES-edge nondeterminism** bug (randomized HashMap iteration over
  duplicate `(file, entity)` pairs from minified assets) is fixed with a
  BTreeMap + within-pair consolidation, guarded by the `dup_minified_assets`
  corpus and the `make gate` determinism reproducer (a stable edge count across
  three consecutive builds).

## Regenerating goldens

Do **not** regenerate a golden to make a red `golden_parity` go green — a digest
change means the graph a corpus produces changed. Regenerate only when that
change is intended (a parser fix, a new edge kind, a property-shape change), and
land the digest change in the same commit as the builder change:

```bash
cargo test -p codingest --test parity -- --ignored capture_goldens
```

Review the diff before committing. Details in
`crates/codingest/tests/goldens/README.md`.

## Behavioral spec archive

`tests/python-legacy/` preserves KGLite's full 47-file `kglite.code_tree`
behavioral suite verbatim (copied 2026-07-16, immediately before KGLite dropped
the Python surface). It is **dormant** — it imports the removed
`kglite.code_tree` module — but it is the exact, byte-for-byte record of the
behaviors the Python binding guaranteed. Three representative files have been
mechanically retargeted to `codingest` and run live under `tests/python/`; the
rest retarget the same way (see the READMEs in both directories).
