//! PyO3 wrapper for codingest — the `codingest` Python wheel.
//!
//! Resurrects the code-graph builder surface that KGLite's `kglite.code_tree`
//! module exposed before KGLite 0.14 removed it. `import codingest; g =
//! codingest.build(".")` parses a codebase with codingest's native builder and
//! returns a **real `kglite.KnowledgeGraph`** — the same object the separately
//! installed `kglite` wheel produces, so every downstream API (`.cypher()`,
//! `.describe()`, …) works unchanged.
//!
//! ## The `.kgl`-bytes handoff
//!
//! Two compiled extensions (this one and the `kglite` wheel) can't share live
//! Rust objects — each links its own copy of the engine. So the builder hands
//! off through serialization: it builds the `Arc<DirGraph>` at full native
//! speed, writes it to a `.kgl` file (`kglite::api::io::save_graph`), then calls
//! the *Python* `kglite.load(path)` (via `py.import("kglite")`) and returns that
//! object. Measured overhead of the save+load round-trip is ~12% of build time.
//! When the caller passes `save_to`, the `.kgl` is written there and kept;
//! otherwise a temp file carries the bytes and is deleted once loaded.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use kglite::api::DirGraph;

/// Serialize the built graph to a `.kgl` and load it back through the *Python*
/// `kglite` wheel, returning that wheel's own `KnowledgeGraph`.
///
/// This is the serialize half of the two-extension handoff (see module docs).
/// The GIL is held on entry (the `py.import` needs it); the save is pure Rust.
fn handoff_via_kgl(
    py: Python<'_>,
    mut graph: Arc<DirGraph>,
    save_to: Option<PathBuf>,
) -> PyResult<Py<PyAny>> {
    match save_to {
        // Caller wants the `.kgl` persisted — write there and load it back.
        Some(path) => {
            kglite::api::io::save_graph(&mut graph, &path.to_string_lossy())
                .map_err(PyRuntimeError::new_err)?;
            load_via_kglite(py, &path)
        }
        // No target path — carry the bytes through a temp file that is deleted
        // once the graph is fully materialized in the loaded object.
        None => {
            let tmp = tempfile::Builder::new()
                .prefix("codingest-")
                .suffix(".kgl")
                .tempfile()
                .map_err(|e| PyRuntimeError::new_err(format!("temp file for handoff: {e}")))?;
            kglite::api::io::save_graph(&mut graph, &tmp.path().to_string_lossy())
                .map_err(PyRuntimeError::new_err)?;
            let obj = load_via_kglite(py, tmp.path())?;
            // `tmp` drops here, removing the file; the graph now lives entirely
            // inside the returned kglite object.
            Ok(obj)
        }
    }
}

/// Import the installed `kglite` wheel and call its top-level `load(path)`.
fn load_via_kglite(py: Python<'_>, path: &Path) -> PyResult<Py<PyAny>> {
    let kglite = py.import("kglite").map_err(|e| {
        PyRuntimeError::new_err(format!(
            "codingest.build() returns a kglite.KnowledgeGraph, but importing \
             `kglite` failed ({e}). Install it: `pip install kglite>=0.15.11`."
        ))
    })?;
    let graph = kglite.call_method1("load", (path.to_string_lossy().as_ref(),))?;
    Ok(graph.unbind())
}

/// Map a file path to its codingest language identifier, or `None` if no parser
/// handles the file. Mirrors the old `kglite.language_for_path`.
#[pyfunction]
pub fn language_for_path(path: &str) -> Option<&'static str> {
    codingest::language_for_path(Path::new(path))
}

/// Parse a directory into a `kglite.KnowledgeGraph`.
///
/// See the type stub (`codingest/__init__.pyi`) for the full contract. `rev`
/// and `revs` are mutually exclusive; both build without touching the working
/// tree (git-archive into a tempdir). `save_to` writes the `.kgl` there and
/// still returns the loaded graph.
#[pyfunction]
#[pyo3(signature = (src_dir, *, save_to=None, verbose=false, include_tests=true, max_loc_per_file=None, include_docs=false, rev=None, revs=None, repo_root=None))]
#[allow(clippy::too_many_arguments)]
pub fn build(
    py: Python<'_>,
    src_dir: PathBuf,
    save_to: Option<PathBuf>,
    verbose: bool,
    include_tests: bool,
    max_loc_per_file: Option<usize>,
    include_docs: bool,
    rev: Option<String>,
    revs: Option<Vec<String>>,
    repo_root: Option<PathBuf>,
) -> PyResult<Py<PyAny>> {
    if rev.is_some() && revs.is_some() {
        return Err(PyValueError::new_err(
            "build(): `rev` and `revs` are mutually exclusive — pass one git \
             revision as `rev=`, or a list of revisions as `revs=[...]` to merge \
             into a multi-rev graph.",
        ));
    }
    // Build with the GIL released (pure-Rust, CPU-heavy). The builder saves
    // nothing here (`save_to=None`); the handoff owns the single save so the
    // temp-file and persist paths share one code path.
    let graph = py
        .detach(|| match (rev, revs) {
            (_, Some(revs)) => codingest::build_code_tree_revs(
                &src_dir,
                &revs,
                repo_root.as_deref(),
                verbose,
                include_tests,
                None,
                max_loc_per_file,
                include_docs,
            ),
            (Some(rev), None) => codingest::archive_and_build(
                &src_dir,
                &rev,
                repo_root.as_deref(),
                verbose,
                include_tests,
                None,
                max_loc_per_file,
                include_docs,
            ),
            (None, None) => codingest::build_code_tree(
                &src_dir,
                verbose,
                include_tests,
                None,
                max_loc_per_file,
                include_docs,
            ),
        })
        .map_err(PyRuntimeError::new_err)?;
    handoff_via_kgl(py, graph, save_to)
}

/// Clone a GitHub repo and build its `kglite.KnowledgeGraph`.
///
/// Cloning shells out to `git` (mirrors the old `kglite.repo_tree`). See the
/// type stub for the full keyword set.
#[pyfunction]
#[pyo3(signature = (
    repo,
    *,
    save_to=None,
    clone_to=None,
    branch=None,
    token=None,
    verbose=false,
    include_tests=true,
    max_loc_per_file=None,
    include_docs=false,
))]
#[allow(clippy::too_many_arguments)]
pub fn repo_tree(
    py: Python<'_>,
    repo: String,
    save_to: Option<PathBuf>,
    clone_to: Option<PathBuf>,
    branch: Option<String>,
    token: Option<String>,
    verbose: bool,
    include_tests: bool,
    max_loc_per_file: Option<usize>,
    include_docs: bool,
) -> PyResult<Py<PyAny>> {
    let graph = py
        .detach(|| {
            codingest::repo::clone_and_build(
                &repo,
                None,
                clone_to.as_deref(),
                branch.as_deref(),
                token.as_deref(),
                verbose,
                include_tests,
                max_loc_per_file,
                include_docs,
            )
        })
        .map_err(PyRuntimeError::new_err)?;
    handoff_via_kgl(py, graph, save_to)
}

/// Read a project manifest and return a dict of project metadata, or `None`
/// if no recognised manifest is found at `project_root`.
#[pyfunction]
pub fn read_manifest<'py>(
    py: Python<'py>,
    project_root: PathBuf,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let Some(info) =
        codingest::manifest::try_read_manifest(&project_root).map_err(PyRuntimeError::new_err)?
    else {
        return Ok(None);
    };
    let d = PyDict::new(py);
    d.set_item("name", info.name)?;
    d.set_item("version", info.version)?;
    d.set_item("description", info.description)?;
    d.set_item("languages", info.languages)?;
    d.set_item("authors", info.authors)?;
    d.set_item("license", info.license)?;
    d.set_item("repository_url", info.repository_url)?;
    d.set_item("manifest_path", info.manifest_path)?;
    d.set_item("build_system", info.build_system)?;
    let src_roots: Vec<String> = info
        .source_roots
        .iter()
        .map(|r| r.path.to_string_lossy().to_string())
        .collect();
    d.set_item("source_roots", src_roots)?;
    let test_roots: Vec<String> = info
        .test_roots
        .iter()
        .map(|r| r.path.to_string_lossy().to_string())
        .collect();
    d.set_item("test_roots", test_roots)?;
    Ok(Some(d))
}

/// Run the shared `codingest` CLI in-process and block until it exits.
///
/// The standalone `codingest-cli` binary and the `codingest` console script
/// bundled in this wheel both call the same pure-Rust library. `argv` excludes
/// the program name, which is synthesized here for clap. The Python shim owns
/// only console-script error formatting; all command behavior remains Rust-side.
#[pyfunction]
fn _run_cli(py: Python<'_>, argv: Vec<String>) -> PyResult<()> {
    let mut full = Vec::with_capacity(argv.len() + 1);
    full.push("codingest".to_string());
    full.extend(argv);
    py.detach(|| codingest_cli::run(full))
        .map_err(|e| PyRuntimeError::new_err(format!("{e:#}")))
}

/// Run the builder-aware Codingest MCP server in-process and block until exit.
///
/// The Python shim supplies argv without a program name and configures the
/// server's selftest respawn vector. Server behavior stays in the shared Rust
/// composition used by the standalone `codingest-mcp` binary.
#[pyfunction]
fn _run_mcp_server(py: Python<'_>, argv: Vec<String>) -> PyResult<()> {
    let mut full = Vec::with_capacity(argv.len() + 1);
    full.push("codingest-mcp".to_string());
    full.extend(argv);
    py.detach(|| codingest_mcp::run(full))
        .map_err(|e| PyRuntimeError::new_err(format!("{e:#}")))
}

/// The native extension module. Renamed to `codingest.codingest` by maturin's
/// `module-name`; the package `__init__.py` re-exports from it. The fn itself
/// is *not* named `codingest` — that would shadow the `codingest` core crate in
/// this module's path resolution — so `#[pyo3(name = ...)]` pins the Python
/// module name (and the `PyInit_codingest` symbol maturin links against).
#[pymodule]
#[pyo3(name = "codingest")]
fn codingest_ext(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(build, m)?)?;
    m.add_function(wrap_pyfunction!(repo_tree, m)?)?;
    m.add_function(wrap_pyfunction!(read_manifest, m)?)?;
    m.add_function(wrap_pyfunction!(language_for_path, m)?)?;
    m.add_function(wrap_pyfunction!(_run_cli, m)?)?;
    m.add_function(wrap_pyfunction!(_run_mcp_server, m)?)?;
    Ok(())
}
