# Parity Verification: in-tree `kglite::code_tree` vs standalone `codingest`

Date: 2026-07-15. Compared the in-tree module (`/Volumes/EksternalHome/Koding/Rust/KGLite/crates/kglite/src/code_tree/`)
against the standalone crate (`crates/codingest/`). Both link the same `kglite`
engine crate, so graphs from either builder are read through identical
`kglite::api` types (`DirGraph` / `NodeData` / `EdgeData` / `Value`).

**Verdict: full feature parity, full performance parity. Zero graph discrepancies found. No fixes required.**

## Update 2026-07-16: in-tree builder removed — parity now enforced by frozen record

KGLite deleted its in-tree `kglite::code_tree` builder on 2026-07-16 (the
planned handover — codingest is now the only builder). **Cross-builder
comparison is therefore no longer possible, and no longer needed.** The live
two-builder tests (`corpus_parity`, `rev_path_parity`) were removed from
`crates/codingest/tests/parity.rs`. Parity is now enforced by three surviving,
single-builder mechanisms:

1. **Golden digests** (`golden_parity`) — per-corpus SHA-256s captured
   2026-07-16 from the last in-sync in-tree authority, while the two builders
   were still verified byte-for-byte identical (§1 below was green). Each corpus
   is rebuilt with the codingest builder and its canonical digest compared to
   the frozen golden.
2. **Rev self-consistency** (`rev_self_consistency`) — the multi-rev fixture
   can't be frozen (fresh commit SHAs leak into `revs`), so it builds the same
   2-commit repo twice with the codingest builder and asserts equivalence,
   including the stamped `revs`/`rev_fp` provenance.
3. **The bench query-parity harness** (`codingest_bench`, `make bench-smoke`) —
   builds the target twice with the codingest builder and asserts identical
   Cypher query results across the two builds (a determinism check; any MISMATCH
   fails the gate).

Sections §1–§4 below are the historical parity/perf record captured while both
builders still existed — retained as the evidence behind the frozen goldens.

## 1. Corpus parity test (permanent regression test)

`crates/codingest/tests/parity.rs` — run with `cargo test -p codingest --test parity`.
Result: **2 passed, 0 failed**.

- `corpus_parity`: for each of `tests/corpus/{py_basic, py_inheritance, rust_xfile,
  ts_callback, cross_ts_py}`, builds the same directory with
  `kglite::code_tree::builder::run_with_options` and
  `codingest::builder::run_with_options` using identical arguments
  (`verbose=false, include_tests=true, save_to=None, max_loc_per_file=None,
  include_docs=true` — docs pass compiled on both sides: the standalone `docs`
  feature is default-on and enables `kglite/okf`). Asserts:
  - identical node-type → count maps
  - identical edge-type → count maps
  - identical sorted sets of `(node_type, id)` (id = qualified_name/path/title id)
  - identical per-node property maps — full sweep, every property, canonicalized
    via `Value`'s `Debug` form, both sides sorted
  - identical per-edge property maps — full sweep, keyed `(conn, src_id, tgt_id)`

- `rev_path_parity`: creates a throwaway git repo in a tempdir (2 commits: rev2
  removes a function, adds one, widens `foo`'s signature — a fingerprint change —
  and changes call edges), then runs `build_code_tree_revs` from BOTH sides over
  the same two revs and applies the same full equivalence check, **including the
  `revs` and `rev_fp` list properties on every node and the `revs` property on
  every edge**. This directly validates the one bridged internal-API gap in the
  standalone transform (`rev.rs` multi-rev stamping: `node.properties.insert(...)`
  → `node.set_property(...)` with a throwaway interner, and `get_or_intern` →
  `try_get_or_intern().expect()`). A sanity assertion confirms the merged graph
  actually carries stamped `revs` lists before comparing. Skips with a clear
  message if `git` is unavailable (it was available: git 2.48.1).

**Property exclusions: none.** No nondeterministic property was found — file
paths are stored relative to the project root, so even the two rev builds
(distinct tempdir snapshots) produce identical property maps.

## 1b. Golden-digest oracle (the survivor)

Added 2026-07-16. The two tests in §1 are the *live* cross-check: they build
each input with BOTH builders and will be **deleted together with KGLite's
in-tree builder**. To keep the authority after that deletion, it was frozen
while the builders were still verified-identical (§1 green) into per-corpus
SHA-256 golden digests at `crates/codingest/tests/goldens/<corpus>.sha256`.

- `golden_parity` (in `tests/parity.rs`) builds each of the 6 corpora with
  **only** `codingest::builder::run_with_options`, renders the graph to a
  deterministic exhaustive string (`canonical_graph_string` — the same
  node/edge count maps, identity set, and full property sweeps that §1
  compares), SHA-256s it, and asserts it equals the stored golden. It never
  references `kglite::code_tree`, so it outlives the in-tree deletion. Digests
  captured from the in-tree authority (first 12 hex):
  `py_basic 83c20d86fa6c`, `py_inheritance d27d37313d02`,
  `rust_xfile a44952b16301`, `ts_callback ea30ba202d55`,
  `cross_ts_py 16abbe05f4bc`, `dup_minified_assets 5a0799382c3b`.
- `capture_goldens` (`#[ignore]`) regenerates the goldens; while the in-tree
  builder exists it captures from that authority, and retargets to the
  codingest builder once the in-tree builder is deleted (documented at the
  call site and in `tests/goldens/README.md`).
- **Rev fixture not frozen.** The multi-rev tempdir repo gets fresh commit
  SHAs each run, and those SHAs are stored in the `revs` node/edge property, so
  its canonical digest is unstable across from-scratch runs (verified: two
  fresh repos of identical content produced different commit SHAs). Instead of
  a golden, `rev_self_consistency` builds the same repo twice with the
  codingest builder and asserts the two graphs are equivalent (including the
  stamped `revs`/`rev_fp` provenance) — a post-deletion-safe determinism check.

## 2. Real-repo stats diff

Both `codingest_stats` bins built `--release` in their own workspaces
(`cargo build -p kglite --bin code_tree_stats --release`,
`cargo build -p codingest --bin codingest_stats --release`). Source-level diff of
the two bins: only the doc-comment usage lines and the crate path
(`kglite::code_tree::builder` vs `code_tree::builder`) differ.

JSON outputs diffed with `jq -S` (sorted keys):

| Target | Result |
|---|---|
| `KGLite/crates/kglite/src` | identical after excluding `build_secs` |
| `codingest/crates/codingest` | **byte-identical including `build_secs`** (both `0.031`) |
| `KGLite` repo root (default) | identical after excluding `build_secs` |
| `KGLite` repo root (`--include-tests`) | identical after excluding `build_secs` |

**Excluded field: `build_secs` only** — it is the measured wall-clock build
time, inherently run-dependent. Every other field (nodes, edges, total_calls,
excluded_noise, no_candidate, ambiguous_dropped, resolved_call_sites,
resolved_via_inheritance, resolved_edges, resolution_rate, path,
include_tests) matched exactly on all targets. Reference figures on
`kglite/src`: 7045 nodes, 33028 edges, resolution_rate 0.505.

## 3. Performance

Largest target: the KGLite repo root. 5 runs each, alternating in-tree ↔
standalone, warm filesystem cache, `/usr/bin/time -p` wall time plus the bin's
internal `build_secs` (excludes process startup + JSON emit). hyperfine not
installed.

| Workload | in-tree median | standalone median | delta |
|---|---|---|---|
| KGLite root, default (build_secs) | 0.289 s | 0.288 s | −0.3 % |
| KGLite root, default (wall, time -p) | 0.29 s | 0.29 s | 0 % |
| KGLite root, `--include-tests` (build_secs) | 0.467 s | 0.461 s | −1.3 % |

Raw samples — default: in-tree 0.284/0.285/0.289/0.290/0.301, standalone
0.286/0.287/0.288/0.291/0.295. `--include-tests`: in-tree
0.462/0.466/0.467/0.467/0.478, standalone 0.445/0.457/0.461/0.470/0.475.

Both within noise (±5 %); the standalone is marginally faster if anything.
Profile parity verified: the standalone workspace `Cargo.toml` carries
`[profile.release] lto = "thin", codegen-units = 1, strip = "symbols"`,
mirroring KGLite's workspace profile, and both bins were built from their
workspace roots so the profiles applied.

## 4. Discrepancies

None. No graph-content difference of any kind was observed across the 5
corpus dirs, the multi-rev merge case, or the 3 real-repo stats targets.
No fixes to the standalone transform were needed.
