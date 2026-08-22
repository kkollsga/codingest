//! Pins the *answers* of variable-length traversals, not the graph bytes.
//!
//! `golden_parity` digests the graph the builder emits; `codingest_bench`
//! compares two builds on the SAME engine. Neither can see an engine-side
//! query-semantics change: both sides of every existing comparison move
//! together. This test closes that blind spot by asserting hand-derived
//! result SETS for the `<-[:CALLS*1..3]-` shape over the one corpus with a
//! call cycle (`rust_import`: `beta_uses_renamed` <-> `deep_fn`, pendant
//! `alpha_helper`).
//!
//! The expected sets encode kglite >= 0.16.6 TRAIL reachability (each
//! relationship used at most once per path; a closed trail re-emits its
//! source). On kglite 0.16.5 the two discriminating assertions fail — that
//! red run is this gate's proof it can fail.
//!
//! Recursion note: a self-call produces NO `:CALLS` self-edge — all four
//! emit sites in `builder/call_edges.rs` guard `target != caller_qn`, and
//! recursion is recorded as the `is_recursive` node property instead. That
//! is why this test needs the 2-cycle and would be vacuous on a
//! self-recursive fixture; `recursion_is_a_property_not_a_self_edge` pins
//! that invariant so the cycle assertions keep their meaning.

use kglite::api::session::{execute_read, ExecuteOptions};
use kglite::api::{DirGraph, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
        .unwrap_or_else(|e| panic!("[{name}] codingest build failed: {e}"))
}

fn string_rows(graph: &DirGraph, query: &str) -> Vec<String> {
    let params: HashMap<String, Value> = HashMap::new();
    let out = execute_read(graph, query, &ExecuteOptions::eager(&params))
        .unwrap_or_else(|e| panic!("query failed: {e}\n  {query}"));
    let mut rows: Vec<String> = out
        .result
        .rows
        .iter()
        .map(|r| match &r[0] {
            Value::String(s) => s.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    rows.sort();
    rows
}

fn callers_1_3(graph: &DirGraph, target: &str) -> Vec<String> {
    string_rows(
        graph,
        &format!(
            "MATCH (f:Function {{qualified_name: '{target}'}})<-[:CALLS*1..3]-(caller:Function) \
             RETURN DISTINCT caller.qualified_name"
        ),
    )
}

const BETA: &str = "crate::src::beta::beta_uses_renamed";
const DEEP: &str = "crate::src::nested::inner::deep_fn";
const ALPHA: &str = "crate::src::alpha::alpha_helper";

#[test]
fn varlen_callers_use_trail_reachability() {
    let g = build("rust_import");

    // Guard the name derivation itself: if qualified names ever change shape,
    // fail here with the real names rather than passing vacuously on empty
    // result sets below.
    let all = string_rows(&g, "MATCH (f:Function) RETURN f.qualified_name");
    for qn in [BETA, DEEP, ALPHA] {
        assert!(
            all.iter().any(|n| n == qn),
            "expected function {qn} not in graph; functions present: {all:?}"
        );
    }

    let both = vec![BETA.to_string(), DEEP.to_string()]; // sorted: "crate::src::b" < "crate::src::n"

    // Discriminating (red on kglite 0.16.5): a closed 2-trail returns to the
    // source, so trail semantics re-emit it as its own transitive caller.
    assert_eq!(
        callers_1_3(&g, BETA),
        both,
        "beta must be its own reachable caller via beta -> deep_fn -> beta"
    );
    assert_eq!(
        callers_1_3(&g, DEEP),
        both,
        "deep_fn must be its own reachable caller via deep_fn -> beta -> deep_fn"
    );

    // Control: no closed trail returns to alpha_helper, so its set is the same
    // under distance and trail semantics — re-emission must be targeted, not a
    // blanket "always include the source".
    assert_eq!(callers_1_3(&g, ALPHA), both);

    // Pendants: no incoming CALLS at all.
    assert!(callers_1_3(&g, "crate::src::root_uses_alpha").is_empty());
    assert!(callers_1_3(&g, "crate::src::nested::nested_uses_super").is_empty());
}

#[test]
fn recursion_is_a_property_not_a_self_edge() {
    // r_basic::deep_count is self-recursive in source. It must surface as
    // is_recursive = true with ZERO :CALLS self-edge — the invariant that
    // makes the cycle-based test above non-vacuous.
    let g = build("r_basic");

    let recursive = string_rows(
        &g,
        "MATCH (f:Function) WHERE f.is_recursive = true RETURN f.qualified_name",
    );
    assert!(
        recursive.iter().any(|n| n.ends_with("deep_count")),
        "deep_count should be flagged is_recursive; got {recursive:?}"
    );

    let self_edges = string_rows(
        &g,
        "MATCH (f:Function)-[:CALLS]->(f2:Function) WHERE f.qualified_name = f2.qualified_name \
         RETURN f.qualified_name",
    );
    assert!(
        self_edges.is_empty(),
        "a self-call must not produce a :CALLS self-edge; got {self_edges:?}"
    );
}
