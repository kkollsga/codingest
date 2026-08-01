//! The `codingest query` exit-code contract, driven through the real binary.
//!
//! `exit_code_for` is unit-tested in-crate, but the mapping is only worth
//! anything if `main.rs` actually applies it — and `main` is not reachable from
//! a unit test. These drive the compiled binary and read the process status.

use std::path::Path;
use std::process::{Command, Output};

fn codingest(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codingest"))
        .args(args)
        .output()
        .expect("failed to run the codingest binary")
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("process terminated by a signal")
}

/// A built graph beside its source tree; returns (tempdir, source, graph).
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("proj");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir(source.join("src")).unwrap();
    std::fs::write(source.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    let graph = dir.path().join("demo.kgl");
    let built = codingest(&[
        "build",
        source.to_str().unwrap(),
        "-o",
        graph.to_str().unwrap(),
    ]);
    assert_eq!(code(&built), 0, "fixture build failed: {built:?}");
    (dir, source, graph)
}

fn query(graph: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["query", "MATCH (f:Function) RETURN f.name"];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["-g", graph.to_str().unwrap()]);
    codingest(&args)
}

#[test]
fn successful_query_exits_zero_with_rows_on_stdout_and_summary_on_stderr() {
    let (_dir, _source, graph) = fixture();
    let out = query(&graph, &[]);
    assert_eq!(code(&out), 0, "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "f.name\nalpha\n");
    // stdout stays pure data: the row summary belongs on stderr.
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 row(s)"));
}

#[test]
fn stale_graph_warns_on_stderr_but_still_exits_zero() {
    let (_dir, source, graph) = fixture();
    std::fs::write(source.join("src/lib.rs"), "pub fn gamma() {}\n").unwrap();
    let out = query(&graph, &[]);
    assert_eq!(code(&out), 0, "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning: graph is stale: source changed since the graph was built"),
        "no stale warning on stderr: {stderr}"
    );
    // The warning must not contaminate the data stream.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "f.name\nalpha\n");
}

#[test]
fn require_fresh_on_a_stale_graph_exits_three() {
    let (_dir, source, graph) = fixture();
    std::fs::write(source.join("src/lib.rs"), "pub fn gamma() {}\n").unwrap();
    let out = query(&graph, &["--require-fresh"]);
    assert_eq!(code(&out), 3, "{out:?}");
    assert!(out.stdout.is_empty(), "refusal wrote rows to stdout");
}

#[test]
fn require_fresh_on_a_fresh_graph_exits_zero() {
    let (_dir, _source, graph) = fixture();
    let out = query(&graph, &["--require-fresh"]);
    assert_eq!(code(&out), 0, "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "f.name\nalpha\n");
}

#[test]
fn operational_errors_exit_one() {
    let (dir, _source, graph) = fixture();

    let bad_cypher = codingest(&[
        "query",
        "MATCH (f:Function RETURN",
        "-g",
        graph.to_str().unwrap(),
    ]);
    assert_eq!(code(&bad_cypher), 1, "{bad_cypher:?}");

    let missing = dir.path().join("absent.kgl");
    let no_graph = codingest(&[
        "query",
        "MATCH (f:Function) RETURN f.name",
        "-g",
        missing.to_str().unwrap(),
    ]);
    assert_eq!(code(&no_graph), 1, "{no_graph:?}");
    let stderr = String::from_utf8_lossy(&no_graph.stderr);
    assert!(
        stderr.contains("codingest build"),
        "no build hint: {stderr}"
    );

    // A mutation is rejected by the engine's read path, not by a CLI policy
    // layer — and it is an operational failure, not a freshness refusal.
    let mutation = codingest(&["query", "CREATE (n:X)", "-g", graph.to_str().unwrap()]);
    assert_eq!(code(&mutation), 1, "{mutation:?}");
}

#[test]
fn usage_errors_exit_two() {
    let out = codingest(&["query"]);
    assert_eq!(code(&out), 2, "missing positional should be a usage error");
}

#[test]
fn malformed_timeout_is_a_usage_error_not_a_panic() {
    let (_dir, _source, graph) = fixture();
    // `-1` and `nan` used to reach `Duration::from_secs_f64` and abort the
    // process with 101 — off the documented 0/1/2/3 contract entirely. `1e30`
    // overflows `Duration` the same way. `0` is rejected by policy: "no
    // timeout" is the flag's absence, so a zero can only be a mistake.
    for value in ["-1", "nan", "1e30", "inf", "0", "banana"] {
        let out = query(&graph, &["--timeout", value]);
        assert_eq!(
            code(&out),
            2,
            "--timeout={value} did not exit 2 as a usage error: {out:?}"
        );
        assert!(
            out.stdout.is_empty(),
            "--timeout={value} wrote rows to stdout"
        );
    }
}

#[test]
fn an_expiring_timeout_still_exits_one() {
    let (_dir, _source, graph) = fixture();
    let out = query(&graph, &["--timeout", "0.000000001"]);
    assert_eq!(code(&out), 1, "expiring timeout changed exit code: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("timed out"),
        "no timeout diagnostic on stderr: {stderr}"
    );
}

#[test]
fn cypher_alias_is_visible_and_equivalent() {
    let (_dir, _source, graph) = fixture();
    let aliased = codingest(&[
        "cypher",
        "MATCH (f:Function) RETURN f.name",
        "-g",
        graph.to_str().unwrap(),
    ]);
    assert_eq!(code(&aliased), 0, "{aliased:?}");
    assert_eq!(String::from_utf8_lossy(&aliased.stdout), "f.name\nalpha\n");

    let help = codingest(&["--help"]);
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(
        text.contains("query") && text.contains("cypher"),
        "alias not discoverable in --help: {text}"
    );
}

#[test]
fn query_text_can_come_from_stdin() {
    use std::io::Write;
    use std::process::Stdio;

    let (_dir, _source, graph) = fixture();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codingest"))
        .args(["query", "-", "-g", graph.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"MATCH (f:Function) RETURN f.name\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(code(&out), 0, "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "f.name\nalpha\n");
}
