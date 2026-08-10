//! Feature-parity regression test for the `codingest` code-tree builder.
//!
//! History: KGLite's in-tree `kglite::code_tree` module was the authority
//! this crate was extracted from. While both existed, `corpus_parity` and
//! `rev_path_parity` built every input with BOTH builders and asserted the
//! graphs were byte-for-byte equivalent. KGLite deleted its in-tree builder
//! on 2026-07-16, so cross-builder comparison is no longer possible — those
//! two tests were removed. The authority they enforced was frozen (while the
//! builders were still verified-identical) into per-corpus SHA-256 golden
//! digests, and the surviving tests carry it forward:
//!
//!   * `golden_parity` builds each corpus `BUILDS_PER_CORPUS` times with
//!     `codingest` and asserts every build's canonical digest matches the
//!     frozen golden (see `tests/goldens/README.md`). Repeating the build is
//!     what makes this the project's **determinism** gate as well: hash
//!     iteration order is randomized per `HashMap` instance, so an
//!     order-dependent builder produces digests that disagree with each other
//!     (reported as NONDETERMINISM) or agree with each other but not the
//!     golden (reported as a behaviour change). This replaced a Makefile-only
//!     step that ran three builds of an *external sibling checkout* and pinned
//!     its exact edge count — a gate whose verdict depended on a repository
//!     this project does not own, which never ran in CI, and which skipped
//!     silently when the checkout was absent.
//!   * `rev_self_consistency` builds the same 2-commit tempdir repo twice with
//!     `codingest` and asserts the graphs are equivalent — INCLUDING the
//!     `revs` / `rev_fp` list properties stamped onto every node and edge — a
//!     determinism check for the multi-rev path (whose digest can't be frozen;
//!     fresh commit SHAs leak into `revs`).
//!
//! `assert_graphs_equiv` compares two graphs along every dimension:
//!   * identical node-type → count maps
//!   * identical edge-type → count maps
//!   * identical sets of (node_type, id)
//!   * identical per-node property maps (full sweep, nothing excluded)
//!   * identical per-edge property maps (full sweep, nothing excluded)
//!
//! Nothing is excluded from the property sweep: file-path properties are
//! stored relative to the project root (`builder/mod.rs`), so they match
//! between two builds of the same tree.

use kglite::api::{DirGraph, GraphRead, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CORPORA: &[&str] = &[
    "py_basic",
    "py_inheritance",
    "rust_xfile",
    "ts_callback",
    "cross_ts_py",
    "dup_minified_assets",
    "agc_basic",
    // Added 2026-08-01. Every other corpus is blind to TS/JS import
    // resolution — none contains a single TS `import` statement, so a change
    // to it could land with zero golden movement. Its golden is therefore
    // captured additively (see `tests/goldens/README.md`).
    "ts_monorepo",
    // Added 2026-08-01. No other corpus contains a `const` bound to a
    // function literal, a `function*` in any spelling, or a factory-wrapped
    // binding (`Effect.fn(…)(function*…)`), so depth-0 higher-order-function
    // bindings and the TS grammar vocabulary could be changed — or broken —
    // with zero golden movement. Its golden is captured additively (see
    // `tests/goldens/README.md`).
    "ts_hof_binding",
    // Added 2026-08-01. Nothing else in the corpus set declares anything
    // below the top level of a TS file — no nested named binding, no
    // `namespace`, no closure-scoped helper — so the entire scope walk
    // (D1/D2/D3/D4) could be changed, or deleted, with zero golden movement.
    // It pins the shapes that must become nodes *and* the ones that must not
    // (a binding under an anonymous callback, `arr.map(f)` at depth > 0, a
    // plain IIFE), the `#{line}` tie-break, and D3's same-file-only CALLS
    // participation via a cross-file caller that must resolve nothing. Its
    // golden is captured additively (see `tests/goldens/README.md`).
    "ts_closure_scope",
    // Added 2026-08-01. The four committed Python corpora contain nothing but
    // top-level `def`s and plain classes — not one nested definition between
    // them — so the Python scope walk (D1/D2/D3/D4) could be changed, or
    // deleted, with zero golden movement. It pins the shapes that must become
    // nodes (a decorator factory two levels deep, a closure factory, a nested
    // helper, a method-local, a function-local class's methods), the block
    // transparency and lambda rules that are Python's answer to D1 clause 5,
    // the `#{line}` tie-break on both the `if`/`else` and `try`/`except`
    // conditional-definition idioms, and D3 from both sides — a cross-file
    // caller that must resolve nothing against same-file CALLS, REFERENCES_FN
    // and DECORATES edges that must resolve. Its golden is captured additively
    // (see `tests/goldens/README.md`).
    "py_nested_defs",
    // Added 2026-08-01. Not one of the ten pre-existing corpora contains an
    // `.mdx`, an upper-cased `.MD`, or a `.txt` that must stay out — so the
    // docs pass's accepted-extension match could be widened (to `.txt`, the
    // tempting "helpful" change) or narrowed (dropping `.mdx`) with zero
    // golden movement. It pins the `.mdx` arm end to end — frontmatter
    // properties, headings, backtick MENTIONS, a doc→doc DOCUMENTS edge whose
    // target is an `.mdx` and whose source is a `.md`, and a doc→File edge —
    // plus the extension-stripped `:Doc` id (`README.MD` → `README`) and the
    // inertness of embedded JSX/ESM. `NOTES.txt` is markdown-shaped and must
    // contribute nothing. Its golden is captured additively (see
    // `tests/goldens/README.md`).
    "docs_mdx",
    // Added 2026-08-10. No other corpus contains two docs in one directory
    // whose names differ only by markup extension, so the concept-id collision
    // policy (`.mdx` > `.md` > `.rst`; the loser is dropped from doc-node
    // emission entirely) could be changed — or regress to the old silent
    // last-write-wins overwrite — with zero golden movement. It pins the
    // survivor's identity (the `.mdx` file's title, `file_path` and mentions),
    // the loser's total absence (a symbol and a link that exist ONLY in
    // `docs/guide.md` must contribute no edge), a link written against the
    // DROPPED spelling (`docs/guide.md`) still resolving to the surviving
    // `docs/guide` node, an uncolliding `.rst` surviving untouched, and a
    // mixed-case `Notes.MD` stripping to `Notes`. Its golden is captured
    // additively (see `tests/goldens/README.md`).
    "docs_ext_collide",
];

/// Independent builds of each corpus per `golden_parity` run.
///
/// One build proves the output still matches the frozen golden; repeating it
/// also proves the output does not depend on hash iteration order. The
/// original nondeterminism bug (randomized `HashMap` iteration over DEFINES
/// (source_type, target_type) pairs — whichever pair went first got
/// skip-existence-check dedup semantics) is reproduced by the
/// `dup_minified_assets` corpus, which defines the same selector/element id
/// from one file more than once. Three builds is enough to make an
/// order-dependent builder fail loudly and quickly; the digest comparison
/// against the golden catches the residual case where all three happen to
/// agree on a wrong order.
const BUILDS_PER_CORPUS: usize = 3;

fn corpus_root() -> PathBuf {
    // tests/parity.rs lives in crates/code-tree; corpus is at the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("corpus")
}

/// Deterministic canonical string for a property value. `{:?}` on `Value`
/// is stable (enums, not maps) and captures scalars and nested lists alike.
fn canon(v: &Value) -> String {
    format!("{v:?}")
}

/// node_type -> count.
fn node_type_counts(g: &DirGraph) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for idx in g.graph.node_indices() {
        if let Some(n) = g.graph.node_weight(idx) {
            *m.entry(n.node_type_str(&g.interner).to_string())
                .or_insert(0) += 1;
        }
    }
    m
}

/// connection_type -> count.
fn edge_type_counts(g: &DirGraph) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for e in g.graph.edge_indices() {
        if let Some(edge) = g.graph.edge_weight(e) {
            *m.entry(edge.connection_type_str(&g.interner).to_string())
                .or_insert(0) += 1;
        }
    }
    m
}

/// Sorted (node_type, id) identity set.
fn node_identities(g: &DirGraph) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = g
        .graph
        .node_indices()
        .filter_map(|i| g.graph.node_weight(i))
        .map(|n| (n.node_type_str(&g.interner).to_string(), n.id().to_string()))
        .collect();
    v.sort();
    v
}

/// Full per-node property sweep: sorted list of
/// (node_type, id, sorted{prop -> canon}). Nothing excluded.
fn node_props(g: &DirGraph) -> Vec<(String, String, BTreeMap<String, String>)> {
    let mut v: Vec<(String, String, BTreeMap<String, String>)> = g
        .graph
        .node_indices()
        .filter_map(|i| g.graph.node_weight(i))
        .map(|n| {
            let props: BTreeMap<String, String> = n
                .properties_cloned(&g.interner)
                .iter()
                .map(|(k, val)| (k.clone(), canon(val)))
                .collect();
            (
                n.node_type_str(&g.interner).to_string(),
                n.id().to_string(),
                props,
            )
        })
        .collect();
    v.sort();
    v
}

/// Full per-edge property sweep: sorted list of
/// (conn, src_id, tgt_id, sorted{prop -> canon}). Edges can share an
/// identity triple, so this is a multiset kept as a sorted Vec.
fn edge_props(g: &DirGraph) -> Vec<(String, String, String, BTreeMap<String, String>)> {
    let mut v: Vec<(String, String, String, BTreeMap<String, String>)> = g
        .graph
        .edge_indices()
        .filter_map(|e| {
            let edge = g.graph.edge_weight(e)?;
            let (s, t) = g.graph.edge_endpoints(e)?;
            let sn = g.graph.node_weight(s)?;
            let tn = g.graph.node_weight(t)?;
            let props: BTreeMap<String, String> = edge
                .properties_cloned(&g.interner)
                .iter()
                .map(|(k, val)| (k.clone(), canon(val)))
                .collect();
            Some((
                edge.connection_type_str(&g.interner).to_string(),
                sn.id().to_string(),
                tn.id().to_string(),
                props,
            ))
        })
        .collect();
    v.sort();
    v
}

// ── canonical rendering + golden digest ──────────────────────────────────
//
// The golden oracle renders a graph to a single deterministic string built
// from the *exact same* data the equivalence assertions sweep — node-type
// counts, edge-type counts, the sorted (node_type, id) identity set, the full
// per-node property sweep, and the full per-edge property sweep — then hashes
// it with SHA-256. Because every input is already sorted (BTreeMap / sorted
// Vec) and `canon` is `Value`'s stable `Debug` form, the rendering is
// reproducible across runs and machines. Nothing is excluded: whatever
// `assert_graphs_equiv` compares, the digest covers.

/// Deterministic, exhaustive canonical rendering of a graph. Two graphs render
/// to the same string iff `assert_graphs_equiv` would consider them equivalent.
fn canonical_graph_string(g: &DirGraph) -> String {
    let mut s = String::new();

    s.push_str("## node_type_counts\n");
    for (ty, n) in node_type_counts(g) {
        s.push_str(&format!("{ty}\t{n}\n"));
    }

    s.push_str("## edge_type_counts\n");
    for (ty, n) in edge_type_counts(g) {
        s.push_str(&format!("{ty}\t{n}\n"));
    }

    s.push_str("## node_identities\n");
    for (ty, id) in node_identities(g) {
        s.push_str(&format!("{ty}\t{id}\n"));
    }

    s.push_str("## node_props\n");
    for (ty, id, props) in node_props(g) {
        s.push_str(&format!("{ty}\t{id}\n"));
        for (k, v) in props {
            s.push_str(&format!("\t{k}={v}\n"));
        }
    }

    s.push_str("## edge_props\n");
    for (conn, src, tgt, props) in edge_props(g) {
        s.push_str(&format!("{conn}\t{src}\t{tgt}\n"));
        for (k, v) in props {
            s.push_str(&format!("\t{k}={v}\n"));
        }
    }

    s
}

/// SHA-256 (lowercase hex) of the canonical graph rendering — the golden digest.
fn graph_digest(g: &DirGraph) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_graph_string(g).as_bytes());
    let out = hasher.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
}

/// Read the stored golden digest for a corpus (one hex line), trimmed.
fn read_golden(corpus: &str) -> String {
    let path = goldens_dir().join(format!("{corpus}.sha256"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "missing golden {} — capture with `cargo test -p codingest --test parity -- --ignored capture_goldens`: {e}",
                path.display()
            )
        })
        .trim()
        .to_string()
}

/// Assert the two graphs are equivalent along every dimension, with
/// pinpointed diffs on failure.
fn assert_graphs_equiv(label: &str, left: &DirGraph, right: &DirGraph) {
    // 1. node-type counts
    let (a, b) = (node_type_counts(left), node_type_counts(right));
    assert_eq!(
        a, b,
        "[{label}] node-type counts differ\nleft={a:?}\nright={b:?}"
    );

    // 2. edge-type counts
    let (a, b) = (edge_type_counts(left), edge_type_counts(right));
    assert_eq!(
        a, b,
        "[{label}] edge-type counts differ\nleft={a:?}\nright={b:?}"
    );

    // 3. node identity set
    let (a, b) = (node_identities(left), node_identities(right));
    if a != b {
        let only_in: Vec<_> = a.iter().filter(|x| !b.contains(x)).take(10).collect();
        let only_st: Vec<_> = b.iter().filter(|x| !a.contains(x)).take(10).collect();
        panic!(
            "[{label}] node identity sets differ\nonly left (<=10): {only_in:?}\nonly right (<=10): {only_st:?}"
        );
    }

    // 4. per-node property sweep
    let (a, b) = (node_props(left), node_props(right));
    if a != b {
        for (x, y) in a.iter().zip(b.iter()) {
            if x != y {
                panic!(
                    "[{label}] node properties differ\nfirst mismatch:\n  left=({}, {}) {:?}\n  right=({}, {}) {:?}",
                    x.0, x.1, x.2, y.0, y.1, y.2
                );
            }
        }
        panic!(
            "[{label}] node property lists differ in length: left={} right={}",
            a.len(),
            b.len()
        );
    }

    // 5. per-edge property sweep
    let (a, b) = (edge_props(left), edge_props(right));
    if a != b {
        for (x, y) in a.iter().zip(b.iter()) {
            if x != y {
                panic!(
                    "[{label}] edge properties differ\nfirst mismatch:\n  left=({}, {}->{}) {:?}\n  right=({}, {}->{}) {:?}",
                    x.0, x.1, x.2, x.3, y.0, y.1, y.2, y.3
                );
            }
        }
        panic!(
            "[{label}] edge property lists differ in length: left={} right={}",
            a.len(),
            b.len()
        );
    }
}

// ── golden parity (the authority carried forward past the in-tree deletion) ─
//
// Builds every corpus with ONLY the `codingest` builder, digests the result,
// and compares it to the frozen golden captured from the in-tree authority
// (see `tests/goldens/README.md`). This is the test that carries the
// authority forward now that KGLite has deleted its in-tree builder: it keeps
// proving that codingest reproduces the frozen graph byte-for-byte.
//
// It is also the determinism gate: each corpus is built `BUILDS_PER_CORPUS`
// times and every build must produce the same digest as every other *and* as
// the golden. The two failure modes are reported separately, because they call
// for opposite responses — a behaviour change may be legitimate and
// regenerated, while nondeterminism is always a bug.
#[test]
fn golden_parity() {
    let root = corpus_root();
    for name in CORPORA {
        let dir = root.join(name);
        assert!(dir.is_dir(), "missing corpus dir: {}", dir.display());

        let build = || {
            codingest::builder::run_with_options(&dir, false, true, None, None, true)
                .unwrap_or_else(|e| panic!("[{name}] codingest build failed: {e}"))
        };

        let first = build();
        let got = graph_digest(&first);

        // 1. Determinism: repeat builds must be identical to the first.
        for run in 2..=BUILDS_PER_CORPUS {
            let again = build();
            let again_digest = graph_digest(&again);
            if again_digest != got {
                // Pinpoint the divergence before reporting it.
                assert_graphs_equiv(
                    &format!("{name} nondeterminism run 1 vs {run}"),
                    &first,
                    &again,
                );
                panic!(
                    "[{name}] NONDETERMINISM: build 1 and build {run} of the same corpus \
                     produced different digests ({got} != {again_digest}) but compared equal \
                     dimension-by-dimension — widen the canonical rendering."
                );
            }
        }

        // 2. Frozen authority: the (now proven stable) digest must be the golden.
        let want = read_golden(name);
        assert_eq!(
            got, want,
            "[{name}] golden digest mismatch\n  golden (frozen authority) = {want}\n  codingest build           = {got}\n\
             All {BUILDS_PER_CORPUS} builds agreed, so this is a builder BEHAVIOUR change, not nondeterminism.\n\
             If it is deliberate, regenerate with\n\
             `cargo test -p codingest --test parity -- --ignored capture_goldens`."
        );
    }
}

/// Regeneration path for the corpus goldens. `#[ignore]` so it never runs in
/// the normal suite — invoke explicitly:
///
///   cargo test -p codingest --test parity -- --ignored capture_goldens
///
/// It writes `tests/goldens/<corpus>.sha256` for every corpus.
///
/// AUTHORITY NOTE: KGLite deleted its in-tree builder on 2026-07-16, so
/// regeneration now captures from the `codingest` builder — codingest is its
/// own oracle. The frozen digests it overwrites were originally captured (on
/// 2026-07-16) from the last in-sync in-tree authority, so only regenerate
/// when a deliberate builder-behavior change makes a golden legitimately
/// stale (see `tests/goldens/README.md`), never to paper over a red
/// `golden_parity`.
#[test]
#[ignore = "regeneration path; run explicitly with --ignored capture_goldens"]
fn capture_goldens() {
    let root = corpus_root();
    let out_dir = goldens_dir();
    std::fs::create_dir_all(&out_dir).expect("create goldens dir");
    for name in CORPORA {
        let dir = root.join(name);
        assert!(dir.is_dir(), "missing corpus dir: {}", dir.display());

        // Capture from the codingest builder — the authority, now that the
        // in-tree builder is gone.
        let g = codingest::builder::run_with_options(&dir, false, true, None, None, true)
            .unwrap_or_else(|e| panic!("[{name}] codingest build failed: {e}"));
        let digest = graph_digest(&g);
        let path = out_dir.join(format!("{name}.sha256"));
        std::fs::write(&path, format!("{digest}\n")).expect("write golden");
        eprintln!("captured {name} -> {digest}");
    }
}

// ── serialized-bytes determinism ─────────────────────────────────────────
//
// `golden_parity` digests the graph as it exists IN MEMORY, from sorted maps
// and sorted vectors. That rendering is structurally insulated from the order
// properties were inserted in — which is precisely the bug class that bit this
// project before: two builds with identical in-memory digests wrote `.kgl`
// files that differed byte-for-byte, because edge-property insertion order
// leaked into the serialization (fixed engine-side in kglite 0.14.5; the
// codingest-side assertion never existed). CALLS edges now carry three more
// properties each, so the exposure is larger, not smaller.
//
// This is the net the in-memory digest cannot provide: build the same corpus
// repeatedly, save each build, compare the FILES.
#[test]
fn kgl_bytes_are_stable_across_builds() {
    let tmp = tempfile::Builder::new()
        .prefix("codingest-kgl-bytes-")
        .tempdir()
        .unwrap();
    let root = corpus_root();

    // `ts_monorepo` carries the densest multi-property CALLS edge set
    // (resolution + candidates + import_backed on every edge); `agc_basic`
    // exercises the sparse-column path, where the semantic pass leaves those
    // three null while populating raw_targets / via / address_lines.
    for name in ["ts_monorepo", "agc_basic"] {
        let dir = root.join(name);
        assert!(dir.is_dir(), "missing corpus dir: {}", dir.display());

        let mut bytes: Vec<Vec<u8>> = Vec::new();
        for run in 1..=BUILDS_PER_CORPUS {
            let dest = tmp.path().join(format!("{name}-{run}.kgl"));
            codingest::builder::run_with_options(&dir, false, true, Some(&dest), None, true)
                .unwrap_or_else(|e| panic!("[{name}] build {run} failed: {e}"));
            bytes.push(std::fs::read(&dest).expect("read written .kgl"));
        }

        for run in 2..=BUILDS_PER_CORPUS {
            let (first, again) = (&bytes[0], &bytes[run - 1]);
            if first == again {
                continue;
            }
            let at = first
                .iter()
                .zip(again.iter())
                .position(|(a, b)| a != b)
                .map(|i| i.to_string())
                .unwrap_or_else(|| "end (length differs)".into());
            panic!(
                "[{name}] .kgl BYTES differ between build 1 and build {run} \
                 ({} vs {} bytes, first difference at offset {at}). The in-memory \
                 parity digest cannot see this: it is built from sorted maps, so \
                 property INSERTION order is invisible to it while remaining \
                 visible in the serialized file.",
                first.len(),
                again.len()
            );
        }
    }
}

/// Diagnostic dump of every corpus's canonical rendering, one file per
/// corpus, into the directory named by `CODINGEST_CANONICAL_DUMP`.
///
/// This is the tool the golden-regeneration protocol runs on: when a change is
/// *supposed* to move only one section of the rendering (e.g. adding edge
/// properties must not touch `node_type_counts`, `edge_type_counts`,
/// `node_identities` or `node_props`), dumping before and after and diffing
/// the sections proves it. A digest tells you *that* something moved; only the
/// rendering tells you *what*, and blessing an edge-set change inside a
/// "properties-only" regeneration would hide it forever.
///
///   CODINGEST_CANONICAL_DUMP=/tmp/before \
///     cargo test -p codingest --test parity -- --ignored dump_canonical
#[test]
#[ignore = "diagnostic; run explicitly with --ignored dump_canonical"]
fn dump_canonical() {
    let out_dir = PathBuf::from(
        std::env::var("CODINGEST_CANONICAL_DUMP")
            .expect("set CODINGEST_CANONICAL_DUMP to the output directory"),
    );
    std::fs::create_dir_all(&out_dir).expect("create dump dir");
    let root = corpus_root();
    for name in CORPORA {
        let dir = root.join(name);
        let g = codingest::builder::run_with_options(&dir, false, true, None, None, true)
            .unwrap_or_else(|e| panic!("[{name}] codingest build failed: {e}"));
        let path = out_dir.join(format!("{name}.txt"));
        std::fs::write(&path, canonical_graph_string(&g)).expect("write dump");
        eprintln!("dumped {name} -> {}", path.display());
    }
}

// ── rev-path parity ──────────────────────────────────────────────────────

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git spawn");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn git_commit(dir: &Path, msg: &str) -> String {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            msg,
        ],
    );
    git(dir, &["rev-parse", "HEAD"])
}

/// Build a throwaway 2-commit python repo in `repo` and return the two commit
/// SHAs (oldest → newest). rev2 widens `foo`'s signature (a fingerprint
/// change), drops `gone`, adds `bar`, and makes `driver` call `bar` too.
fn build_two_commit_repo(repo: &Path) -> Vec<String> {
    git(repo, &["init", "-q"]);

    // rev1: a small python package with two functions and a caller.
    std::fs::create_dir_all(repo.join("pkg")).unwrap();
    std::fs::write(
        repo.join("pkg/mod_a.py"),
        "def foo(a):\n    return a + 1\n\n\ndef gone():\n    return 0\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("pkg/mod_b.py"),
        "from pkg.mod_a import foo\n\n\ndef driver(x):\n    return foo(x)\n",
    )
    .unwrap();
    let s1 = git_commit(repo, "rev1");

    // rev2: widen foo's signature (fingerprint change), drop `gone`, add `bar`,
    // and make driver call the new bar too.
    std::fs::write(
        repo.join("pkg/mod_a.py"),
        "def foo(a, b):\n    return a + b\n\n\ndef bar(y):\n    return y * 2\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("pkg/mod_b.py"),
        "from pkg.mod_a import foo, bar\n\n\ndef driver(x):\n    return foo(x, 1) + bar(x)\n",
    )
    .unwrap();
    let s2 = git_commit(repo, "rev2");

    vec![s1, s2]
}

fn assert_has_stamped_revs(g: &DirGraph) {
    let has_revs = g
        .graph
        .node_indices()
        .filter_map(|i| g.graph.node_weight(i))
        .any(|n| matches!(n.get_property_value("revs"), Some(Value::List(_))));
    assert!(
        has_revs,
        "expected stamped `revs` list properties on rev-merge graph"
    );
}

// ── rev self-consistency (the multi-rev survivor) ──────────────────────────
//
// The rev fixture can't be frozen into a golden (commit SHAs leak into the
// `revs` property, so its digest is unstable across from-scratch runs — see
// `tests/goldens/README.md`). Instead we prove the codingest builder is
// self-consistent: building the SAME two-commit repo twice (identical SHAs,
// since it's one repo) yields two equivalent graphs, INCLUDING the stamped
// `revs`/`rev_fp` provenance.
#[test]
fn rev_self_consistency() {
    if !git_available() {
        eprintln!("SKIP rev_self_consistency: git not available in test environment");
        return;
    }

    let tmp = tempfile::Builder::new()
        .prefix("codingest-rev-selfconsistency-")
        .tempdir()
        .unwrap();
    let repo = tmp.path();
    let revs = build_two_commit_repo(repo);

    let build = || {
        codingest::rev::build_code_tree_revs(repo, &revs, Some(repo), false, true, None, None, true)
            .expect("codingest build_code_tree_revs")
    };
    let first = build();
    let second = build();

    // Confirm the multi-rev provenance is actually present before comparing.
    assert_has_stamped_revs(&first);

    // Two builds of the same fixed rev input must be byte-for-byte equivalent.
    assert_graphs_equiv("rev-self-consistency", &first, &second);
}
