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
use clap::Args;
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
}

/// A rendered query result: the bytes destined for stdout plus the row count
/// that the printing shell reports on stderr.
#[derive(Debug)]
pub(crate) struct QueryOutput {
    pub(crate) stdout: String,
    pub(crate) rows: usize,
}

pub(crate) fn run(args: &QueryArgs) -> Result<()> {
    let query = read_query(&args.query, std::io::stdin().lock())?;
    let output = run_query(&args.graph, &query, args.timeout)?;
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

/// Load `graph_path`, run `query` read-only, and render the result.
pub(crate) fn run_query(
    graph_path: &Path,
    query: &str,
    timeout: Option<f64>,
) -> Result<QueryOutput> {
    if !graph_path.exists() {
        anyhow::bail!(
            "graph artifact not found: {} — build one with `codingest build <dir>`",
            graph_path.display()
        );
    }
    let graph_text = graph_path.to_string_lossy().to_string();
    let graph = load_file(&graph_text)
        .with_context(|| format!("could not load graph artifact {}", graph_path.display()))?;

    let params: HashMap<String, Value> = HashMap::new();
    // `ExecuteOptions::eager` is mandatory here: the lazy path yields silently
    // empty row sets for any caller without a lazy materializer.
    let mut opts = ExecuteOptions::eager(&params).with_csv_import(CsvImportPolicy::LocalFilesystem);
    if let Some(seconds) = timeout {
        opts.deadline = Some(Instant::now() + Duration::from_secs_f64(seconds));
    }
    let outcome = execute_read(&graph, query, &opts).map_err(|e| anyhow::anyhow!("{e}"))?;

    let result = &outcome.result;
    Ok(QueryOutput {
        stdout: render_human(&result.columns, &result.rows),
        rows: result.rows.len(),
    })
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
        graph: PathBuf,
    }

    /// A one-file source tree built into a `.kgl` next to it.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let graph = dir.path().join("demo.kgl");
        build(&BuildArgs {
            source: dir.path().to_path_buf(),
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
        Fixture { _dir: dir, graph }
    }

    #[test]
    fn query_returns_rows_from_built_graph() {
        let fx = fixture();
        let out = run_query(
            &fx.graph,
            "MATCH (f:Function) RETURN f.name, f.qualified_name ORDER BY f.name ASC",
            None,
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
        let counted = run_query(&fx.graph, "MATCH (f:Function) RETURN count(f)", None).unwrap();
        assert_eq!(counted.stdout, "count(f)\n2\n");
        assert_eq!(counted.rows, 1);

        let listed = run_query(&fx.graph, "MATCH (f:File) RETURN f.path, labels(f)", None).unwrap();
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
        let error = run_query(&fx.graph, "CREATE (n:X {name: 'nope'})", None)
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
        let error = run_query(&missing, "MATCH (f:File) RETURN f.path", None)
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
