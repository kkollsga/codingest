//! code_tree accuracy harness — the measurement substrate for the
//! re-resolution phases.
//!
//! Builds the code graph for a repo and reports, as a single JSON object:
//! build wall-time, node/edge counts, and the CALLS-resolution breakdown
//! (`total_calls` → `excluded_noise` / `no_candidate` / `ambiguous_dropped`
//! / `resolved_call_sites`, plus the de-duplicated `resolved_edges`).
//!
//! The headline metric is `resolution_rate` = `resolved_call_sites` /
//! (`total_calls` - `excluded_noise`): of every call site that wasn't stdlib
//! noise, the fraction we pinned to at least one in-project symbol. Track it
//! across phases; re-resolution should push it up without moving build-time
//! on the default path.
//!
//! Usage:
//!   cargo run -p codingest --bin codingest_stats --release -- <path>
//!   cargo run -p codingest --bin codingest_stats --release -- <path> --include-tests
//!   cargo run -p codingest --bin codingest_stats --release -- <path> --function-metrics
//!   cargo run -p codingest --bin codingest_stats --release -- <path> --edge-breakdown
//!   cargo run -p codingest --bin codingest_stats --release -- <path> --dump-calls a,b,c
//!
//! `--edge-breakdown` appends a per-connection-type edge histogram to the JSON
//! object, with `IMPORTS` split by endpoint node type (`IMPORTS(File->File)`
//! vs `IMPORTS(File->Module)`) — the two are the same connection type but
//! answer different questions, and the File→File half is the dependency
//! conduit. `--dump-calls` emits the raw CALLS edges whose callee short-name
//! is in the supplied list, which is the substrate for labeling call-resolution
//! precision by hand. Both are read-only reporting over the built graph.

use kglite::api::GraphRead;
use kglite::api::{session, DirGraph, Value};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::time::Instant;

#[derive(Serialize)]
struct FunctionMetric {
    path: String,
    qualified_name: String,
    start_line: i64,
    end_line: i64,
    branch_count: i64,
    max_nesting: i64,
    is_test: bool,
}

fn string_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        other => panic!("expected string metric, got {other:?}"),
    }
}

fn int_value(value: &Value) -> i64 {
    match value {
        Value::Int64(value) => *value,
        Value::UniqueId(value) => i64::from(*value),
        Value::Null => 0,
        other => panic!("expected integer metric, got {other:?}"),
    }
}

fn bool_value(value: &Value) -> bool {
    match value {
        Value::Boolean(value) => *value,
        Value::Null => false,
        other => panic!("expected boolean metric, got {other:?}"),
    }
}

fn function_metrics(graph: &kglite::api::DirGraph) -> Vec<FunctionMetric> {
    let params = HashMap::new();
    let options = session::ExecuteOptions::eager(&params);
    let query = "MATCH (f:Function) RETURN f.file_path, f.qualified_name, \
                 f.line_number, f.end_line, f.branch_count, f.max_nesting, f.is_test";
    let outcome = session::execute_read(graph, query, &options).expect("query function metrics");
    let mut metrics: Vec<_> = outcome
        .result
        .rows
        .iter()
        .map(|row| FunctionMetric {
            path: string_value(&row[0]),
            qualified_name: string_value(&row[1]),
            start_line: int_value(&row[2]),
            end_line: int_value(&row[3]),
            branch_count: int_value(&row[4]),
            max_nesting: int_value(&row[5]),
            is_test: bool_value(&row[6]),
        })
        .collect();
    metrics.sort_by(|a, b| {
        (&a.path, &a.qualified_name, a.start_line).cmp(&(&b.path, &b.qualified_name, b.start_line))
    });
    metrics
}

// ── edge breakdown ───────────────────────────────────────────────────────

/// Histogram key for one edge. `IMPORTS` is the only connection type this
/// build emits between two *different* endpoint shapes (File→File and
/// File→Module), so it is the only one split; every other type collapses to
/// its bare connection name.
fn edge_breakdown_key(conn: &str, source_type: &str, target_type: &str) -> String {
    if conn == "IMPORTS" {
        format!("IMPORTS({source_type}->{target_type})")
    } else {
        conn.to_string()
    }
}

/// connection type (IMPORTS split by endpoint) → edge count. Sums to
/// `graph.edge_count()` by construction: every edge contributes exactly one key.
fn edge_breakdown(graph: &DirGraph) -> BTreeMap<String, usize> {
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    for e in graph.graph.edge_indices() {
        let Some(edge) = graph.graph.edge_weight(e) else {
            continue;
        };
        let Some((s, t)) = graph.graph.edge_endpoints(e) else {
            continue;
        };
        let (Some(sn), Some(tn)) = (graph.graph.node_weight(s), graph.graph.node_weight(t)) else {
            continue;
        };
        let key = edge_breakdown_key(
            edge.connection_type_str(&graph.interner),
            sn.node_type_str(&graph.interner),
            tn.node_type_str(&graph.interner),
        );
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

// ── CALLS dump (the labeling substrate) ──────────────────────────────────

/// Terminal segment of a `::` / `.` / `/`-separated qualified name — the bare
/// symbol a `--dump-calls` name list is matched against. Mirrors
/// `call_edges::short_type_name`'s rule (rightmost separator of any flavour
/// wins) so the dump selects exactly the names the resolver keyed on.
fn short_name(qname: &str) -> &str {
    let mut cut = 0usize;
    for sep in ["::", ".", "/"] {
        if let Some(i) = qname.rfind(sep) {
            let after = i + sep.len();
            if after > cut {
                cut = after;
            }
        }
    }
    &qname[cut..]
}

/// A node id as its bare string. `NodeData::id()` hands back a `Value`, whose
/// `Display` quotes strings (`"pkg.app.run"`), so the raw `to_string()` cannot
/// be short-name-split or compared against a qualified name.
fn id_string(id: std::borrow::Cow<'_, Value>) -> String {
    match id.as_ref() {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A string-valued property of a node/edge, or `""` when absent.
fn str_prop(props: &HashMap<String, Value>, key: &str) -> String {
    match props.get(key) {
        Some(Value::String(v)) => v.clone(),
        _ => String::new(),
    }
}

/// One CALLS edge, with both endpoints' files. The files are what makes the
/// dump self-sufficient as a labeling substrate: deciding whether an edge is a
/// true call means reading the caller's source at the recorded lines, and
/// whether the caller imports the callee's file is the structural signal
/// Phase 5's gate keys on.
#[derive(Serialize, PartialEq, Eq, Debug)]
struct CallRow {
    caller: String,
    callee: String,
    caller_file: String,
    callee_file: String,
    call_lines: String,
    /// Resolution metadata, empty/absent on edges that predate it or that the
    /// semantic pass produced. Carried here so the labeled truth set can be
    /// re-scored against a candidate suppression policy without rebuilding.
    resolution: String,
    candidates: i64,
    import_backed: bool,
}

/// Every CALLS edge whose callee short-name is in `names`, sorted by
/// (callee, caller) so two runs — and two builds — produce identical output.
fn dump_calls(graph: &DirGraph, names: &BTreeSet<String>) -> Vec<CallRow> {
    let mut rows: Vec<CallRow> = Vec::new();
    for e in graph.graph.edge_indices() {
        let Some(edge) = graph.graph.edge_weight(e) else {
            continue;
        };
        if edge.connection_type_str(&graph.interner) != "CALLS" {
            continue;
        }
        let Some((s, t)) = graph.graph.edge_endpoints(e) else {
            continue;
        };
        let (Some(sn), Some(tn)) = (graph.graph.node_weight(s), graph.graph.node_weight(t)) else {
            continue;
        };
        let callee = id_string(tn.id());
        if !names.contains(short_name(&callee)) {
            continue;
        }
        let props = edge.properties_cloned(&graph.interner);
        rows.push(CallRow {
            caller: id_string(sn.id()),
            callee,
            caller_file: str_prop(&sn.properties_cloned(&graph.interner), "file_path"),
            callee_file: str_prop(&tn.properties_cloned(&graph.interner), "file_path"),
            call_lines: str_prop(&props, "call_lines"),
            resolution: str_prop(&props, "resolution"),
            candidates: match props.get("candidates") {
                Some(Value::Int64(v)) => *v,
                _ => 0,
            },
            import_backed: matches!(props.get("import_backed"), Some(Value::Boolean(true))),
        });
    }
    rows.sort_by(|a, b| (&a.callee, &a.caller).cmp(&(&b.callee, &b.caller)));
    rows
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!(
                "usage: codingest_stats <path> [--include-tests] [--function-metrics] \
                 [--edge-breakdown] [--dump-calls name1,name2,...]"
            );
            std::process::exit(2);
        }
    };
    let options: Vec<_> = args.collect();
    let include_tests = options.iter().any(|a| a == "--include-tests");
    let emit_function_metrics = options.iter().any(|a| a == "--function-metrics");
    let emit_edge_breakdown = options.iter().any(|a| a == "--edge-breakdown");
    let dump_call_names: Option<BTreeSet<String>> = options
        .iter()
        .position(|a| a == "--dump-calls")
        .map(|i| match options.get(i + 1) {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            None => {
                eprintln!("--dump-calls requires a comma-separated name list");
                std::process::exit(2);
            }
        });

    let t = Instant::now();
    let (graph, stats) = match codingest::builder::run_with_options_stats(
        Path::new(&path),
        false,
        include_tests,
        None,
        None,
        false,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("build failed: {e}");
            std::process::exit(1);
        }
    };
    let build_secs = t.elapsed().as_secs_f64();

    if emit_function_metrics {
        println!(
            "{}",
            serde_json::to_string_pretty(&function_metrics(&graph)).unwrap()
        );
        return;
    }

    if let Some(names) = &dump_call_names {
        println!(
            "{}",
            serde_json::to_string_pretty(&dump_calls(&graph, names)).unwrap()
        );
        return;
    }

    let denom = stats.total_calls.saturating_sub(stats.excluded_noise);
    let resolution_rate = if denom > 0 {
        stats.resolved_call_sites as f64 / denom as f64
    } else {
        0.0
    };

    let mut out = serde_json::json!({
        "path": path,
        "include_tests": include_tests,
        "build_secs": (build_secs * 1000.0).round() / 1000.0,
        "nodes": graph.graph.node_count(),
        "edges": graph.graph.edge_count(),
        "total_calls": stats.total_calls,
        "excluded_noise": stats.excluded_noise,
        "no_candidate": stats.no_candidate,
        "ambiguous_dropped": stats.ambiguous_dropped,
        "resolved_call_sites": stats.resolved_call_sites,
        "resolved_via_inheritance": stats.resolved_via_inheritance,
        "resolved_edges": stats.resolved_edges,
        "resolution_rate": (resolution_rate * 10000.0).round() / 10000.0,
    });
    if emit_edge_breakdown {
        out["edge_breakdown"] = serde_json::to_value(edge_breakdown(&graph)).unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn corpus(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("tests")
            .join("corpus")
            .join(name)
    }

    fn build(name: &str) -> std::sync::Arc<DirGraph> {
        codingest::builder::run_with_options(&corpus(name), false, true, None, None, true)
            .expect("corpus build")
    }

    #[test]
    fn short_name_takes_the_rightmost_separator_of_any_flavour() {
        assert_eq!(short_name("pkg.util.helper"), "helper");
        assert_eq!(short_name("crate::graph::cypher::run"), "run");
        assert_eq!(short_name("packages/opencode/src/mcp/fetch"), "fetch");
        assert_eq!(short_name("bare"), "bare");
        // Mixed separators: the rightmost one wins regardless of flavour.
        assert_eq!(short_name("web/api.ts::Client.send"), "send");
    }

    #[test]
    fn edge_breakdown_splits_imports_and_accounts_for_every_edge() {
        // `agc_basic` is the ONLY committed corpus that currently produces
        // IMPORTS edges at all — the very blind spot this plan's `ts_monorepo`
        // corpus is added to close. Its single `#	INCLUDE` resolves to one
        // File→Module and one File→File edge.
        let graph = build("agc_basic");
        let breakdown = edge_breakdown(&graph);

        // Every edge lands in exactly one bucket.
        assert_eq!(
            breakdown.values().sum::<usize>(),
            graph.graph.edge_count(),
            "breakdown must account for every edge: {breakdown:?}"
        );

        // Pinned exactly — an instrument that mislabels a connection type or
        // collapses the IMPORTS split would still sum correctly, so the sum
        // alone is not a gate.
        assert_eq!(
            breakdown.get("IMPORTS(File->Module)").copied(),
            Some(1),
            "expected exactly one File→Module IMPORTS edge: {breakdown:?}"
        );
        assert_eq!(
            breakdown.get("IMPORTS(File->File)").copied(),
            Some(1),
            "expected exactly one File→File IMPORTS edge: {breakdown:?}"
        );
        assert!(
            !breakdown.contains_key("IMPORTS"),
            "IMPORTS must always be split by endpoint, never reported bare: {breakdown:?}"
        );
        // Representative unsplit types, to catch a key derivation that mangles
        // every connection name rather than just IMPORTS.
        assert_eq!(breakdown.get("CALLS").copied(), Some(4));
        assert_eq!(breakdown.get("DEFINES").copied(), Some(21));
    }

    #[test]
    fn dump_calls_selects_by_callee_short_name_and_sorts() {
        let graph = build("py_basic");

        // `py_basic.pkg.app.run` calls `helper` on line 5 of pkg/app.py.
        let rows = dump_calls(&graph, &BTreeSet::from(["helper".to_string()]));
        assert_eq!(
            rows,
            vec![CallRow {
                caller: "py_basic.pkg.app.run".into(),
                callee: "py_basic.pkg.util.helper".into(),
                caller_file: "pkg/app.py".into(),
                callee_file: "pkg/util.py".into(),
                call_lines: "5".into(),
                resolution: "unique_name".into(),
                candidates: 1,
                // No IMPORTS edge exists between these two files — Python
                // absolute imports do not resolve (filed separately), which is
                // exactly why this property must be read, not assumed.
                import_backed: false,
            }],
            "expected the single helper CALLS edge"
        );

        // A name nobody calls selects nothing (the filter is real, not a
        // pass-through) — `other` IS defined in the corpus but never called.
        assert!(dump_calls(&graph, &BTreeSet::from(["other".to_string()])).is_empty());
        assert!(dump_calls(&graph, &BTreeSet::new()).is_empty());

        // Sorted by (callee, caller) — proven on the corpus with the most
        // CALLS edges, which is also the one with several distinct callees.
        let graph = build("agc_basic");
        let all: BTreeSet<String> = graph
            .graph
            .edge_indices()
            .filter_map(|e| {
                let edge = graph.graph.edge_weight(e)?;
                if edge.connection_type_str(&graph.interner) != "CALLS" {
                    return None;
                }
                let (_, t) = graph.graph.edge_endpoints(e)?;
                Some(short_name(&id_string(graph.graph.node_weight(t)?.id())).to_string())
            })
            .collect();
        assert!(all.len() > 1, "need several distinct callees to prove sort");
        let rows = dump_calls(&graph, &all);
        assert_eq!(rows.len(), 4, "every CALLS edge must be selected");
        let keys: Vec<(String, String)> = rows
            .iter()
            .map(|r| (r.callee.clone(), r.caller.clone()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "dump must be (callee, caller)-sorted");
        for row in &rows {
            assert!(
                all.contains(short_name(&row.callee)),
                "row {row:?} escaped the name filter"
            );
            assert!(
                !row.call_lines.is_empty(),
                "row {row:?} lost its call_lines property"
            );
            assert!(
                row.caller_file.ends_with(".agc") && row.callee_file.ends_with(".agc"),
                "row {row:?} lost an endpoint file_path"
            );
        }
    }
}
