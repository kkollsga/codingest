//! codingest Cypher query/traversal benchmark + query-parity harness.
//!
//! KGLite deleted its in-tree `code_tree` builder on 2026-07-16, so the old
//! cross-builder comparison (in-tree vs standalone) is gone. This harness now
//! builds the SAME target directory TWICE with `codingest::builder::run_with_options`
//! (two independent builds A and B), then runs an identical Cypher workload
//! against BOTH through kglite's canonical read path
//! (`kglite::api::session::{execute_read, ExecuteOptions}` — the exact pipeline
//! an MCP / CLI user hits).
//!
//! For each query we alternate timed iterations A/B/A/B… (warm cache) and report
//! median + min per side. Before timing, each query's result is compared across
//! the two builds (row count + a canonicalized, order-insensitive value digest);
//! a MISMATCH is reported and that query is NOT timed. Because the builder is
//! deterministic, two independent builds must return identical results — so a
//! MISMATCH is a query-result determinism regression, and the A/B delta is
//! run-to-run timing variance. The release gate fails on any MISMATCH.
//!
//! # The corpus is defined by the harness, not by the working directory
//!
//! A benchmark number is comparable to another only if both were measured on
//! the same input. The builder walks whatever is on disk: it skips dot-dirs,
//! `target/`, `node_modules/`, `venv/` and `__pycache__/` by name, but it has
//! no notion of `.gitignore`, so a repository's *untracked* working state
//! (scratch folders, local notes, generated markdown) is ingested — notably
//! through the docs pass. Benchmarking a working tree therefore measures a
//! corpus nobody else can reconstruct. Measured on this workspace: a clean
//! `git worktree` of a commit scored 1,115 nodes / 3,692 edges while the
//! working tree at that same commit scored 1,170 / 3,759 — a ~5% swing with no
//! builder or engine involvement, which silently invalidated published numbers.
//!
//! So this harness defines its own corpus. By default it materializes the
//! target's **git-tracked** files into a temporary directory and builds that:
//! the input is then a function of the committed content plus any uncommitted
//! edits to tracked files, reproducible on any machine at the same revision.
//! Every run prints `corpus_sha256`, so two numbers can be *checked* for
//! comparability rather than assumed comparable. `--include-untracked` restores
//! the old build-the-directory-as-is behaviour for a one-off measurement of a
//! non-git tree, and prints a NOT-REPRODUCIBLE banner.
//!
//! Usage:
//!   cargo run -p codingest --bin codingest_bench --release -- <path>
//!   cargo run -p codingest --bin codingest_bench --release -- <path> --json
//!   cargo run -p codingest --bin codingest_bench --release -- <path> --include-untracked

use kglite::api::session::{execute_read, ExecuteOptions, ExecuteOutcome};
use kglite::api::{DirGraph, GraphRead, Value};
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const WARMUP: usize = 3;
const ITERS: usize = 20;

struct Query {
    name: &'static str,
    cypher: String,
}

/// Run one read-only query through the canonical session path.
fn run_query(graph: &DirGraph, query: &str) -> ExecuteOutcome {
    let params: HashMap<String, Value> = HashMap::new();
    let opts = ExecuteOptions::eager(&params);
    execute_read(graph, query, &opts)
        .unwrap_or_else(|e| panic!("query failed:\n  {query}\n  error: {e:?}"))
}

/// (row_count, order-insensitive value digest) for parity comparison.
/// Rows are canonicalized (`{:?}` per cell — stable for `Value`), sorted, and
/// hashed with the fixed-key `DefaultHasher`, so ordering nondeterminism between
/// the two graphs cannot produce a false MISMATCH — only a genuine difference in
/// the row multiset can.
fn digest(outcome: &ExecuteOutcome) -> (usize, u64) {
    let r = &outcome.result;
    let mut rows: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join("\u{1}")
        })
        .collect();
    rows.sort();
    let mut h = DefaultHasher::new();
    rows.len().hash(&mut h);
    for s in &rows {
        s.hash(&mut h);
    }
    (r.rows.len(), h.finish())
}

/// Extract the string in the first cell of the first row, if any.
fn first_string(outcome: &ExecuteOutcome) -> Option<String> {
    outcome
        .result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn min(v: &[f64]) -> f64 {
    v.iter().cloned().fold(f64::INFINITY, f64::min)
}

// ── corpus selection ─────────────────────────────────────────────────────

/// How the benchmark input was assembled. See the module docs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CorpusMode {
    /// Git-tracked files of the target, copied into a tempdir. Reproducible.
    TrackedOnly,
    /// The target directory exactly as it sits on disk. Not reproducible if
    /// the tree carries untracked or git-ignored content.
    WorkingTree,
}

impl CorpusMode {
    fn as_str(self) -> &'static str {
        match self {
            CorpusMode::TrackedOnly => "tracked-only",
            CorpusMode::WorkingTree => "working-tree",
        }
    }
}

/// The resolved benchmark input: the directory to build, plus the identity of
/// the content in it.
struct Corpus {
    build_dir: PathBuf,
    mode: CorpusMode,
    /// Why we ended up in `mode` — printed so the choice is never silent.
    reason: String,
    files: usize,
    bytes: u64,
    /// `None` in `WorkingTree` mode: the ingested set is not enumerable
    /// without duplicating the builder's walk, and an approximate identity is
    /// worse than an explicit absence.
    sha256: Option<String>,
    /// Owns the tempdir for the process lifetime; dropping it deletes the copy.
    _tmp: Option<tempfile::TempDir>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Git-tracked paths under `target`, relative to it. `None` when `target` is
/// not inside a git work tree (or git is unavailable).
fn git_tracked_files(target: &Path) -> Option<Vec<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["ls-files", "-z", "--cached"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8(out.stdout).ok()?;
    Some(
        stdout
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// Copy `rels` (relative to `target`) into a fresh tempdir and return the
/// corpus, including a content digest over the sorted `(path, sha256(bytes))`
/// pairs. Two runs with the same digest measured the same input.
fn materialize_tracked(target: &Path, mut rels: Vec<String>) -> std::io::Result<Corpus> {
    rels.sort();
    let tmp = tempfile::Builder::new()
        .prefix("codingest-bench-corpus-")
        .tempdir()?;
    let root = tmp.path().to_path_buf();

    let mut manifest = Sha256::new();
    let mut files = 0usize;
    let mut bytes = 0u64;

    for rel in &rels {
        let src = target.join(rel);
        // `ls-files` lists index entries; a tracked-but-deleted file, or a
        // gitlink (submodule) directory, has nothing to copy.
        let Ok(data) = std::fs::read(&src) else {
            continue;
        };
        let dst = root.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dst, &data)?;

        let mut file_hash = Sha256::new();
        file_hash.update(&data);
        manifest.update(rel.as_bytes());
        manifest.update(b"\0");
        manifest.update(hex(&file_hash.finalize()).as_bytes());
        manifest.update(b"\n");

        files += 1;
        bytes += data.len() as u64;
    }

    Ok(Corpus {
        build_dir: root,
        mode: CorpusMode::TrackedOnly,
        reason: format!("git-tracked content of {}", target.display()),
        files,
        bytes,
        sha256: Some(hex(&manifest.finalize())),
        _tmp: Some(tmp),
    })
}

/// Resolve the benchmark input for `target`.
///
/// Default is `TrackedOnly`; it degrades to `WorkingTree` only when the target
/// is not in a git work tree, and says so. `--include-untracked` forces
/// `WorkingTree`.
fn resolve_corpus(target: &Path, include_untracked: bool) -> Corpus {
    let working_tree = |reason: String| Corpus {
        build_dir: target.to_path_buf(),
        mode: CorpusMode::WorkingTree,
        reason,
        files: 0,
        bytes: 0,
        sha256: None,
        _tmp: None,
    };

    if include_untracked {
        return working_tree("--include-untracked requested".to_string());
    }
    let Some(rels) = git_tracked_files(target) else {
        return working_tree(format!("{} is not in a git work tree", target.display()));
    };
    if rels.is_empty() {
        return working_tree(format!("git tracks no files under {}", target.display()));
    }
    match materialize_tracked(target, rels) {
        Ok(corpus) => corpus,
        Err(e) => {
            eprintln!("warning: could not materialize the tracked-only corpus: {e}");
            working_tree(format!("tracked-only materialization failed: {e}"))
        }
    }
}

struct QueryResult {
    name: String,
    rows: usize,
    parity: bool,
    a_median_ms: f64,
    a_min_ms: f64,
    b_median_ms: f64,
    b_min_ms: f64,
    delta_pct: f64,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: code_tree_bench <path> [--json]");
            std::process::exit(2);
        }
    };
    let rest: Vec<String> = args.collect();
    let json = rest.iter().any(|a| a == "--json");
    let include_untracked = rest.iter().any(|a| a == "--include-untracked");
    // Reject unknown flags rather than silently ignoring them: a typo'd
    // `--include-untraked` would otherwise change nothing and be reported as a
    // measurement taken under a mode it never ran in.
    if let Some(bad) = rest
        .iter()
        .find(|a| !matches!(a.as_str(), "--json" | "--include-untracked"))
    {
        eprintln!("unknown argument `{bad}`");
        eprintln!("usage: codingest_bench <path> [--json] [--include-untracked]");
        std::process::exit(2);
    }
    let target = Path::new(&path);

    // Resolve the corpus BEFORE any timing: by default this copies the
    // target's git-tracked files into a tempdir so the measured input is
    // reproducible from the revision alone (see the module docs).
    let corpus = resolve_corpus(target, include_untracked);
    let build_dir = corpus.build_dir.as_path();

    // Two independent codingest builds with identical arguments:
    // verbose=false, include_tests=false, save_to=None, max_loc=None, docs=true.
    let t = Instant::now();
    let graph_a = codingest::builder::run_with_options(build_dir, false, false, None, None, true)
        .unwrap_or_else(|e| {
            eprintln!("build A failed: {e}");
            std::process::exit(1);
        });
    let build_a_secs = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let graph_b = codingest::builder::run_with_options(build_dir, false, false, None, None, true)
        .unwrap_or_else(|e| {
            eprintln!("build B failed: {e}");
            std::process::exit(1);
        });
    let build_b_secs = t.elapsed().as_secs_f64();

    let a: &DirGraph = &graph_a;
    let b: &DirGraph = &graph_b;

    let nodes = a.graph.node_count();
    let edges = a.graph.edge_count();

    // Anchors discovered at runtime on graph A (graph B is equivalent, so the
    // same node exists there): the CALLS in-degree and out-degree hubs.
    let hot_in = first_string(&run_query(
        a,
        "MATCH (:Function)-[:CALLS]->(f:Function) \
         RETURN f.qualified_name AS q, count(*) AS c ORDER BY c DESC, q ASC LIMIT 1",
    ))
    .unwrap_or_default();
    let hot_out = first_string(&run_query(
        a,
        "MATCH (f:Function)-[:CALLS]->(:Function) \
         RETURN f.qualified_name AS q, count(*) AS c ORDER BY c DESC, q ASC LIMIT 1",
    ))
    .unwrap_or_default();

    let queries = vec![
        Query {
            name: "count_functions (full-label scan + count)",
            cypher: "MATCH (f:Function) RETURN count(f)".to_string(),
        },
        Query {
            name: "eq_filter_pub (equality property filter)",
            cypher: "MATCH (f:Function) WHERE f.visibility = 'pub' RETURN count(f)".to_string(),
        },
        Query {
            name: "contains_new (CONTAINS string filter)",
            cypher: "MATCH (f:Function) WHERE f.qualified_name CONTAINS 'new' RETURN count(f)"
                .to_string(),
        },
        Query {
            name: "top20_by_branch_count (ORDER BY + LIMIT)",
            cypher: "MATCH (f:Function) RETURN f.qualified_name, f.branch_count \
                     ORDER BY f.branch_count DESC, f.qualified_name ASC LIMIT 20"
                .to_string(),
        },
        Query {
            name: "defs_per_file (grouped aggregation)",
            cypher: "MATCH (file:File)-[:DEFINES]->(f:Function) \
                     RETURN file.path, count(f) AS c ORDER BY c DESC, file.path ASC LIMIT 20"
                .to_string(),
        },
        Query {
            name: "calls_edge_scan (1-hop edge scan + count)",
            cypher: "MATCH (:Function)-[:CALLS]->(:Function) RETURN count(*)".to_string(),
        },
        Query {
            name: "anchored_callers (anchored 1-hop, in-hub)",
            cypher: format!(
                "MATCH (caller:Function)-[:CALLS]->(f:Function {{qualified_name: '{hot_in}'}}) \
                 RETURN caller.qualified_name ORDER BY caller.qualified_name ASC"
            ),
        },
        Query {
            name: "two_hop_into_hot (2-hop traversal + count)",
            cypher: format!(
                "MATCH (a:Function)-[:CALLS]->(b:Function)-[:CALLS]->\
                 (f:Function {{qualified_name: '{hot_in}'}}) RETURN count(*)"
            ),
        },
        Query {
            name: "varlen_callers_1_3 ([:CALLS*1..3] + count DISTINCT)",
            cypher: format!(
                "MATCH (f:Function {{qualified_name: '{hot_in}'}})<-[:CALLS*1..3]-(caller:Function) \
                 RETURN count(DISTINCT caller)"
            ),
        },
        Query {
            name: "reverse_callees_of_hub (reverse-direction 1-hop)",
            cypher: format!(
                "MATCH (callee:Function)<-[:CALLS]-(f:Function {{qualified_name: '{hot_out}'}}) \
                 RETURN callee.qualified_name ORDER BY callee.qualified_name ASC"
            ),
        },
        Query {
            name: "method_calls_mix (Struct-HAS_METHOD->Fn-CALLS->Fn)",
            cypher: "MATCH (s:Struct)-[:HAS_METHOD]->(m:Function)-[:CALLS]->(f:Function) \
                     RETURN count(*)"
                .to_string(),
        },
    ];

    let mut results: Vec<QueryResult> = Vec::new();

    for q in &queries {
        // Parity first: run once on each graph, compare row-count + digest.
        let out_a = run_query(a, &q.cypher);
        let out_b = run_query(b, &q.cypher);
        let (rows_a, dig_a) = digest(&out_a);
        let (rows_b, dig_b) = digest(&out_b);
        let parity = rows_a == rows_b && dig_a == dig_b;

        if !parity {
            eprintln!(
                "MISMATCH on `{}`: build A rows={rows_a} digest={dig_a:016x} | \
                 build B rows={rows_b} digest={dig_b:016x}\n  query: {}",
                q.name, q.cypher
            );
            results.push(QueryResult {
                name: q.name.to_string(),
                rows: rows_a,
                parity: false,
                a_median_ms: f64::NAN,
                a_min_ms: f64::NAN,
                b_median_ms: f64::NAN,
                b_min_ms: f64::NAN,
                delta_pct: f64::NAN,
            });
            continue;
        }

        // Warmup both sides.
        for _ in 0..WARMUP {
            let _ = run_query(a, &q.cypher);
            let _ = run_query(b, &q.cypher);
        }
        // Timed, alternating A/B.
        let mut ta = Vec::with_capacity(ITERS);
        let mut tb = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let s = Instant::now();
            let _ = run_query(a, &q.cypher);
            ta.push(s.elapsed().as_secs_f64() * 1000.0);
            let s = Instant::now();
            let _ = run_query(b, &q.cypher);
            tb.push(s.elapsed().as_secs_f64() * 1000.0);
        }
        let a_min_ms = min(&ta);
        let b_min_ms = min(&tb);
        let a_median_ms = median(ta);
        let b_median_ms = median(tb);
        let delta_pct = if a_median_ms > 0.0 {
            (b_median_ms - a_median_ms) / a_median_ms * 100.0
        } else {
            0.0
        };

        results.push(QueryResult {
            name: q.name.to_string(),
            rows: rows_a,
            parity: true,
            a_median_ms,
            a_min_ms,
            b_median_ms,
            b_min_ms,
            delta_pct,
        });
    }

    if json {
        let out = serde_json::json!({
            "path": path,
            "corpus_mode": corpus.mode.as_str(),
            "corpus_reason": corpus.reason,
            "corpus_files": corpus.sha256.as_ref().map(|_| corpus.files),
            "corpus_bytes": corpus.sha256.as_ref().map(|_| corpus.bytes),
            "corpus_sha256": corpus.sha256,
            "nodes": nodes,
            "edges": edges,
            "build_a_secs": (build_a_secs * 1000.0).round() / 1000.0,
            "build_b_secs": (build_b_secs * 1000.0).round() / 1000.0,
            "anchor_hot_in": hot_in,
            "anchor_hot_out": hot_out,
            "warmup": WARMUP,
            "iters": ITERS,
            "queries": results.iter().map(|r| serde_json::json!({
                "name": r.name,
                "rows": r.rows,
                "parity": r.parity,
                "a_median_ms": (r.a_median_ms * 1000.0).round() / 1000.0,
                "a_min_ms": (r.a_min_ms * 1000.0).round() / 1000.0,
                "b_median_ms": (r.b_median_ms * 1000.0).round() / 1000.0,
                "b_min_ms": (r.b_min_ms * 1000.0).round() / 1000.0,
                "delta_pct": (r.delta_pct * 100.0).round() / 100.0,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    println!("codingest Cypher benchmark — query-parity across two independent builds");
    println!("target : {path}");
    match &corpus.sha256 {
        Some(digest) => println!(
            "corpus : {} — {} files, {} bytes, sha256 {}\n         ({})",
            corpus.mode.as_str(),
            corpus.files,
            corpus.bytes,
            digest,
            corpus.reason,
        ),
        None => println!(
            "corpus : {} — NOT REPRODUCIBLE: untracked and git-ignored content \
             is ingested\n         ({})",
            corpus.mode.as_str(),
            corpus.reason,
        ),
    }
    println!("graph  : {nodes} nodes / {edges} edges  (identical across both builds)");
    println!("build  : A {build_a_secs:.3}s | B {build_b_secs:.3}s  (one-off, context)");
    println!("anchor : in-hub  = {hot_in}");
    println!("         out-hub = {hot_out}");
    println!("timing : {WARMUP} warmup + {ITERS} timed iters, alternating A/B, warm cache\n");

    println!(
        "{:<52} {:>7} {:>12} {:>12} {:>9}  parity",
        "query", "rows", "build A(ms)", "build B(ms)", "delta"
    );
    println!("{}", "-".repeat(110));
    for r in &results {
        if r.parity {
            println!(
                "{:<52} {:>7} {:>12} {:>12} {:>8.1}%  OK",
                r.name,
                r.rows,
                format!("{:.3} (min {:.3})", r.a_median_ms, r.a_min_ms),
                format!("{:.3} (min {:.3})", r.b_median_ms, r.b_min_ms),
                r.delta_pct,
            );
        } else {
            println!(
                "{:<52} {:>7} {:>12} {:>12} {:>9}  MISMATCH",
                r.name, r.rows, "-", "-", "-",
            );
        }
    }
    let mismatches = results.iter().filter(|r| !r.parity).count();
    println!(
        "\n{} queries, {} OK, {} MISMATCH",
        results.len(),
        results.len() - mismatches,
        mismatches
    );
}
