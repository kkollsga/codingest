//! `codingest query` — one-shot Cypher read over a saved `.kgl` artifact.
//!
//! Deliberately no implicit build: `codingest build <dir> && codingest query
//! '<cypher>'` is shell composition, and loading a saved artifact is far
//! cheaper than rebuilding it per query. Read-only callers need no writer
//! lease and writers replace the `.kgl` atomically, so this composes with a
//! concurrent rebuild or a `codingest-mcp --graph` serving the same file.
//!
//! Output is never truncated — row budgeting belongs to Cypher `LIMIT`. The
//! MCP server's 15-row inline preview is a host-context budget; a pipe has no
//! such constraint.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use kglite::api::cypher::OutputFormat;
use kglite::api::io::load_file;
use kglite::api::param::kglite_value_to_json;
use kglite::api::session::{execute_read, CsvImportPolicy, ExecuteOptions};
use kglite::api::Value;

use crate::code_tree_cli::DEFAULT_GRAPH;

#[derive(Args, Debug)]
pub struct QueryArgs {
    /// Cypher query to run. `-` reads the query text from stdin.
    pub query: String,
    /// Graph artifact to query. The artifact is an input here, hence
    /// `--graph` rather than `build`/`status`'s output-shaped `--output`.
    #[arg(short, long, default_value = DEFAULT_GRAPH)]
    pub graph: PathBuf,
    /// Abort the query after this many seconds.
    #[arg(long)]
    pub timeout: Option<f64>,
    /// Result rendering. An in-query `FORMAT CSV` overrides this.
    #[arg(long, value_enum, default_value_t = QueryFormat::Human)]
    pub format: QueryFormat,
    /// Refuse to query a graph that is not provably fresh (exit code 3)
    /// instead of warning on stderr and running anyway.
    #[arg(long)]
    pub require_fresh: bool,
}

/// Result rendering. A separate enum from `StatusFormat` because the variants
/// differ — a query result has a CSV projection, a freshness verdict does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum QueryFormat {
    /// Header line of column names, then one TSV row per result row.
    #[default]
    Human,
    /// `CypherResult::to_csv()` — byte-identical to the MCP `FORMAT CSV` export.
    Csv,
    /// One compact `{"columns": [...], "rows": [[...]]}` object.
    Json,
}

/// A rendered query result: the bytes destined for stdout plus what the
/// printing shell reports on stderr. The freshness warning is returned as
/// data, not printed here, so the decision of which stream it lands on stays
/// in one place — and so tests need not capture stderr.
#[derive(Debug)]
pub(crate) struct QueryOutput {
    pub(crate) stdout: String,
    pub(crate) rows: usize,
    pub(crate) warning: Option<String>,
}

/// A `--require-fresh` refusal. Typed so `exit_code_for` can distinguish it
/// from an operational failure and give CI a dedicated exit code.
#[derive(Debug)]
pub struct StaleGraph {
    pub graph: PathBuf,
    pub reason: String,
}

impl std::fmt::Display for StaleGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "--require-fresh: refusing to query {} — {}",
            self.graph.display(),
            self.reason
        )
    }
}

impl std::error::Error for StaleGraph {}

pub(crate) fn run(args: &QueryArgs) -> Result<()> {
    let query = read_query(&args.query, std::io::stdin().lock())?;
    let output = run_query(args, &query)?;
    if let Some(warning) = &output.warning {
        eprintln!("warning: {warning}");
    }
    print!("{}", output.stdout);
    eprintln!("{} row(s)", output.rows);
    Ok(())
}

/// Resolve the positional query argument: `-` means "read stdin to EOF".
pub(crate) fn read_query(spec: &str, mut stdin: impl Read) -> Result<String> {
    if spec != "-" {
        return Ok(spec.to_string());
    }
    let mut text = String::new();
    stdin
        .read_to_string(&mut text)
        .context("could not read the query from stdin")?;
    Ok(text)
}

/// Load the artifact named by `args`, run `query` read-only, and render it.
pub(crate) fn run_query(args: &QueryArgs, query: &str) -> Result<QueryOutput> {
    let graph_path = args.graph.as_path();
    if !graph_path.exists() {
        anyhow::bail!(
            "graph artifact not found: {} — build one with `codingest build <dir>`",
            graph_path.display()
        );
    }

    let warning = freshness_warning(graph_path);
    if let (Some(reason), true) = (warning.as_deref(), args.require_fresh) {
        return Err(StaleGraph {
            graph: graph_path.to_path_buf(),
            reason: reason.to_string(),
        }
        .into());
    }

    let graph_text = graph_path.to_string_lossy().to_string();
    let graph = load_file(&graph_text)
        .with_context(|| format!("could not load graph artifact {}", graph_path.display()))?;

    let params: HashMap<String, Value> = HashMap::new();
    // `ExecuteOptions::eager` is mandatory here: the lazy path yields silently
    // empty row sets for any caller without a lazy materializer.
    let mut opts = ExecuteOptions::eager(&params).with_csv_import(CsvImportPolicy::LocalFilesystem);
    if let Some(seconds) = args.timeout {
        opts.deadline = Some(Instant::now() + Duration::from_secs_f64(seconds));
    }
    let outcome = execute_read(&graph, query, &opts).map_err(|e| anyhow::anyhow!("{e}"))?;

    // An in-query `FORMAT CSV` is a parser-level output switch that the MCP
    // server honors over its own default rendering; honoring it here too keeps
    // one query behaving the same on both interfaces.
    let effective = if outcome.output_format == OutputFormat::Csv {
        QueryFormat::Csv
    } else {
        args.format
    };
    let result = &outcome.result;
    let stdout = match effective {
        QueryFormat::Human => render_human(&result.columns, &result.rows),
        QueryFormat::Csv => result.to_csv(),
        QueryFormat::Json => render_json(&result.columns, &result.rows),
    };
    Ok(QueryOutput {
        stdout,
        rows: result.rows.len(),
        warning,
    })
}

/// `None` when the artifact is provably fresh; otherwise the text to warn with.
///
/// Note the third outcome: the sidecar check can *fail* rather than merely
/// report staleness — `source_fingerprint` errors outright when the recorded
/// source directory is unreadable or a recorded git rev no longer resolves,
/// which is exactly what a `.kgl` copied to another machine hits. That is a
/// "freshness unknown" warning, not a reason to refuse to query.
fn freshness_warning(graph_path: &Path) -> Option<String> {
    match crate::code_tree_cli::status(graph_path) {
        Ok(report) => {
            if report["fresh"] == serde_json::Value::Bool(true) {
                return None;
            }
            let state = report["status"].as_str().unwrap_or("unknown");
            let reason = report["reason"].as_str().unwrap_or("no reason recorded");
            Some(format!("graph is {state}: {reason}"))
        }
        Err(error) => Some(format!("freshness could not be verified: {error}")),
    }
}

/// One compact JSON object per query — the single-line convention
/// `status --format json` already set.
fn render_json(columns: &[String], rows: &[Vec<Value>]) -> String {
    let payload = serde_json::json!({
        "columns": columns,
        "rows": rows
            .iter()
            .map(|row| row.iter().map(kglite_value_to_json).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    });
    format!("{}\n", serde_json::to_string(&payload).expect("JSON value"))
}

/// Header line of column names, then one TSV row per result row — all rows.
fn render_human(columns: &[String], rows: &[Vec<Value>]) -> String {
    let mut out = String::new();
    out.push_str(&columns.join("\t"));
    out.push('\n');
    for row in rows {
        let cells: Vec<String> = row.iter().map(render_cell).collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    out
}

/// Strings print raw with the TSV-hostile control characters escaped; every
/// other variant serializes through the engine's canonical JSON projection.
fn render_cell(value: &Value) -> String {
    match value {
        Value::String(s) => s
            .replace('\t', "\\t")
            .replace('\n', "\\n")
            .replace('\r', "\\r"),
        other => kglite_value_to_json(other).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_tree_cli::{build, BuildArgs, StatusFormat};
    use std::fs;

    struct Fixture {
        _dir: tempfile::TempDir,
        source: PathBuf,
        graph: PathBuf,
    }

    /// A one-file source tree built into a `.kgl` that sits *beside* the tree,
    /// not inside it — so a test can delete the sources and exercise the
    /// "freshness could not be verified" path with the artifact still loadable.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("proj");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::create_dir(source.join("src")).unwrap();
        fs::write(
            source.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let graph = dir.path().join("demo.kgl");
        build(&BuildArgs {
            source: source.clone(),
            output: Some(graph.clone()),
            rev: None,
            revs: vec![],
            repo_root: None,
            no_tests: false,
            include_docs: false,
            max_loc_per_file: None,
            verbose: false,
            format: StatusFormat::Json,
        })
        .unwrap();
        Fixture {
            _dir: dir,
            source,
            graph,
        }
    }

    /// `QueryArgs` for a fixture graph. `query` is unused by `run_query` (the
    /// text is passed separately, already resolved from stdin if it was `-`).
    fn args(graph: &Path, format: QueryFormat) -> QueryArgs {
        QueryArgs {
            query: String::new(),
            graph: graph.to_path_buf(),
            timeout: None,
            format,
            require_fresh: false,
        }
    }

    #[test]
    fn query_returns_rows_from_built_graph() {
        let fx = fixture();
        let out = run_query(
            &args(&fx.graph, QueryFormat::Human),
            "MATCH (f:Function) RETURN f.name, f.qualified_name ORDER BY f.name ASC",
        )
        .unwrap();
        assert_eq!(
            out.stdout,
            "f.name\tf.qualified_name\n\
             alpha\tcrate::src::alpha\n\
             beta\tcrate::src::beta\n"
        );
        assert_eq!(out.rows, 2);
    }

    #[test]
    fn query_renders_non_string_cells_as_json() {
        let fx = fixture();
        let counted = run_query(
            &args(&fx.graph, QueryFormat::Human),
            "MATCH (f:Function) RETURN count(f)",
        )
        .unwrap();
        assert_eq!(counted.stdout, "count(f)\n2\n");
        assert_eq!(counted.rows, 1);

        let listed = run_query(
            &args(&fx.graph, QueryFormat::Human),
            "MATCH (f:File) RETURN f.path, labels(f)",
        )
        .unwrap();
        assert_eq!(listed.stdout, "f.path\tlabels(f)\nsrc/lib.rs\t[\"File\"]\n");
        assert_eq!(listed.rows, 1);
    }

    #[test]
    fn render_cell_escapes_tsv_hostile_control_characters() {
        assert_eq!(
            render_cell(&Value::String("a\tb\nc\rd".to_string())),
            "a\\tb\\nc\\rd"
        );
        assert_eq!(render_cell(&Value::Int64(-7)), "-7");
        assert_eq!(render_cell(&Value::Null), "null");
    }

    #[test]
    fn query_rejects_mutation_cypher() {
        let fx = fixture();
        let error = run_query(
            &args(&fx.graph, QueryFormat::Human),
            "CREATE (n:X {name: 'nope'})",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("execute_read called with a mutation query"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn query_missing_graph_names_path_and_hint() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.kgl");
        let error = run_query(
            &args(&missing, QueryFormat::Human),
            "MATCH (f:File) RETURN f.path",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(&missing.display().to_string()),
            "error omits the path: {error}"
        );
        assert!(
            error.contains("codingest build"),
            "error omits the build hint: {error}"
        );
    }

    /// Independently execute `query` and hand back the raw engine result, so a
    /// format test can compare the CLI's rendering against the engine's own
    /// projection rather than against a hand-copied string.
    fn engine_result(graph_path: &Path, query: &str) -> kglite::api::cypher::CypherResult {
        let graph = load_file(&graph_path.to_string_lossy()).unwrap();
        let params: HashMap<String, Value> = HashMap::new();
        let opts = ExecuteOptions::eager(&params);
        execute_read(&graph, query, &opts).unwrap().result
    }

    #[test]
    fn query_format_json_parses_and_matches() {
        let fx = fixture();
        let out = run_query(
            &args(&fx.graph, QueryFormat::Json),
            "MATCH (f:Function) RETURN f.name, f.qualified_name ORDER BY f.name ASC",
        )
        .unwrap();
        assert!(out.stdout.ends_with('\n'));
        assert_eq!(out.stdout.lines().count(), 1, "JSON must be one line");
        let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!({
                "columns": ["f.name", "f.qualified_name"],
                "rows": [
                    ["alpha", "crate::src::alpha"],
                    ["beta", "crate::src::beta"],
                ],
            })
        );
        assert_eq!(out.rows, 2);
    }

    #[test]
    fn query_format_json_projects_non_string_cells_naturally() {
        let fx = fixture();
        let out = run_query(
            &args(&fx.graph, QueryFormat::Json),
            "MATCH (f:File) RETURN f.path, labels(f)",
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(
            parsed["rows"],
            serde_json::json!([["src/lib.rs", ["File"]]]),
        );
    }

    #[test]
    fn query_format_csv_equals_result_to_csv() {
        let fx = fixture();
        let query = "MATCH (f:Function) RETURN f.name, f.qualified_name ORDER BY f.name ASC";
        let out = run_query(&args(&fx.graph, QueryFormat::Csv), query).unwrap();
        let expected = engine_result(&fx.graph, query).to_csv();
        assert_eq!(out.stdout, expected);
        assert_eq!(
            out.stdout,
            "f.name,f.qualified_name\nalpha,crate::src::alpha\nbeta,crate::src::beta\n"
        );
        assert_eq!(out.rows, 2);
    }

    #[test]
    fn query_inline_format_csv_overrides_flag() {
        let fx = fixture();
        // Two columns deliberately: a one-column CSV is byte-identical to the
        // TSV rendering, so the assertion would also pass on a Human fallback.
        let query = "MATCH (f:Function) RETURN f.name, f.qualified_name \
                     ORDER BY f.name ASC FORMAT CSV";
        for flag in [QueryFormat::Json, QueryFormat::Human] {
            let out = run_query(&args(&fx.graph, flag), query).unwrap();
            assert_eq!(
                out.stdout,
                "f.name,f.qualified_name\nalpha,crate::src::alpha\nbeta,crate::src::beta\n",
                "--format {flag:?} survived an in-query FORMAT CSV"
            );
        }
    }

    #[test]
    fn query_explain_renders_rows() {
        let fx = fixture();
        let out = run_query(
            &args(&fx.graph, QueryFormat::Human),
            "EXPLAIN MATCH (f:Function) RETURN f.name",
        )
        .unwrap();
        assert!(out.rows > 0, "EXPLAIN produced no plan rows");
        assert!(
            out.stdout.lines().count() > 1,
            "no rendered plan: {:?}",
            out
        );
    }

    const ROWS: &str = "MATCH (f:Function) RETURN f.name ORDER BY f.name ASC";
    const EXPECTED: &str = "f.name\nalpha\nbeta\n";

    #[test]
    fn query_fresh_graph_has_no_warning() {
        let fx = fixture();
        let out = run_query(&args(&fx.graph, QueryFormat::Human), ROWS).unwrap();
        assert_eq!(out.warning, None);
        assert_eq!(out.stdout, EXPECTED);
    }

    #[test]
    fn query_warns_on_stale_graph() {
        let fx = fixture();
        fs::write(fx.source.join("src/lib.rs"), "pub fn gamma() {}\n").unwrap();
        let out = run_query(&args(&fx.graph, QueryFormat::Human), ROWS).unwrap();
        assert_eq!(
            out.warning.as_deref(),
            Some("graph is stale: source changed since the graph was built")
        );
        // The stale graph is still queried — rows come from the artifact, not
        // from the source that moved underneath it.
        assert_eq!(out.stdout, EXPECTED);
        assert_eq!(out.rows, 2);
    }

    #[test]
    fn query_missing_sidecar_warns_but_runs() {
        let fx = fixture();
        let sidecar = fx.graph.with_extension("kgl.meta.json");
        assert!(sidecar.exists(), "fixture sidecar missing: {sidecar:?}");
        fs::remove_file(&sidecar).unwrap();
        let out = run_query(&args(&fx.graph, QueryFormat::Human), ROWS).unwrap();
        assert_eq!(
            out.warning.as_deref(),
            Some("graph is missing: graph artifact or metadata sidecar is missing")
        );
        assert_eq!(out.stdout, EXPECTED);
    }

    #[test]
    fn query_unverifiable_freshness_warns_but_runs() {
        let fx = fixture();
        // A `.kgl` whose recorded source tree is gone — the copied-artifact
        // case. `status()` errors rather than reporting stale; that must
        // degrade to "freshness unknown", not kill the query.
        fs::remove_dir_all(&fx.source).unwrap();
        let out = run_query(&args(&fx.graph, QueryFormat::Human), ROWS).unwrap();
        let warning = out.warning.expect("no warning for an unverifiable graph");
        assert!(
            warning.starts_with("freshness could not be verified: "),
            "unexpected warning: {warning}"
        );
        assert_eq!(out.stdout, EXPECTED);
    }

    #[test]
    fn query_require_fresh_errors_on_stale() {
        let fx = fixture();
        fs::write(fx.source.join("src/lib.rs"), "pub fn gamma() {}\n").unwrap();
        let mut strict = args(&fx.graph, QueryFormat::Human);
        strict.require_fresh = true;
        let error = run_query(&strict, ROWS).unwrap_err();
        let stale = error
            .downcast_ref::<StaleGraph>()
            .expect("--require-fresh did not produce a typed StaleGraph");
        assert_eq!(stale.graph, fx.graph);
        assert_eq!(
            stale.reason,
            "graph is stale: source changed since the graph was built"
        );
        assert_eq!(crate::exit_code_for(&error), 3);
    }

    #[test]
    fn query_require_fresh_passes_on_fresh_graph() {
        let fx = fixture();
        let mut strict = args(&fx.graph, QueryFormat::Human);
        strict.require_fresh = true;
        let out = run_query(&strict, ROWS).unwrap();
        assert_eq!(out.stdout, EXPECTED);
    }

    #[test]
    fn exit_code_for_maps_stale_to_3_and_other_to_1() {
        let stale: anyhow::Error = StaleGraph {
            graph: PathBuf::from("/tmp/demo.kgl"),
            reason: "source changed".to_string(),
        }
        .into();
        assert_eq!(crate::exit_code_for(&stale), 3);
        assert_eq!(crate::exit_code_for(&anyhow::anyhow!("bad cypher")), 1);
        assert_eq!(
            crate::exit_code_for(&std::io::Error::from(std::io::ErrorKind::NotFound).into()),
            1
        );
    }

    #[test]
    fn query_reads_query_from_stdin_dash() {
        let piped = "MATCH (f:File) RETURN f.path\n";
        assert_eq!(read_query("-", piped.as_bytes()).unwrap(), piped);
        assert_eq!(
            read_query("MATCH (n) RETURN n", piped.as_bytes()).unwrap(),
            "MATCH (n) RETURN n"
        );
    }
}
