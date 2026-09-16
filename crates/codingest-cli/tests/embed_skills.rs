//! `codingest build --embed-skills`: the opt-in graph-carried copy of the
//! code-review methodology, driven through the real binary.
//!
//! Off by default and attached only at the persist site, so a default build's
//! bytes — and every parity golden — are untouched; on, the artifact carries
//! one `KgliteSkill` and seven `KgliteRecipe` records and is byte-stable
//! across builds.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn codingest(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codingest"))
        .args(args)
        .output()
        .expect("failed to run the codingest binary")
}

fn source_tree() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("proj");
    std::fs::create_dir_all(source.join("src")).unwrap();
    std::fs::write(
        source.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        source.join("src/lib.rs"),
        "pub fn alpha() -> usize { beta() }\npub fn beta() -> usize { 1 }\n",
    )
    .unwrap();
    (dir, source)
}

fn build(source: &Path, graph: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "build",
        source.to_str().unwrap(),
        "-o",
        graph.to_str().unwrap(),
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    let out = codingest(&args);
    assert_eq!(out.status.code(), Some(0), "build failed: {out:?}");
    serde_json::from_slice(&out.stdout).expect("build --format json is JSON")
}

fn count(graph: &Path, cypher: &str) -> u64 {
    let out = codingest(&[
        "query",
        cypher,
        "-g",
        graph.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_eq!(out.status.code(), Some(0), "query failed: {out:?}");
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    payload["rows"][0][0].as_u64().expect("count row")
}

#[test]
fn default_build_carries_no_methodology_records() {
    let (_dir, source) = source_tree();
    let graph = source.join("plain.kgl");
    let report = build(&source, &graph, &[]);
    assert_eq!(report["embed_skills"], false);
    assert_eq!(count(&graph, "MATCH (s:KgliteSkill) RETURN count(s)"), 0);
    assert_eq!(count(&graph, "MATCH (r:KgliteRecipe) RETURN count(r)"), 0);
}

#[test]
fn embed_skills_writes_the_skill_and_seven_recipes_and_is_byte_stable() {
    let (_dir, source) = source_tree();
    let plain = source.join("plain.kgl");
    let first = source.join("embedded-1.kgl");
    let second = source.join("embedded-2.kgl");
    build(&source, &plain, &[]);
    let report = build(&source, &first, &["--embed-skills"]);
    build(&source, &second, &["--embed-skills"]);

    assert_eq!(report["embed_skills"], true);
    assert_eq!(count(&first, "MATCH (s:KgliteSkill) RETURN count(s)"), 1);
    assert_eq!(
        count(
            &first,
            "MATCH (s:KgliteSkill {name: 'code_review'}) RETURN count(s)"
        ),
        1
    );
    assert_eq!(count(&first, "MATCH (r:KgliteRecipe) RETURN count(r)"), 7);
    assert_eq!(
        count(
            &first,
            "MATCH (r:KgliteRecipe {recipe: 'code_review', name: 'target_coverage'}) RETURN count(r)"
        ),
        1
    );
    // Exactly the eight records on top of the plain graph — nothing else moved.
    let plain_nodes = count(&plain, "MATCH (n) RETURN count(n)");
    assert_eq!(count(&first, "MATCH (n) RETURN count(n)"), plain_nodes + 8);
    assert_eq!(
        count(&first, "MATCH (f:Function) RETURN count(f)"),
        count(&plain, "MATCH (f:Function) RETURN count(f)")
    );

    // Two flagged builds of the same source are byte-identical.
    let first_bytes = std::fs::read(&first).unwrap();
    let second_bytes = std::fs::read(&second).unwrap();
    assert_eq!(
        first_bytes, second_bytes,
        "embedded builds are not byte-stable"
    );
    assert_ne!(first_bytes, std::fs::read(&plain).unwrap());

    // The sidecar records the choice, and the artifact is still fresh.
    let sidecar: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("embedded-1.kgl.meta.json")).unwrap())
            .unwrap();
    assert_eq!(sidecar["embed_skills"], true);
    let status = codingest(&["status", "-o", first.to_str().unwrap(), "--format", "json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["fresh"], true, "{status}");
}
