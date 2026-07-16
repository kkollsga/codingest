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
//!   * `golden_parity` builds each corpus once with `codingest` and asserts
//!     its canonical digest matches the frozen golden (see
//!     `tests/goldens/README.md`).
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
];

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
fn assert_graphs_equiv(label: &str, in_tree: &DirGraph, standalone: &DirGraph) {
    // 1. node-type counts
    let (a, b) = (node_type_counts(in_tree), node_type_counts(standalone));
    assert_eq!(
        a, b,
        "[{label}] node-type counts differ\nin-tree={a:?}\nstandalone={b:?}"
    );

    // 2. edge-type counts
    let (a, b) = (edge_type_counts(in_tree), edge_type_counts(standalone));
    assert_eq!(
        a, b,
        "[{label}] edge-type counts differ\nin-tree={a:?}\nstandalone={b:?}"
    );

    // 3. node identity set
    let (a, b) = (node_identities(in_tree), node_identities(standalone));
    if a != b {
        let only_in: Vec<_> = a.iter().filter(|x| !b.contains(x)).take(10).collect();
        let only_st: Vec<_> = b.iter().filter(|x| !a.contains(x)).take(10).collect();
        panic!(
            "[{label}] node identity sets differ\nonly in-tree (<=10): {only_in:?}\nonly standalone (<=10): {only_st:?}"
        );
    }

    // 4. per-node property sweep
    let (a, b) = (node_props(in_tree), node_props(standalone));
    if a != b {
        for (x, y) in a.iter().zip(b.iter()) {
            if x != y {
                panic!(
                    "[{label}] node properties differ\nfirst mismatch:\n  in-tree=({}, {}) {:?}\n  standalone=({}, {}) {:?}",
                    x.0, x.1, x.2, y.0, y.1, y.2
                );
            }
        }
        panic!(
            "[{label}] node property lists differ in length: in-tree={} standalone={}",
            a.len(),
            b.len()
        );
    }

    // 5. per-edge property sweep
    let (a, b) = (edge_props(in_tree), edge_props(standalone));
    if a != b {
        for (x, y) in a.iter().zip(b.iter()) {
            if x != y {
                panic!(
                    "[{label}] edge properties differ\nfirst mismatch:\n  in-tree=({}, {}->{}) {:?}\n  standalone=({}, {}->{}) {:?}",
                    x.0, x.1, x.2, x.3, y.0, y.1, y.2, y.3
                );
            }
        }
        panic!(
            "[{label}] edge property lists differ in length: in-tree={} standalone={}",
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
#[test]
fn golden_parity() {
    let root = corpus_root();
    for name in CORPORA {
        let dir = root.join(name);
        assert!(dir.is_dir(), "missing corpus dir: {}", dir.display());

        let g = codingest::builder::run_with_options(&dir, false, true, None, None, true)
            .unwrap_or_else(|e| panic!("[{name}] codingest build failed: {e}"));
        let got = graph_digest(&g);
        let want = read_golden(name);
        assert_eq!(
            got, want,
            "[{name}] golden digest mismatch\n  golden (frozen authority) = {want}\n  codingest build           = {got}\n\
             If this change to builder behavior is deliberate, regenerate with\n\
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
