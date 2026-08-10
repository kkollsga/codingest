//! CLI wrapper around the pure-Rust code-tree builder.

use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use codingest::{archive_and_build, build_code_tree, build_code_tree_revs};
use kglite::api::io::save_graph;
use kglite::api::DirGraph;
use serde_json::{json, Value};

const METADATA_FORMAT: u64 = 2;
pub(crate) const DEFAULT_GRAPH: &str = ".kglite/code-review.kgl";

#[derive(Subcommand, Debug)]
pub enum CodeTreeCommand {
    /// Parse a checkout or one or more git revisions into a `.kgl` graph.
    Build(BuildArgs),
    /// Check whether a built graph still matches its recorded source state.
    Status(StatusArgs),
    /// Run a read-only Cypher query against a saved graph artifact.
    #[command(visible_alias = "cypher")]
    Query(crate::query::QueryArgs),
    /// Install or remove the bundled code-review skill for an agent host.
    Skill {
        #[command(subcommand)]
        command: crate::skill::SkillCommand,
    },
}

#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Source directory or project manifest to parse.
    pub source: PathBuf,
    /// Artifact path. Defaults to `<source>/.kglite/code-review.kgl`.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Build committed content at one git revision.
    #[arg(long, conflicts_with = "revs")]
    pub rev: Option<String>,
    /// Merge committed content from several revisions, oldest to newest.
    #[arg(long, num_args = 1.., conflicts_with = "rev")]
    pub revs: Vec<String>,
    /// Override the git repository root used for revision builds.
    #[arg(long)]
    pub repo_root: Option<PathBuf>,
    /// Omit manifest-declared test roots.
    #[arg(long)]
    pub no_tests: bool,
    /// Include markdown documentation nodes and links.
    #[arg(long)]
    pub include_docs: bool,
    /// Skip parsing files above this line count while keeping File nodes.
    #[arg(long)]
    pub max_loc_per_file: Option<usize>,
    /// Print parser progress to stderr.
    #[arg(long)]
    pub verbose: bool,
    /// Status output format.
    #[arg(long, value_enum, default_value_t = StatusFormat::Human)]
    pub format: StatusFormat,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Graph artifact whose sidecar should be checked.
    #[arg(short, long, default_value = DEFAULT_GRAPH)]
    pub output: PathBuf,
    /// Status output format.
    #[arg(long, value_enum, default_value_t = StatusFormat::Human)]
    pub format: StatusFormat,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum StatusFormat {
    #[default]
    Human,
    Json,
}

pub fn run(command: &CodeTreeCommand) -> Result<()> {
    match command {
        CodeTreeCommand::Build(args) => {
            let status = build(args)?;
            print_status(&status, args.format);
        }
        CodeTreeCommand::Status(args) => {
            let status = status(&args.output)?;
            print_status(&status, args.format);
        }
        CodeTreeCommand::Query(args) => crate::query::run(args)?,
        CodeTreeCommand::Skill { command } => crate::skill::run(command)?,
    }
    Ok(())
}

struct BuildPlan {
    source: PathBuf,
    output: PathBuf,
    repo_root: Option<PathBuf>,
    include_tests: bool,
    mode: &'static str,
    revisions: Vec<String>,
}

pub(crate) fn build(args: &BuildArgs) -> Result<Value> {
    let plan = prepare_build(args)?;
    let graph = construct_graph(args, &plan)?;
    persist_build(args, &plan, graph)
}

fn prepare_build(args: &BuildArgs) -> Result<BuildPlan> {
    let source = args
        .source
        .canonicalize()
        .with_context(|| format!("source does not exist: {}", args.source.display()))?;
    let output = resolved_output(&source, args.output.as_deref());
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let repo_root = args
        .repo_root
        .as_ref()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()));
    let include_tests = !args.no_tests;
    let (mode, revisions) = if let Some(rev) = &args.rev {
        ("single-revision", vec![rev.clone()])
    } else if !args.revs.is_empty() {
        ("multi-revision", args.revs.clone())
    } else {
        ("working-tree", Vec::new())
    };

    Ok(BuildPlan {
        source,
        output,
        repo_root,
        include_tests,
        mode,
        revisions,
    })
}

fn construct_graph(args: &BuildArgs, plan: &BuildPlan) -> Result<Arc<DirGraph>> {
    let mut graph = match (args.rev.as_deref(), args.revs.is_empty()) {
        (Some(rev), _) => archive_and_build(
            &plan.source,
            rev,
            plan.repo_root.as_deref(),
            args.verbose,
            plan.include_tests,
            None,
            args.max_loc_per_file,
            args.include_docs,
        ),
        (None, false) => build_code_tree_revs(
            &plan.source,
            &args.revs,
            plan.repo_root.as_deref(),
            args.verbose,
            plan.include_tests,
            None,
            args.max_loc_per_file,
            args.include_docs,
        ),
        (None, true) => build_code_tree(
            &plan.source,
            args.verbose,
            plan.include_tests,
            None,
            args.max_loc_per_file,
            args.include_docs,
        ),
    }
    .map_err(anyhow::Error::msg)?;

    if plan.mode == "working-tree" {
        let instructions = format!(
            "Code graph built from the current working tree at {}. Refresh the artifact after source changes and verify review findings against exact source lines.",
            plan.source.display()
        );
        Arc::make_mut(&mut graph).set_instructions(&instructions, None);
    }
    Ok(graph)
}

/// Emit a stderr-only `[timing]` line for a once-per-invocation CLI cost.
///
/// Gated on `KGLITE_CODE_TREE_VERBOSE` — the same switch the builder's phase
/// timers use — rather than on `--verbose`. Two reasons, and they are why one
/// switch beats plumbing a flag: `source_fingerprint` also runs from `status`
/// and from every `query`'s freshness check, neither of which has a verbose
/// flag in scope; and a reader correlating a build's phase timings against its
/// persist cost should not have to know that the two halves answer to different
/// switches.
///
/// **stderr only, without exception.** `query --format json` and `status
/// --format json` write a machine-readable payload to stdout; a timing line on
/// that stream breaks every JSON consumer, and `query` runs this path on every
/// invocation.
fn mark(started: std::time::Instant, label: &str) {
    if std::env::var_os("KGLITE_CODE_TREE_VERBOSE").is_some() {
        eprintln!("[timing] {label}: {:.3}s", started.elapsed().as_secs_f64());
    }
}

fn persist_build(args: &BuildArgs, plan: &BuildPlan, mut graph: Arc<DirGraph>) -> Result<Value> {
    let output_text = plan.output.to_string_lossy().to_string();
    let t_save = std::time::Instant::now();
    save_graph(&mut graph, &output_text)
        .map_err(|e| anyhow::anyhow!("failed to save {}: {e}", plan.output.display()))?;
    mark(t_save, "save graph");

    let fingerprint = source_fingerprint(&plan.source, plan.repo_root.as_deref(), &plan.revisions)?;
    let (artifact_bytes, artifact_fingerprint) = artifact_fingerprint(&plan.output)?;
    let metadata = json!({
        "format": METADATA_FORMAT,
        "source": &plan.source,
        "output": &plan.output,
        "mode": plan.mode,
        "revisions": &plan.revisions,
        "repo_root": &plan.repo_root,
        "include_tests": plan.include_tests,
        "include_docs": args.include_docs,
        "max_loc_per_file": args.max_loc_per_file,
        "fingerprint": fingerprint,
        "artifact_bytes": artifact_bytes,
        "artifact_fingerprint": artifact_fingerprint,
    });
    let metadata_path = metadata_path(&plan.output);
    fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)
        .with_context(|| format!("could not write {}", metadata_path.display()))?;
    Ok(json!({
        "status": "built",
        "fresh": true,
        "graph": &plan.output,
        "metadata": metadata_path,
        "source": &plan.source,
        "mode": plan.mode,
        "revisions": metadata["revisions"],
        "bytes": artifact_bytes,
    }))
}

pub(crate) fn status(output: &Path) -> Result<Value> {
    let output = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    let sidecar = metadata_path(&output);
    if !output.exists() || !sidecar.exists() {
        return Ok(json!({
            "status": "missing",
            "fresh": false,
            "graph": output,
            "metadata": sidecar,
            "reason": "graph artifact or metadata sidecar is missing",
        }));
    }
    let metadata: Value = serde_json::from_slice(
        &fs::read(&sidecar).with_context(|| format!("could not read {}", sidecar.display()))?,
    )
    .with_context(|| format!("invalid metadata sidecar: {}", sidecar.display()))?;
    if metadata["format"].as_u64() != Some(METADATA_FORMAT) {
        return Ok(json!({
            "status": "stale",
            "fresh": false,
            "graph": output,
            "metadata": sidecar,
            "reason": "unsupported metadata format",
        }));
    }
    let source = PathBuf::from(
        metadata["source"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("metadata is missing source"))?,
    );
    let repo_root = metadata["repo_root"].as_str().map(PathBuf::from);
    let revisions: Vec<String> = metadata["revisions"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("metadata is missing revisions"))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let current = source_fingerprint(&source, repo_root.as_deref(), &revisions)?;
    let recorded = metadata["fingerprint"].as_str().unwrap_or("");
    let source_fresh = current == recorded;
    let (artifact_bytes, artifact_fingerprint) = artifact_fingerprint(&output)?;
    let recorded_bytes = metadata["artifact_bytes"].as_u64();
    let recorded_artifact = metadata["artifact_fingerprint"].as_str();
    let artifact_fresh = recorded_bytes == Some(artifact_bytes)
        && recorded_artifact == Some(artifact_fingerprint.as_str());
    let fresh = source_fresh && artifact_fresh;
    let reason = if !source_fresh {
        "source changed since the graph was built"
    } else if recorded_bytes != Some(artifact_bytes) {
        "graph artifact size changed since it was built"
    } else if !artifact_fresh {
        "graph artifact contents changed since it was built"
    } else {
        "source and graph artifact fingerprints match"
    };
    Ok(json!({
        "status": if fresh { "fresh" } else { "stale" },
        "fresh": fresh,
        "graph": output,
        "metadata": sidecar,
        "source": source,
        "mode": metadata["mode"],
        "revisions": revisions,
        "reason": reason,
    }))
}

fn resolved_output(source: &Path, output: Option<&Path>) -> PathBuf {
    output.map(Path::to_path_buf).unwrap_or_else(|| {
        if source.is_file() {
            source.parent().unwrap_or(source).join(DEFAULT_GRAPH)
        } else {
            source.join(DEFAULT_GRAPH)
        }
    })
}

fn metadata_path(output: &Path) -> PathBuf {
    let mut path: OsString = output.as_os_str().to_os_string();
    path.push(".meta.json");
    PathBuf::from(path)
}

fn print_status(status: &Value, format: StatusFormat) {
    match format {
        StatusFormat::Json => println!("{}", serde_json::to_string(status).expect("JSON value")),
        StatusFormat::Human => {
            let state = status["status"].as_str().unwrap_or("unknown");
            let graph = status["graph"].as_str().unwrap_or("");
            println!("{state}: {graph}");
            if let Some(reason) = status["reason"].as_str() {
                println!("{reason}");
            }
        }
    }
}

fn source_fingerprint(
    source: &Path,
    repo_root: Option<&Path>,
    revisions: &[String],
) -> Result<String> {
    let started = std::time::Instant::now();
    let mut hash = Fnv64::new();
    hash.update(source.to_string_lossy().as_bytes());
    if revisions.is_empty() {
        if source.is_file() {
            let root = source.parent().unwrap_or(source);
            hash_source_tree(root, &mut hash)?;
        } else {
            hash_source_tree(source, &mut hash)?;
        }
    } else {
        let root = repo_root.unwrap_or(source);
        for rev in revisions {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["rev-parse", "--verify"])
                .arg(format!("{rev}^{{commit}}"))
                .output()
                .with_context(|| "failed to run git for freshness check")?;
            if !output.status.success() {
                anyhow::bail!(
                    "could not resolve git revision {:?} in {}: {}",
                    rev,
                    root.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            hash.update(rev.as_bytes());
            hash.update(&output.stdout);
        }
    }
    // Success path only: an early bail already prints its own diagnostic, and a
    // partial walk's duration is not the cost this line exists to report.
    mark(started, "source fingerprint");
    Ok(format!("fnv1a64:{:016x}", hash.finish()))
}

/// Upper bound on fingerprint hashing threads.
///
/// This is IO-bound work over many small files, not tree-sitter recursion, so
/// it deliberately does **not** use the builder's `parser_pool` — those threads
/// carry 16 MB stacks for deep parse recursion, which is the wrong tool and the
/// wrong cost for reading files. A small fixed cap keeps a 128-core machine from
/// spawning 128 threads to read a repo that has 40 source files.
const MAX_FINGERPRINT_THREADS: usize = 8;

/// Whether `path`'s bytes can change the built graph — i.e. whether it belongs
/// in the freshness fingerprint.
///
/// Three sources, all **derived from the builder** rather than restated here,
/// because a restated list drifts the moment a language is added and drift in
/// this direction is invisible: a file the builder ingests but the fingerprint
/// ignores makes an edited repo report **fresh**, and the user gets stale query
/// answers with no error anywhere. `fingerprint_accepts_every_ingestible_input`
/// is the guard that fails when the derivation is broken.
///
/// **Not scoped by gitignore, deliberately.** The docs pass ingests every
/// `.md`/`.mdx`/`.rst` under the root whether or not git tracks it, so scoping
/// the fingerprint by gitignore would silently drop gitignored docs out of it —
/// the same false-fresh failure by another route.
///
/// Matching is case-insensitive even though `parsers::language_for_extension`
/// is case-sensitive: over-hashing a `FOO.RS` the builder would skip costs one
/// file read, while under-hashing costs correctness.
fn is_fingerprint_input(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        if codingest::manifest::GRAPH_SHAPING_MANIFESTS
            .iter()
            .any(|manifest| name.eq_ignore_ascii_case(manifest))
        {
            return true;
        }
    }
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    codingest::parsers::EXTENSION_MAP
        .iter()
        .any(|(candidate, _)| extension.eq_ignore_ascii_case(candidate))
        || codingest::DOC_EXTENSIONS
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

/// Fold every ingestible file under `root` into `hash`.
///
/// The walk is sequential (directory listing is cheap); the file reads are
/// spread across up to [`MAX_FINGERPRINT_THREADS`] threads. **Order is fixed
/// before any thread starts**: each file is reduced to its own digest, and the
/// digests are folded into `hash` in sorted relative-path order, so the result
/// does not depend on which thread finished first.
fn hash_source_tree(root: &Path, hash: &mut Fnv64) -> Result<()> {
    let mut files = Vec::new();
    collect_fingerprint_inputs(root, root, &mut files)?;
    files.sort_by(|(left, _), (right, _)| left.cmp(right));
    for digest in file_digests(&files)? {
        hash.update(&digest.to_le_bytes());
    }
    Ok(())
}

/// Recursive half of [`hash_source_tree`]: collect `(relative, absolute)` pairs
/// for the ingestible files under `dir`.
///
/// Directory pruning calls `codingest`'s own
/// [`codingest::manifest::is_ignored_dir_name`] rather than a local list, so
/// the fingerprint descends exactly where the builder's walk descends. As
/// there, the walk **root itself is never filtered** — pointing the tool at a
/// `target/` or a `.`-prefixed tempdir is an explicit request to read it.
fn collect_fingerprint_inputs(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<()> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("could not read source directory {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if codingest::manifest::is_ignored_dir_name(&name.to_string_lossy()) {
                continue;
            }
            collect_fingerprint_inputs(root, &path, out)?;
        } else if file_type.is_file() && is_fingerprint_input(&path) {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push((relative, path));
        }
    }
    Ok(())
}

/// Digest each file in `files`, preserving input order in the output.
fn file_digests(files: &[(PathBuf, PathBuf)]) -> Result<Vec<u64>> {
    let mut digests = vec![0_u64; files.len()];
    if files.is_empty() {
        return Ok(digests);
    }
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, MAX_FINGERPRINT_THREADS)
        .min(files.len());
    let chunk = files.len().div_ceil(threads);
    std::thread::scope(|scope| -> Result<()> {
        let handles: Vec<_> = files
            .chunks(chunk)
            .zip(digests.chunks_mut(chunk))
            .map(|(inputs, slots)| {
                scope.spawn(move || -> Result<()> {
                    for ((relative, path), slot) in inputs.iter().zip(slots.iter_mut()) {
                        *slot = file_digest(path, relative)?;
                    }
                    Ok(())
                })
            })
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(result) => result?,
                // A panic in a worker is a bug, not a fingerprint outcome:
                // re-raise it rather than reporting a digest computed from
                // partial reads.
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
        Ok(())
    })?;
    Ok(digests)
}

/// One file's contribution: its path *and* its bytes, so a rename with
/// identical content still moves the fingerprint.
fn file_digest(path: &Path, relative: &Path) -> Result<u64> {
    let mut hash = Fnv64::new();
    hash.update(relative.to_string_lossy().as_bytes());
    let mut file = fs::File::open(path)
        .with_context(|| format!("could not read source file {}", path.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finish())
}

fn artifact_fingerprint(path: &Path) -> Result<(u64, String)> {
    let bytes = fs::metadata(path)?.len();
    let mut hash = Fnv64::new();
    let mut file = fs::File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok((bytes, format!("fnv1a64:{:016x}", hash.finish())))
}

struct Fnv64(u64);

impl Fnv64 {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    /// The phase's real gate. Enumerate every input the builder can ingest —
    /// each parser extension, each docs extension, each graph-shaping manifest
    /// name — and assert the fingerprint accepts it.
    ///
    /// This is what makes "add a language" safe: a new entry in
    /// `EXTENSION_MAP` that the fingerprint does not see would otherwise ship a
    /// silent false-fresh (edit a `.foo`, `status` still says fresh), which no
    /// end-to-end test would catch because the graph itself is correct — only
    /// the *decision to rebuild it* is wrong.
    #[test]
    fn fingerprint_accepts_every_ingestible_input() {
        for (extension, language) in codingest::parsers::EXTENSION_MAP {
            let path = PathBuf::from(format!("src/file.{extension}"));
            assert!(
                is_fingerprint_input(&path),
                "parser extension .{extension} ({language}) is ingested but not fingerprinted"
            );
            let shouted = PathBuf::from(format!("src/file.{}", extension.to_uppercase()));
            assert!(
                is_fingerprint_input(&shouted),
                "extension matching must be case-insensitive: .{extension}"
            );
        }
        for extension in codingest::DOC_EXTENSIONS {
            assert!(
                is_fingerprint_input(&PathBuf::from(format!("docs/page.{extension}"))),
                "doc extension .{extension} is ingested but not fingerprinted"
            );
            assert!(
                is_fingerprint_input(&PathBuf::from(format!(
                    "docs/page.{}",
                    extension.to_uppercase()
                ))),
                "doc extension matching must be case-insensitive: .{extension}"
            );
        }
        for manifest in codingest::manifest::GRAPH_SHAPING_MANIFESTS {
            assert!(
                is_fingerprint_input(&PathBuf::from(format!("pkg/{manifest}"))),
                "manifest {manifest} shapes the graph but is not fingerprinted"
            );
        }
    }

    /// The reverse guard — the whole point of scoping. These are the bytes that
    /// dominated the old fingerprint (169 MB of 243 MB on the KGLite checkout)
    /// and can never reach the graph; hashing them made every rebuilt binary
    /// flip `status` to stale.
    #[test]
    fn fingerprint_rejects_never_ingested_files() {
        for name in [
            "target/debug/libdemo.so",
            "target/release/libdemo.dylib",
            "vendor/tool.jar",
            "docs/diagram.png",
            "notes.txt",
            "data.bin",
            "Makefile",
        ] {
            assert!(
                !is_fingerprint_input(&PathBuf::from(name)),
                "{name} cannot affect the graph and must not be fingerprinted"
            );
        }
    }

    /// Scoped by **ingestibility, not gitignore**. The docs pass reads every
    /// `.md` under the root regardless of git, so a gitignored one must still
    /// move the fingerprint — and a rebuilt `.so`, which no pass reads, must
    /// not.
    #[test]
    fn fingerprint_tracks_gitignored_docs_but_not_binaries() {
        let source = tempfile::tempdir().unwrap();
        let root = source.path();
        fs::write(root.join(".gitignore"), "notes.md\nlib.so\n").unwrap();
        fs::write(root.join("demo.rs"), "pub fn demo() {}\n").unwrap();
        fs::write(root.join("notes.md"), "# first\n").unwrap();
        fs::write(root.join("lib.so"), b"\x7fELF-one").unwrap();

        let baseline = source_fingerprint(root, None, &[]).unwrap();
        assert_eq!(
            source_fingerprint(root, None, &[]).unwrap(),
            baseline,
            "fingerprint must be stable across runs"
        );

        fs::write(root.join("lib.so"), b"\x7fELF-two-longer").unwrap();
        assert_eq!(
            source_fingerprint(root, None, &[]).unwrap(),
            baseline,
            "rebuilding a shared library must not report the source as stale"
        );

        fs::write(root.join("notes.md"), "# second\n").unwrap();
        assert_ne!(
            source_fingerprint(root, None, &[]).unwrap(),
            baseline,
            "a gitignored doc is still ingested, so editing it must flip the fingerprint"
        );
    }

    /// Digests are folded in sorted path order, so a repo big enough to be
    /// split across worker threads still fingerprints identically every time.
    #[test]
    fn fingerprint_is_order_independent_across_threads() {
        let source = tempfile::tempdir().unwrap();
        let root = source.path();
        for index in 0..200 {
            fs::write(
                root.join(format!("mod{index}.rs")),
                format!("pub fn f{index}() {{}}\n"),
            )
            .unwrap();
        }
        let first = source_fingerprint(root, None, &[]).unwrap();
        for _ in 0..4 {
            assert_eq!(source_fingerprint(root, None, &[]).unwrap(), first);
        }
    }

    fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("demo.rs"), "pub fn first() {}\n").unwrap();
        git(tmp.path(), &["init", "-q"]);
        git(tmp.path(), &["config", "user.email", "test@example.com"]);
        git(tmp.path(), &["config", "user.name", "Test"]);
        git(tmp.path(), &["add", "demo.rs"]);
        git(tmp.path(), &["commit", "-qm", "first"]);
        fs::write(
            tmp.path().join("demo.rs"),
            "pub fn first() {}\npub fn second() {}\n",
        )
        .unwrap();
        git(tmp.path(), &["add", "demo.rs"]);
        git(tmp.path(), &["commit", "-qm", "second"]);
        tmp
    }

    #[test]
    fn working_tree_build_reports_fresh_then_stale() {
        let parent = tempfile::tempdir().unwrap();
        let source = parent.path().join("source with spaces");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("demo.rs"), "pub fn demo() {}\n").unwrap();
        let output = source.join("code review.kgl");
        let result = build(&BuildArgs {
            source: source.clone(),
            output: Some(output.clone()),
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
        assert_eq!(result["fresh"], true);
        assert_eq!(status(&output).unwrap()["fresh"], true);
        let original = fs::read(&output).unwrap();
        fs::write(&output, &original[..original.len() / 2]).unwrap();
        let truncated = status(&output).unwrap();
        assert_eq!(truncated["fresh"], false);
        assert!(truncated["reason"].as_str().unwrap().contains("size"));
        fs::write(&output, &original).unwrap();
        let mut replaced = original.clone();
        let midpoint = replaced.len() / 2;
        replaced[midpoint] ^= 1;
        fs::write(&output, replaced).unwrap();
        let changed = status(&output).unwrap();
        assert_eq!(changed["fresh"], false);
        assert!(changed["reason"].as_str().unwrap().contains("contents"));
        fs::write(&output, original).unwrap();
        let sidecar = metadata_path(&output);
        let mut metadata: Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        metadata["format"] = Value::from(1);
        fs::write(&sidecar, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert_eq!(
            status(&output).unwrap()["reason"],
            "unsupported metadata format"
        );

        build(&BuildArgs {
            source: source.clone(),
            output: Some(output.clone()),
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
        fs::write(source.join("demo.rs"), "pub fn changed() {}\n").unwrap();
        assert_eq!(status(&output).unwrap()["fresh"], false);
    }

    #[test]
    fn multi_revision_build_and_bad_revision() {
        let repo = fixture();
        let output = repo.path().join("multi.kgl");
        let args = BuildArgs {
            source: repo.path().to_path_buf(),
            output: Some(output.clone()),
            rev: None,
            revs: vec!["HEAD~1".into(), "HEAD".into()],
            repo_root: None,
            no_tests: false,
            include_docs: false,
            max_loc_per_file: None,
            verbose: false,
            format: StatusFormat::Json,
        };
        assert_eq!(build(&args).unwrap()["mode"], "multi-revision");
        assert_eq!(status(&output).unwrap()["fresh"], true);

        let bad = BuildArgs {
            revs: vec!["not-a-revision".into()],
            ..args
        };
        let error = build(&bad).unwrap_err().to_string();
        assert!(error.contains("could not resolve git revision"));
    }
}
