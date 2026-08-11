//! Optional docs pass for `code_tree`.
//!
//! Ingests a repo's prose documentation as `:Doc` nodes and links them to the
//! rest of the graph:
//!
//! - `(:Doc)-[:MENTIONS]->(:Function|:Class|:Struct|:Enum|:Trait|:Interface|:Constant)`
//!   — the *prize*: an agent can jump from a doc's prose to the symbol it
//!   describes (and back). Resolution is **conservative** — only strong code
//!   signals are considered (Markdown backtick spans / `::`-qualified names;
//!   reStructuredText `:func:`/`:class:`/… roles and ``` ``literals``` ```),
//!   matched to an exact `qualified_name` or a **unique** bare `name`; ambiguous
//!   / common-word tokens never link.
//! - `(:Doc)-[:DOCUMENTS]->(:Doc|:File)` — links from one doc to another doc or
//!   to a source file (the latter matched by **unique basename**, robust to
//!   source-root-relative path bases).
//!
//! Each `:Doc` node also carries a `kind` (readme / changelog / guide / …,
//! inferred from the filename) and a `headings` outline (JSON list).
//!
//! **Two markup formats** are understood: Markdown (`.md` and `.mdx`, parsed
//! via the OKF loader — frontmatter, markdown links) and reStructuredText
//! (`.rst`, parsed by the [`rst`] submodule — Sphinx is the dominant doc
//! toolchain for the scientific-Python ecosystem). `.mdx` is Markdown with
//! embedded JSX/ESM and shares the Markdown path unchanged; the JSX is inert
//! text to every extractor. Format-specific extraction (title / headings /
//! mention candidates / links) dispatches on [`DocFormat`]; everything
//! downstream (symbol index, edge emit) is shared. `.txt` is **not** a doc
//! format — see the reasoning in [`discover_docs`].
//!
//! Runs *after* the code nodes are loaded, so symbol resolution can find them.
//! Gated on the `okf` feature.
//!
//! Repo docs (READMEs, `docs/`, design notes) rarely carry YAML frontmatter, so
//! this ingests **all** `.md` / `.mdx` / `.rst` (`require_frontmatter = false`)
//! while honoring `kg_skip: true` markers (Markdown) and the same directory
//! pruning as the code walk (node_modules / target / hidden dirs). Doc bodies
//! are kept transiently for the link scan but not stored as node properties.

mod rst;

use kglite::api::mutation as maintain;
use kglite::api::DirGraph;
use kglite::datatypes::values::{DataFrame, Value};
use kglite::okf;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

// Copied verbatim from kglite's `okf::build::column_value`
// (crates/kglite/src/okf/build.rs) — `pub(crate)` there, so not reachable
// through kglite's public API. Coerces a property value for columnar storage:
// structured values (lists, maps) JSON-encode to a String; scalars pass through
// unchanged.
fn column_value(v: &Value) -> Value {
    match v {
        Value::List(_) | Value::Map(_) => Value::String(
            serde_json::to_string(&kglite::api::param::kglite_value_to_json(v)).unwrap_or_default(),
        ),
        other => other.clone(),
    }
}

// Copied verbatim from kglite's `parent_dir`
// (crates/kglite/src/okf/mod.rs) — `pub(crate)` there, so not reachable through
// kglite's public API. Directory portion of a concept-id (`""` at the bundle
// root).
fn parent_dir(concept_id: &str) -> &str {
    match concept_id.rfind('/') {
        Some(i) => &concept_id[..i],
        None => "",
    }
}

/// Last `/`-separated segment of a concept id / relative path. Mirrors the
/// private `okf::stem`, so the two agree on what a doc's fallback title is.
fn path_stem(rel_path: &str) -> &str {
    rel_path.rsplit('/').next().unwrap_or(rel_path)
}
use std::sync::OnceLock;
use walkdir::WalkDir;

/// Node label for ingested repo documentation (distinct from code nodes).
const DOC_LABEL: &str = "Doc";
/// Code node labels that carry resolvable symbols (id = `qualified_name`,
/// title = `name`). `MENTIONS` edges only ever target these.
const SYMBOL_LABELS: &[&str] = &[
    "Function",
    "Class",
    "Struct",
    "Enum",
    "Trait",
    "Interface",
    "Constant",
];
/// Doc → code symbol edge.
const MENTIONS_CONN: &str = "MENTIONS";
/// Doc → doc / doc → file edge.
const DOCUMENTS_CONN: &str = "DOCUMENTS";
/// File node label (id = `path`).
const FILE_LABEL: &str = "File";
/// Project node label (id = `name`).
const PROJECT_LABEL: &str = "Project";
/// Project → doc ownership edge. Structural, not semantic: one per `:Doc`
/// node, emitted whether or not the doc mentions any code. MENTIONS and
/// DOCUMENTS remain the semantic links and are untouched by it.
const HAS_DOC_CONN: &str = "HAS_DOC";
/// Heading-outline cap (keeps the `headings` property bounded).
const MAX_HEADINGS: usize = 64;

/// Common identifiers that appear as prose words — never link a bare token in
/// this set (a `qualified_name` exact match still wins, this only guards the
/// unique-bare-name fallback).
const STOP_WORDS: &[&str] = &[
    "build", "new", "get", "set", "run", "main", "test", "init", "default", "from", "into", "len",
    "name", "id", "value", "type", "self", "str", "ok", "err", "none", "some", "string", "result",
    "error", "config", "data", "node", "graph", "list", "map", "key", "item", "args", "path",
    "file", "add", "remove", "update", "create", "delete", "read", "write", "open", "close",
    "start", "stop", "next", "iter", "size", "count", "index",
];

/// `(source_label, target_label, conn_type)` → `[(source_id, target_id)]`.
type EdgeGroups = BTreeMap<(String, String, String), Vec<(String, String)>>;

/// Markup format of an ingested doc — selects the title / heading / mention /
/// link extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocFormat {
    Markdown,
    Rst,
}

/// One ingested documentation file, normalized across markup formats. `body` is
/// retained transiently for the link/mention scan; it is not stored on the node.
struct DocEntry {
    concept_id: String,
    file_path: String,
    title: String,
    body: String,
    /// Flattened frontmatter (Markdown only; empty for RST).
    props: Vec<(String, Value)>,
    format: DocFormat,
}

/// A candidate symbol token extracted from a doc body. `allow_fallback` permits
/// the unique-bare-name match (set for strong code signals — backtick spans,
/// RST roles — and cleared for weaker bare `::` prose, which requires an exact
/// `qualified_name`).
struct Candidate {
    token: String,
    allow_fallback: bool,
}

/// A resolved outbound documentation link target.
enum LinkTarget {
    /// Another doc, by extension-stripped `concept_id`.
    Doc(String),
    /// A source file, by repo-relative path (matched on unique basename).
    File(String),
}

/// Ingest the repo's docs as `:Doc` nodes and link them to code + each other.
/// `graph` already contains the code nodes.
///
/// `project` is the `:Project` node's id (its name) when the build has one —
/// from a manifest or inferred from the root. Every ingested doc is anchored
/// to it with [`HAS_DOC_CONN`], which is what keeps docs attached to the graph
/// structurally even when they mention no code. `None` (no project could be
/// identified at all) simply skips the anchoring.
pub fn ingest_and_link(
    graph: &mut DirGraph,
    root: &Path,
    project: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    let docs = discover_and_parse(root)?;
    if docs.is_empty() {
        return Ok(());
    }
    add_doc_nodes(graph, &docs)?;
    let mentions = link_docs_to_code(graph, &docs)?;
    let documents = link_docs_to_docs_and_files(graph, &docs)?;
    let owned = match project {
        Some(project) => link_project_to_docs(graph, project, &docs)?,
        None => 0,
    };
    if verbose {
        let md = docs
            .iter()
            .filter(|d| d.format == DocFormat::Markdown)
            .count();
        let rst = docs.len() - md;
        eprintln!(
            "[docs] ingested {} doc(s) ({md} md/mdx, {rst} rst); {mentions} MENTIONS, {documents} DOCUMENTS, {owned} HAS_DOC edge(s)",
            docs.len()
        );
    }
    Ok(())
}

/// Anchor every ingested doc to the project that owns it: `Project HAS_DOC Doc`,
/// one edge per `:Doc` node, in discovery order. Doc identity is the
/// `concept_id`, and two docs can collapse onto one (a `.md`/`.mdx` pair
/// resolves to a single node), so ids are de-duplicated first — one node must
/// not receive two ownership edges.
fn link_project_to_docs(
    graph: &mut DirGraph,
    project: &str,
    docs: &[DocEntry],
) -> Result<usize, String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(docs.len());
    for d in docs {
        if seen.insert(d.concept_id.as_str()) {
            pairs.push((project.to_string(), d.concept_id.clone()));
        }
    }
    if pairs.is_empty() {
        return Ok(0);
    }
    let mut groups: EdgeGroups = EdgeGroups::new();
    groups.insert(
        (
            PROJECT_LABEL.to_string(),
            DOC_LABEL.to_string(),
            HAS_DOC_CONN.to_string(),
        ),
        pairs,
    );
    emit_groups(graph, groups)
}

// ── discovery + parsing ─────────────────────────────────────────────────────

/// A discovered doc file (before parsing): repo-relative path, abs path, format.
struct Discovered {
    rel_path: String,
    abs_path: PathBuf,
    format: DocFormat,
}

/// Walk `root` for `.md` / `.mdx` / `.rst` files, then parse each via its
/// format's extractor. Markdown reuses the OKF parser (frontmatter, `kg_skip`); RST uses
/// the [`rst`] submodule. Directory pruning matches the code walk
/// ([`crate::manifest::walk_filter`]).
fn discover_and_parse(root: &Path) -> Result<Vec<DocEntry>, String> {
    let found = discover_docs(root);

    // Markdown: hand the discovered `.md` / `.mdx` files to the OKF parser (it
    // computes titles from frontmatter / first heading, honors `kg_skip`,
    // flattens frontmatter). We bypass `okf::walk` so `.rst` shares one traversal and so
    // `index.md` is kept (the docs pass builds no Folder hierarchy).
    let md_opts = okf::BuildOptions {
        dialect: okf::Dialect::Okf,
        require_frontmatter: false,
        respect_skip: true,
        skip_dirs: Vec::new(),
        with_body: true,
        embed: false,
    };
    let md_files: Vec<okf::walk::DiscoveredFile> = found
        .iter()
        .filter(|d| d.format == DocFormat::Markdown)
        .map(|d| okf::walk::DiscoveredFile {
            rel_path: d.rel_path.clone(),
            abs_path: d.abs_path.clone(),
        })
        .collect();
    let mut docs: Vec<DocEntry> = okf::parse_concepts(&md_files, &md_opts)
        .into_iter()
        .map(|c| {
            // The OKF parser derives its `concept_id` by stripping a literal
            // lowercase `.md`, so any other markup extension we hand it —
            // `.mdx`, or an upper-cased `.MD` — would survive into the `:Doc`
            // node id. That matters beyond cosmetics: doc→doc links resolve by
            // comparing an extension-stripped link target against the set of
            // concept ids, so an id that kept its extension can never be linked
            // to. Re-derive it from the file path with the same
            // [`strip_doc_ext`] the RST path uses, so a `:Doc` id means the
            // same thing in every markup flavour.
            // `c.file_path` is verbatim the `rel_path` we passed in, so this
            // strips the file's real extension and is a no-op for plain `.md`.
            let concept_id = strip_doc_ext(&c.file_path).to_string();
            // OKF's last-resort title is the stem of *its* concept id, so an
            // extension it did not recognise would surface as the node title
            // ("intro.mdx"). Only that fallback is rewritten; a frontmatter
            // title or an `# H1` is left exactly as parsed.
            let title = if c.title == path_stem(&c.file_path) {
                path_stem(&concept_id).to_string()
            } else {
                c.title
            };
            DocEntry {
                concept_id,
                file_path: c.file_path,
                title,
                body: c.body.unwrap_or_default(),
                props: c.props,
                format: DocFormat::Markdown,
            }
        })
        .collect();

    // reStructuredText.
    for d in found.iter().filter(|d| d.format == DocFormat::Rst) {
        if let Some(entry) = rst::parse(&d.rel_path, &d.abs_path) {
            docs.push(entry);
        }
    }

    for c in sort_and_drop_collisions(&mut docs) {
        eprintln!(
            "[docs] warning: {} and {} both map to the doc id '{}'; keeping {} (.mdx > .md > .rst) and ignoring {}",
            c.kept, c.dropped, c.concept_id, c.kept, c.dropped
        );
    }
    Ok(docs)
}

/// The markup extensions the docs pass ingests, each with the extractor it
/// selects. **The single place that decides what counts as documentation**:
/// [`discover_docs`] asks it what to admit and [`strip_doc_ext`] asks it what a
/// `concept_id` drops, so the two cannot drift apart. They did drift: the
/// admission test was case-insensitive while the stripper matched six literal
/// suffixes, so a `Guide.Mdx` was ingested with its extension welded into its
/// id and could never be the target of a doc→doc link.
///
/// `.mdx` is Markdown plus embedded JSX/ESM. Every Markdown extractor
/// (frontmatter, headings, backtick mentions, `[](…)` links) reads it
/// unchanged; the JSX is inert text to all of them — `import {X} from "y"` and
/// `<Component />` carry no backtick span, no `::`-qualified prose and no
/// markdown link, so they contribute nothing rather than noise.
/// Astro/Starlight/Docusaurus sites keep their entire documentation set in
/// `.mdx`.
///
/// `.txt` is deliberately NOT here — do not "fix" this. Two independent
/// reasons:
///  1. No structure. A `.txt` file has no frontmatter, no heading syntax and
///     no link syntax, so every extractor degrades to nothing: the node would
///     carry a filename-derived title, an empty `headings` outline, no
///     MENTIONS and no DOCUMENTS.
///  2. The extension is indiscriminate. `requirements.txt`, `CMakeLists.txt`,
///     `LICENSE.txt`, vendored word lists and test fixtures all share it, so
///     ingesting it turns build inputs and data blobs into `:Doc` nodes.
///
/// Real prose does live in `.txt` (opencode ships 37 model-facing prompt files
/// that way), but a generic builder cannot separate those from the junk
/// without a manifest-driven opt-in, which is a known follow-up and
/// deliberately not built here.
const DOC_EXTENSIONS: &[(&str, DocFormat)] = &[
    ("md", DocFormat::Markdown),
    ("mdx", DocFormat::Markdown),
    ("rst", DocFormat::Rst),
];

/// The format `ext` (without its dot, any case) selects, or `None` if it is
/// not a doc extension.
fn doc_format_for_ext(ext: &str) -> Option<DocFormat> {
    DOC_EXTENSIONS
        .iter()
        .find(|(e, _)| ext.eq_ignore_ascii_case(e))
        .map(|(_, f)| *f)
}

/// Enumerate [`DOC_EXTENSIONS`] files under `root`, pruning hidden / build
/// dirs.
fn discover_docs(root: &Path) -> Vec<Discovered> {
    let mut out = Vec::new();
    let walker = WalkDir::new(root)
        .into_iter()
        .filter_entry(crate::manifest::walk_filter);
    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry.path().extension().and_then(|e| e.to_str());
        let Some(format) = ext.and_then(doc_format_for_ext) else {
            continue;
        };
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel_path = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join("/");
        out.push(Discovered {
            rel_path,
            abs_path: entry.path().to_path_buf(),
            format,
        });
    }
    out
}

/// Strip the trailing markup extension from a path, yielding the `concept_id`.
///
/// Only the segment after the LAST `.` is considered, and only if
/// [`DOC_EXTENSIONS`] admits it (case-insensitively, exactly as
/// [`discover_docs`] does). So `a.mdx.md` loses just its `.md` — the inner
/// `.mdx` is part of the name — and a path whose real extension is not a doc
/// extension (`notes.txt`), or which has none at all (`Makefile`), comes back
/// untouched.
fn strip_doc_ext(rel_path: &str) -> &str {
    match rel_path.rfind('.') {
        Some(i) if doc_format_for_ext(&rel_path[i + 1..]).is_some() => &rel_path[..i],
        _ => rel_path,
    }
}

/// Collision precedence for two docs whose paths strip to the same
/// `concept_id`: `.mdx` (0) beats `.md` (1) beats `.rst` (2). Lower wins.
///
/// The order is not arbitrary. An `.mdx` is a superset of Markdown, so where a
/// project ships both spellings of one page the `.mdx` is the live one and the
/// `.md` is what it was migrated from; `.rst` loses to both because a tree
/// mixing Sphinx and Markdown is mid-migration in the same direction.
fn ext_precedence(file_path: &str) -> u8 {
    match file_path.rfind('.').map(|i| &file_path[i + 1..]) {
        Some(e) if e.eq_ignore_ascii_case("mdx") => 0,
        Some(e) if e.eq_ignore_ascii_case("md") => 1,
        _ => 2,
    }
}

/// One dropped doc, reported by [`drop_colliding_docs`] for warning.
struct Collision {
    concept_id: String,
    kept: String,
    dropped: String,
}

/// Sort `docs` into ingest order and drop every doc whose `concept_id` a
/// higher-precedence doc already claims, returning one [`Collision`] per
/// dropped file.
///
/// The sort is by `concept_id` first — the order everything downstream sees —
/// then by the collision keys, so any two docs that strip to the same id land
/// adjacent with the winner first and the drop is a single adjacent-pair scan.
/// `file_path` breaks a tie between two equal-precedence spellings
/// (`guide.md` + `guide.MD`) and is unique per doc, so the order is total: the
/// surviving set never depends on the walk's arrival order.
///
/// Why dropping rather than merging: `add_doc_nodes` builds one DataFrame row
/// per doc and calls `add_nodes` with `conflict_handling = "update"`, so two
/// rows sharing a `concept_id` used to silently collapse — the LAST row won,
/// which meant the surviving node's title, `file_path` and frontmatter came
/// from whichever file the walk happened to reach second. The node count was
/// right, so nothing looked wrong.
fn sort_and_drop_collisions(docs: &mut Vec<DocEntry>) -> Vec<Collision> {
    docs.sort_by(|a, b| {
        a.concept_id
            .cmp(&b.concept_id)
            .then_with(|| ext_precedence(&a.file_path).cmp(&ext_precedence(&b.file_path)))
            .then_with(|| a.file_path.cmp(&b.file_path))
    });
    let mut collisions = Vec::new();
    let mut kept: Option<(String, String)> = None;
    docs.retain(|d| match &kept {
        Some((cid, keeper)) if *cid == d.concept_id => {
            collisions.push(Collision {
                concept_id: cid.clone(),
                kept: keeper.clone(),
                dropped: d.file_path.clone(),
            });
            false
        }
        _ => {
            kept = Some((d.concept_id.clone(), d.file_path.clone()));
            true
        }
    });
    collisions
}

// ── :Doc node materialisation ───────────────────────────────────────────────

/// Add one `:Doc` node per doc. Label is forced to `Doc` (repo docs aren't typed
/// concepts). Each node carries the flattened frontmatter plus a `kind`
/// (filename heuristic) and `headings` outline (JSON list). Mirrors `okf::build`'s
/// columnar add-nodes pattern.
fn add_doc_nodes(graph: &mut DirGraph, docs: &[DocEntry]) -> Result<(), String> {
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    for d in docs {
        for (k, _) in &d.props {
            keys.insert(k.as_str());
        }
    }
    let keys: Vec<&str> = keys.into_iter().collect();

    let mut columns = vec![
        "concept_id".to_string(),
        "title".to_string(),
        "file_path".to_string(),
        "kind".to_string(),
        "headings".to_string(),
    ];
    columns.extend(keys.iter().map(|k| k.to_string()));

    let mut rows = Vec::with_capacity(docs.len());
    for d in docs {
        let headings = doc_headings(d);
        let headings_val = if headings.is_empty() {
            Value::Null
        } else {
            column_value(&Value::List(
                headings.into_iter().map(Value::String).collect(),
            ))
        };
        let mut row = vec![
            Value::String(d.concept_id.clone()),
            Value::String(d.title.clone()),
            Value::String(d.file_path.clone()),
            Value::String(doc_kind(&d.concept_id)),
            headings_val,
        ];
        let pm: HashMap<&str, &Value> = d.props.iter().map(|(k, v)| (k.as_str(), v)).collect();
        for k in &keys {
            row.push(pm.get(k).map(|v| column_value(v)).unwrap_or(Value::Null));
        }
        rows.push(row);
    }

    let df = DataFrame::from_cypher_rows(columns, rows)?;
    maintain::add_nodes(
        graph,
        df,
        DOC_LABEL.to_string(),
        "concept_id".to_string(),
        Some("title".to_string()),
        Some("update".to_string()),
    )?;
    Ok(())
}

/// Classify a doc by its filename stem (lowercased). Captures the well-known
/// repo doc roles; everything else is `doc` (or `guide` under a `docs/` dir).
fn doc_kind(concept_id: &str) -> String {
    let stem = concept_id
        .rsplit('/')
        .next()
        .unwrap_or(concept_id)
        .to_ascii_lowercase();
    let in_docs_dir = concept_id
        .split('/')
        .any(|seg| matches!(seg.to_ascii_lowercase().as_str(), "docs" | "doc"));
    let kind = if stem.starts_with("readme") {
        "readme"
    } else if stem.starts_with("changelog")
        || stem == "changes"
        || stem == "history"
        || stem.starts_with("whats-new")
        || stem.starts_with("whatsnew")
    {
        "changelog"
    } else if stem.starts_with("contributing") {
        "contributing"
    } else if stem.starts_with("license") || stem.starts_with("licence") || stem == "copying" {
        "license"
    } else if stem.contains("code_of_conduct") || stem.contains("code-of-conduct") {
        "code_of_conduct"
    } else if stem.starts_with("security") {
        "security"
    } else if stem.starts_with("adr") || concept_id.to_ascii_lowercase().contains("/adr") {
        "adr"
    } else if in_docs_dir {
        "guide"
    } else {
        "doc"
    };
    kind.to_string()
}

/// Heading outline for a doc, dispatched on format.
fn doc_headings(d: &DocEntry) -> Vec<String> {
    match d.format {
        DocFormat::Markdown => markdown_headings(&d.body),
        DocFormat::Rst => rst::headings(&d.body),
    }
}

/// Markdown heading outline (`#`-prefixed), fenced code skipped, capped.
fn markdown_headings(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let h = rest.trim_start_matches('#').trim();
            if !h.is_empty() {
                out.push(h.to_string());
                if out.len() >= MAX_HEADINGS {
                    break;
                }
            }
        }
    }
    out
}

// ── Symbol index + MENTIONS ────────────────────────────────────────────────

/// Code node labels that *contain* methods. A symbol whose parent qualified-name
/// is one of these is a method (not module-level) — used to prefer a free
/// function over class methods when a bare name is ambiguous.
const CONTAINER_LABELS: &[&str] = &["Class", "Struct", "Enum", "Trait", "Interface"];

/// One resolvable code symbol: its qualified name + label, with a `method` flag
/// (parent qname is a container) so the resolver can prefer module-level defs.
#[derive(Clone)]
struct Symbol {
    qname: String,
    label: &'static str,
    method: bool,
}

/// Split a qualified name on both `.` (Python) and `::` (Rust) separators.
fn qname_segments(qname: &str) -> Vec<&str> {
    qname.split("::").flat_map(|s| s.split('.')).collect()
}

/// Resolvable code symbols, indexed for three matching strategies (most precise
/// first): exact `qualified_name`, dotted-suffix of a doc-supplied path, and
/// bare last-segment `name` (with module-level preference to disambiguate).
struct SymbolIndex {
    qname_to_label: HashMap<String, &'static str>,
    /// bare name → every same-named symbol (the disambiguation candidate set).
    by_name: HashMap<String, Vec<Symbol>>,
}

impl SymbolIndex {
    fn build(graph: &DirGraph) -> Self {
        let mut qname_to_label = HashMap::new();
        let mut by_name: HashMap<String, Vec<Symbol>> = HashMap::new();
        // Container qnames (Class/Struct/…) — a symbol whose parent is one of
        // these is a method. Collected first so the `method` flag is exact.
        let mut container_qnames: BTreeSet<String> = BTreeSet::new();
        for &label in CONTAINER_LABELS {
            if let Some(nodes) = graph.type_indices.get(label) {
                for idx in nodes.iter() {
                    if let Some(nd) = graph.get_node(idx) {
                        if let Value::String(q) = &*nd.id() {
                            container_qnames.insert(q.clone());
                        }
                    }
                }
            }
        }

        for &label in SYMBOL_LABELS {
            let Some(nodes) = graph.type_indices.get(label) else {
                continue;
            };
            for idx in nodes.iter() {
                let Some(nd) = graph.get_node(idx) else {
                    continue;
                };
                let qname = match &*nd.id() {
                    Value::String(s) => s.clone(),
                    _ => continue,
                };
                qname_to_label.entry(qname.clone()).or_insert(label);
                if let Value::String(name) = &*nd.title() {
                    if !name.is_empty() {
                        let method =
                            parent_qname(&qname).is_some_and(|p| container_qnames.contains(p));
                        by_name.entry(name.clone()).or_default().push(Symbol {
                            qname: qname.clone(),
                            label,
                            method,
                        });
                    }
                }
            }
        }
        SymbolIndex {
            qname_to_label,
            by_name,
        }
    }

    fn is_empty(&self) -> bool {
        self.qname_to_label.is_empty()
    }

    /// Resolve one candidate token to a `(qualified_name, label)` target, or
    /// `None`. `allow_name_fallback` enables the bare-name strategies (used for
    /// strong code signals — backtick spans, RST roles — which beat bare prose).
    ///
    /// Strategies, most precise first: (1) exact qualified-name; (2) when the
    /// token is itself a dotted path, a segment-aligned **suffix** match against
    /// a qualified name (`Dataset.mean` → `…core.dataset.Dataset.mean`); (3) a
    /// **unique** bare last-segment name; (4) when the bare name is ambiguous, a
    /// unique **module-level** def (free function over class methods — recovers
    /// re-exported top-level API like `concat` / `merge`).
    fn resolve(&self, token: &str, allow_name_fallback: bool) -> Option<(String, &'static str)> {
        if let Some(&label) = self.qname_to_label.get(token) {
            return Some((token.to_string(), label));
        }
        if !allow_name_fallback {
            return None;
        }
        let segs = qname_segments(token);
        let last = *segs.last()?;
        // Skip dunder / private names (`__init__`, `_helper`): no agent benefits
        // from a doc→`__init__` edge, and they're the main source of low-signal
        // matches. An exact `qualified_name` hit above is still honored.
        if last.len() < 3
            || last.starts_with('_')
            || STOP_WORDS.contains(&last.to_ascii_lowercase().as_str())
        {
            return None;
        }
        let cands = self.by_name.get(last)?;

        // (2) Dotted-suffix: the doc gave a path like `Type.method` — match it
        // segment-aligned against a single qualified name.
        if segs.len() > 1 {
            let mut hits = cands.iter().filter(|s| qname_ends_with(&s.qname, &segs));
            if let Some(first) = hits.next() {
                if hits.next().is_none() {
                    return Some((first.qname.clone(), first.label));
                }
            }
        }

        // (3) Unique bare name.
        if cands.len() == 1 {
            return Some((cands[0].qname.clone(), cands[0].label));
        }

        // (4) Ambiguous bare name → prefer a unique module-level def.
        let mut module_level = cands.iter().filter(|s| !s.method);
        if let Some(first) = module_level.next() {
            if module_level.next().is_none() {
                return Some((first.qname.clone(), first.label));
            }
        }
        None
    }
}

/// Parent qualified-name (strip the last `.`/`::` segment), or `None` at the
/// top level. The separator is whichever is rightmost (`::` for Rust, `.` for
/// Python).
fn parent_qname(qname: &str) -> Option<&str> {
    let dot = qname.rfind('.');
    let colon = qname.rfind("::");
    match (dot, colon) {
        (Some(d), Some(c)) => Some(&qname[..d.max(c)]),
        (Some(d), None) => Some(&qname[..d]),
        (None, Some(c)) => Some(&qname[..c]),
        (None, None) => None,
    }
}

/// Whether `qname`'s trailing segments equal `suffix` (segment-aligned).
fn qname_ends_with(qname: &str, suffix: &[&str]) -> bool {
    if suffix.is_empty() {
        return false;
    }
    let segs = qname_segments(qname);
    segs.len() >= suffix.len() && segs[segs.len() - suffix.len()..] == *suffix
}

/// Leading identifier path of a code span: `parse_wkt`, `Type::method`,
/// `mod.Class`. Anchored at the start of an already-extracted span. Shared with
/// the [`rst`] submodule.
fn ident_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.)[A-Za-z_][A-Za-z0-9_]*)*").unwrap()
    })
}

fn backtick_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"`([^`\n]+)`").unwrap())
}

fn qualified_prose_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Bare `::`-qualified names in prose: `KnowledgeGraph::cypher`.
    RE.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+").unwrap())
}

/// Collect symbol-mention candidates from a doc body, dispatched on format.
fn mention_candidates(d: &DocEntry) -> Vec<Candidate> {
    match d.format {
        DocFormat::Markdown => markdown_candidates(&d.body),
        DocFormat::Rst => rst::candidates(&d.body),
    }
}

/// Markdown mention candidates: backtick spans (fallback ON) + bare `::` prose
/// names (fallback OFF, exact qualified_name only). Fenced code skipped.
fn markdown_candidates(body: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for cap in backtick_re().captures_iter(line) {
            let span = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if let Some(m) = ident_path_re().find(span) {
                out.push(Candidate {
                    token: m.as_str().to_string(),
                    allow_fallback: true,
                });
            }
        }
        for m in qualified_prose_re().find_iter(line) {
            out.push(Candidate {
                token: m.as_str().to_string(),
                allow_fallback: false,
            });
        }
    }
    out
}

/// Build `(:Doc)-[:MENTIONS]->(:<symbol>)` edges. Returns the edge count.
fn link_docs_to_code(graph: &mut DirGraph, docs: &[DocEntry]) -> Result<usize, String> {
    let index = SymbolIndex::build(graph);
    if index.is_empty() {
        return Ok(0);
    }
    let mut groups: EdgeGroups = BTreeMap::new();
    for d in docs {
        let mut hits: BTreeSet<(String, &'static str)> = BTreeSet::new();
        for c in mention_candidates(d) {
            if let Some(hit) = index.resolve(&c.token, c.allow_fallback) {
                hits.insert(hit);
            }
        }
        for (qname, label) in hits {
            groups
                .entry((
                    DOC_LABEL.to_string(),
                    label.to_string(),
                    MENTIONS_CONN.to_string(),
                ))
                .or_default()
                .push((d.concept_id.clone(), qname));
        }
    }
    emit_groups(graph, groups)
}

// ── DOCUMENTS (doc → doc / doc → file) ─────────────────────────────────────

/// Collect outbound link targets from a doc body, dispatched on format.
fn doc_link_targets(d: &DocEntry) -> Vec<LinkTarget> {
    let src_dir = parent_dir(&d.concept_id);
    match d.format {
        DocFormat::Markdown => markdown_link_targets(&d.body, src_dir),
        DocFormat::Rst => rst::link_targets(&d.body, src_dir),
    }
}

/// Build `(:Doc)-[:DOCUMENTS]->(:Doc|:File)` edges. Doc targets match by exact
/// `concept_id`; file targets by **unique basename** (robust to source-root-
/// relative File ids). Returns the edge count.
fn link_docs_to_docs_and_files(graph: &mut DirGraph, docs: &[DocEntry]) -> Result<usize, String> {
    let doc_ids: BTreeSet<&str> = docs.iter().map(|d| d.concept_id.as_str()).collect();
    let file_by_basename = file_basename_index(graph);

    let mut groups: EdgeGroups = BTreeMap::new();
    for d in docs {
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        for target in doc_link_targets(d) {
            let edge = match target {
                LinkTarget::Doc(cid) => doc_ids
                    .contains(cid.as_str())
                    .then(|| (DOC_LABEL.to_string(), cid)),
                LinkTarget::File(path) => {
                    let base = path.rsplit('/').next().unwrap_or(&path);
                    file_by_basename
                        .get(base)
                        .and_then(|ids| (ids.len() == 1).then(|| ids[0].clone()))
                        .map(|id| (FILE_LABEL.to_string(), id))
                }
            };
            let Some((tgt_label, tgt_id)) = edge else {
                continue;
            };
            // Don't self-link a doc to itself.
            if tgt_label == DOC_LABEL && tgt_id == d.concept_id {
                continue;
            }
            if seen.insert((tgt_label.clone(), tgt_id.clone())) {
                groups
                    .entry((DOC_LABEL.to_string(), tgt_label, DOCUMENTS_CONN.to_string()))
                    .or_default()
                    .push((d.concept_id.clone(), tgt_id));
            }
        }
    }
    emit_groups(graph, groups)
}

/// `basename → [File node ids]` for unique-basename Doc→File resolution.
fn file_basename_index(graph: &DirGraph) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(nodes) = graph.type_indices.get(FILE_LABEL) {
        for idx in nodes.iter() {
            if let Some(nd) = graph.get_node(idx) {
                if let Value::String(path) = &*nd.id() {
                    let base = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                    out.entry(base).or_default().push(path.clone());
                }
            }
        }
    }
    out
}

fn markdown_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap())
}

/// Markdown link targets: `[text](dest)` → a Doc (`.md`/`.mdx` target) or File (other),
/// fenced code + image links skipped.
fn markdown_link_targets(body: &str, src_dir: &str) -> Vec<LinkTarget> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for raw in body.lines() {
        let t = raw.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for cap in markdown_link_re().captures_iter(raw) {
            let m = cap.get(0).unwrap();
            if m.start() > 0 && raw.as_bytes()[m.start() - 1] == b'!' {
                continue; // image link
            }
            let Some(dest) = cap.get(1) else { continue };
            let Some(target) = resolve_rel_path(dest.as_str(), src_dir) else {
                continue;
            };
            // A `.md` / `.mdx` destination names another doc; anything else is
            // a source file. (`.rst` is deliberately absent: a markdown link
            // into an RST tree is vanishingly rare and adding it here would
            // change existing edges for no measured gain.)
            match target
                .strip_suffix(".md")
                .or_else(|| target.strip_suffix(".mdx"))
            {
                Some(rest) => out.push(LinkTarget::Doc(rest.to_string())),
                None => out.push(LinkTarget::File(target)),
            }
        }
    }
    out
}

/// Resolve a relative/absolute link destination to a repo-relative path (keeping
/// any extension), or `None` for external / anchor-only / mailto links. Shared
/// with the [`rst`] submodule. `src_dir` is the linking doc's directory; a
/// leading `/` is treated as repo-root-absolute.
fn resolve_rel_path(dest: &str, src_dir: &str) -> Option<String> {
    let dest = dest.split(['#', '?']).next().unwrap_or(dest);
    if dest.is_empty() || dest.contains("://") || dest.starts_with("mailto:") {
        return None;
    }
    let combined = if let Some(abs) = dest.strip_prefix('/') {
        abs.to_string()
    } else if src_dir.is_empty() {
        dest.to_string()
    } else {
        format!("{src_dir}/{dest}")
    };
    let mut stack: Vec<&str> = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    let joined = stack.join("/");
    (!joined.is_empty()).then_some(joined)
}

// ── shared edge emit ───────────────────────────────────────────────────────

/// Emit grouped edges: one `add_connections` per `(src_label, tgt_label, conn)`
/// so every call has correctly-typed endpoints. Endpoints already exist (docs +
/// code nodes were added first) so no provisional stubs are created. Returns the
/// total edge count emitted.
fn emit_groups(graph: &mut DirGraph, groups: EdgeGroups) -> Result<usize, String> {
    let mut total = 0;
    for ((src_label, tgt_label, conn), pairs) in groups {
        total += pairs.len();
        let rows: Vec<Vec<Value>> = pairs
            .into_iter()
            .map(|(s, t)| vec![Value::String(s), Value::String(t)])
            .collect();
        let df = DataFrame::from_cypher_rows(
            vec!["source_id".to_string(), "target_id".to_string()],
            rows,
        )?;
        maintain::add_connections(
            graph,
            df,
            conn,
            src_label,
            "source_id".to_string(),
            tgt_label,
            "target_id".to_string(),
            None,
            None,
            Some("update".to_string()),
        )?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::run_with_options;
    use kglite::api::GraphRead;
    use std::fs;
    use tempfile::tempdir;

    /// The crate-root [`crate::DOC_EXTENSIONS`] exists so consumers compiled
    /// without the `docs` feature still know which files this pass would
    /// ingest. It is a second spelling of the same fact, so it is pinned to
    /// this module's table: adding `.adoc` here without adding it there would
    /// leave the CLI's freshness fingerprint blind to every AsciiDoc file —
    /// editing one would not flip the fingerprint, and the graph would read
    /// fresh with stale docs in it.
    #[test]
    fn doc_extensions_match_crate_root() {
        let from_table: Vec<&str> = DOC_EXTENSIONS.iter().map(|(ext, _)| *ext).collect();
        assert_eq!(from_table, crate::DOC_EXTENSIONS);
    }

    fn count_label(g: &DirGraph, label: &str) -> usize {
        g.graph
            .node_indices()
            .filter(|&n| {
                g.get_node(n)
                    .is_some_and(|nd| nd.node_type_str(&g.interner) == label)
            })
            .count()
    }

    fn count_conn(g: &DirGraph, conn: &str) -> usize {
        g.graph
            .edge_indices()
            .filter(|&e| {
                g.graph
                    .edge_weight(e)
                    .is_some_and(|w| w.connection_type_str(&g.interner) == conn)
            })
            .count()
    }

    /// Target-node titles for every edge of `conn` originating at a `Doc`.
    fn mention_target_names(g: &DirGraph, conn: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for e in g.graph.edge_indices() {
            let is_conn = g
                .graph
                .edge_weight(e)
                .is_some_and(|w| w.connection_type_str(&g.interner) == conn);
            if !is_conn {
                continue;
            }
            if let Some((_, tgt)) = g.graph.edge_endpoints(e) {
                if let Some(nd) = g.get_node(tgt) {
                    if let Value::String(name) = &*nd.title() {
                        out.insert(name.clone());
                    }
                }
            }
        }
        out
    }

    #[test]
    fn include_docs_adds_doc_nodes_only_when_enabled() {
        // Build inside a non-hidden subdir — tempdir() names dirs `.tmpXXXX`,
        // and code_tree's walk prunes hidden directories.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub fn parse_wkt() {}\npub struct Graph;",
        )
        .unwrap();
        fs::write(
            root.join("README.md"),
            "# Demo\nThe `parse_wkt` function parses WKT.",
        )
        .unwrap();

        // Without docs: no :Doc nodes (code still parsed).
        let g = run_with_options(&root, false, true, None, None, false).unwrap();
        assert_eq!(count_label(&g, "Doc"), 0);
        assert!(count_label(&g, "Function") >= 1, "code still parsed");

        // With docs: the README becomes a :Doc node.
        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        assert_eq!(count_label(&g, "Doc"), 1);
    }

    #[test]
    fn doc_mentions_link_to_symbols_conservatively() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub fn parse_wkt() {}\npub struct KnowledgeGraph;\npub fn run() {}\npub fn _internal() {}",
        )
        .unwrap();
        // `parse_wkt` (unique fn) and `KnowledgeGraph` (unique struct) link;
        // `run` is a stop-word and must NOT link; `_internal` is private and
        // must NOT link; `nonexistent` resolves to nothing.
        fs::write(
            root.join("README.md"),
            "# Guide\nCall `parse_wkt` then build a `KnowledgeGraph`.\n\
             Do not `run` this or `_internal`. The `nonexistent` symbol is absent.",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        let names = mention_target_names(&g, "MENTIONS");
        assert!(names.contains("parse_wkt"), "unique fn links");
        assert!(names.contains("KnowledgeGraph"), "unique struct links");
        assert!(!names.contains("run"), "stop-word must not link");
        assert!(!names.contains("_internal"), "private name must not link");
    }

    #[test]
    fn documents_links_doc_to_doc_and_file() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/engine.rs"), "pub fn go() {}").unwrap();
        fs::write(root.join("docs/design.md"), "# Design\nNotes.").unwrap();
        fs::write(
            root.join("README.md"),
            "# Project\nSee [design](docs/design.md) and the [engine](src/engine.rs).",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        // README → design (Doc) and README → engine.rs (File) = 2 DOCUMENTS.
        assert_eq!(count_conn(&g, "DOCUMENTS"), 2);
        // README classified as kind=readme.
        let readme_kind = g
            .graph
            .node_indices()
            .filter_map(|n| g.node_view(n))
            .find(|nd| {
                nd.node_type_str(&g.interner) == "Doc"
                    && matches!(&*nd.id(), Value::String(s) if s == "README")
            })
            .and_then(|nd| nd.get_field_ref("kind").map(|v| v.into_owned()));
        assert_eq!(readme_kind, Some(Value::String("readme".to_string())));
    }

    #[test]
    fn rst_docs_link_via_roles_and_doc_refs() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("python/pkg")).unwrap();
        fs::create_dir_all(root.join("doc")).unwrap();
        fs::write(
            root.join("python/pkg/core.py"),
            "def open_dataset():\n    pass\n\nclass DataArray:\n    pass\n",
        )
        .unwrap();
        fs::write(root.join("doc/io.rst"), "I/O\n===\nNotes.\n").unwrap();
        // RST roles (:func:/:class:) are explicit symbol refs; `:doc:` is a
        // doc cross-reference; the `~` prefix and `text <target>` forms resolve
        // to the underlying symbol.
        fs::write(
            root.join("doc/index.rst"),
            "xarray\n======\n\nLoad with :func:`~pkg.open_dataset` into a \
             :class:`DataArray`. See :doc:`io` for details.\n",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        assert!(count_label(&g, "Doc") >= 2, "two rst docs");
        let mentioned = mention_target_names(&g, "MENTIONS");
        assert!(mentioned.contains("open_dataset"), ":func: role links");
        assert!(mentioned.contains("DataArray"), ":class: role links");
        // index.rst :doc:`io` → doc/io  => one DOCUMENTS (Doc) edge.
        assert!(count_conn(&g, "DOCUMENTS") >= 1, ":doc: ref links doc->doc");
    }

    #[test]
    fn ambiguous_name_resolves_via_module_level_and_dotted_suffix() {
        // `concat` exists as a free function *and* as methods on two classes;
        // the free (module-level) def wins. `Dataset.mean` is a method — the
        // dotted path resolves it by segment-aligned suffix even though `mean`
        // alone would be ambiguous (two `mean` methods).
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(
            root.join("pkg/core.py"),
            "def concat():\n    pass\n\n\n\
             class Dataset:\n    def concat(self):\n        pass\n    def mean(self):\n        pass\n\n\n\
             class DataArray:\n    def concat(self):\n        pass\n    def mean(self):\n        pass\n",
        )
        .unwrap();
        fs::write(
            root.join("guide.rst"),
            "Guide\n=====\n\nUse :func:`concat` and :meth:`Dataset.mean`.\n",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        let targets: BTreeSet<String> = g
            .graph
            .edge_indices()
            .filter(|&e| {
                g.graph
                    .edge_weight(e)
                    .is_some_and(|w| w.connection_type_str(&g.interner) == "MENTIONS")
            })
            .filter_map(|e| g.graph.edge_endpoints(e).map(|(_, t)| t))
            .filter_map(|t| g.get_node(t))
            .filter_map(|nd| match &*nd.id() {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        // module-level `concat` (free fn), not a method.
        assert!(
            targets.iter().any(|q| q.ends_with("core.concat")),
            "module-level concat resolved, got {targets:?}"
        );
        // `Dataset.mean` via dotted suffix (not the DataArray.mean).
        assert!(
            targets.iter().any(|q| q.ends_with("Dataset.mean")),
            "Dataset.mean resolved by suffix, got {targets:?}"
        );
        assert!(
            !targets.iter().any(|q| q.ends_with("DataArray.mean")),
            "the other mean must not link"
        );
    }

    #[test]
    fn rst_title_from_section_heading() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn f() {}").unwrap();
        fs::write(
            root.join("guide.rst"),
            "Getting Started\n===============\n\nIntro.\n",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        let title = g
            .graph
            .node_indices()
            .filter_map(|n| g.get_node(n))
            .find(|nd| {
                nd.node_type_str(&g.interner) == "Doc"
                    && matches!(&*nd.id(), Value::String(s) if s == "guide")
            })
            .map(|nd| nd.title().into_owned());
        assert_eq!(title, Some(Value::String("Getting Started".to_string())));
    }

    /// Every `:Doc` id in `g`.
    fn doc_ids(g: &DirGraph) -> BTreeSet<String> {
        g.graph
            .node_indices()
            .filter_map(|n| g.get_node(n))
            .filter(|nd| nd.node_type_str(&g.interner) == "Doc")
            .filter_map(|nd| match &*nd.id() {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn strip_doc_ext_covers_mdx_in_both_cases() {
        assert_eq!(strip_doc_ext("docs/guide.mdx"), "docs/guide");
        assert_eq!(strip_doc_ext("docs/guide.MDX"), "docs/guide");
        // The pre-existing formats keep working, and the suffix match is exact
        // in both directions: `.md` must not swallow `.mdx`, and `.mdx` must
        // not swallow `.md`.
        assert_eq!(strip_doc_ext("README.md"), "README");
        assert_eq!(strip_doc_ext("README.MD"), "README");
        assert_eq!(strip_doc_ext("doc/io.rst"), "doc/io");
        // Only the trailing extension goes; an inner `.mdx` is part of the name.
        assert_eq!(strip_doc_ext("docs/a.mdx.md"), "docs/a.mdx");
        // Neither a non-doc extension nor a bare name is touched.
        assert_eq!(strip_doc_ext("notes.txt"), "notes.txt");
        assert_eq!(strip_doc_ext("Makefile"), "Makefile");
    }

    /// `discover_docs` accepts extensions case-insensitively, so mixed case is
    /// not an exotic input — it is whatever the filesystem happens to hold.
    /// Every spelling it admits must strip here, or the resulting `:Doc` id
    /// keeps its extension and can never be the target of a doc→doc link.
    #[test]
    fn strip_doc_ext_matches_discover_docs_case_insensitively() {
        assert_eq!(strip_doc_ext("docs/Guide.Mdx"), "docs/Guide");
        assert_eq!(strip_doc_ext("README.Md"), "README");
        assert_eq!(strip_doc_ext("doc/intro.Rst"), "doc/intro");
        assert_eq!(strip_doc_ext("NOTES.mD"), "NOTES");
        // Every extension `discover_docs` admits, in every case, must strip —
        // and nothing else may.
        for stem in ["md", "mdx", "rst"] {
            for ext in [stem.to_ascii_lowercase(), stem.to_ascii_uppercase()] {
                assert_eq!(strip_doc_ext(&format!("d/x.{ext}")), "d/x", "ext {ext}");
            }
        }
        // The pins from `strip_doc_ext_covers_mdx_in_both_cases` restated as
        // the boundary of the case-insensitive match: an inner extension and a
        // non-doc extension stay put whatever their case.
        assert_eq!(strip_doc_ext("docs/a.MDX.Md"), "docs/a.MDX");
        assert_eq!(strip_doc_ext("notes.TXT"), "notes.TXT");
        assert_eq!(strip_doc_ext("Makefile"), "Makefile");
    }

    fn doc_entry(rel_path: &str) -> DocEntry {
        let format = match doc_format_for_ext(rel_path.rsplit('.').next().unwrap()) {
            Some(f) => f,
            None => panic!("not a doc path: {rel_path}"),
        };
        DocEntry {
            concept_id: strip_doc_ext(rel_path).to_string(),
            file_path: rel_path.to_string(),
            title: rel_path.to_string(),
            body: String::new(),
            props: Vec::new(),
            format,
        }
    }

    #[test]
    fn collision_verdict_is_independent_of_discovery_order() {
        // Every arrival order of one colliding trio plus two innocents. The
        // walk's order is a filesystem property, so the policy is only a policy
        // if all of these agree.
        let paths = [
            "docs/guide.md",
            "docs/guide.mdx",
            "docs/guide.rst",
            "docs/intro.rst",
            "Notes.MD",
        ];
        for rotation in 0..paths.len() {
            let mut docs: Vec<DocEntry> = paths
                .iter()
                .cycle()
                .skip(rotation)
                .take(paths.len())
                .map(|p| doc_entry(p))
                .collect();
            let collisions = sort_and_drop_collisions(&mut docs);

            // The survivors, in ingest order: the `.mdx` won `docs/guide`, and
            // the two non-colliding docs are untouched.
            let kept: Vec<&str> = docs.iter().map(|d| d.file_path.as_str()).collect();
            assert_eq!(
                kept,
                ["Notes.MD", "docs/guide.mdx", "docs/intro.rst"],
                "rotation {rotation}"
            );
            assert!(
                docs.windows(2).all(|w| w[0].concept_id < w[1].concept_id),
                "survivors stay sorted by concept_id and unique"
            );

            // One warning per DROPPED file — not per collider, and not
            // repeated: the two losers of a three-way collision report once
            // each, naming both files.
            let reported: Vec<(&str, &str)> = collisions
                .iter()
                .map(|c| (c.kept.as_str(), c.dropped.as_str()))
                .collect();
            assert_eq!(
                reported,
                [
                    ("docs/guide.mdx", "docs/guide.md"),
                    ("docs/guide.mdx", "docs/guide.rst"),
                ],
                "rotation {rotation}"
            );
            assert!(collisions.iter().all(|c| c.concept_id == "docs/guide"));
        }
    }

    /// Read one property off the single `:Doc` node with id `id`.
    fn doc_field(g: &DirGraph, id: &str, field: &str) -> Option<Value> {
        g.graph
            .node_indices()
            .filter_map(|n| g.node_view(n))
            .find(|nd| {
                nd.node_type_str(&g.interner) == "Doc"
                    && matches!(&*nd.id(), Value::String(s) if s == id)
            })
            .and_then(|nd| nd.get_field_ref(field).map(|v| v.into_owned()))
    }

    #[test]
    fn same_stem_different_ext_collapses_to_the_highest_precedence_file() {
        // Three spellings of one concept id in one directory. They collide by
        // construction — `strip_doc_ext` maps all three to `docs/guide` — and
        // the policy is explicit precedence (`.mdx` > `.md` > `.rst`) with the
        // losers dropped, not last-write-wins on a duplicate DataFrame row.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub fn parse_wkt() {}\npub fn shadowedOnly() {}",
        )
        .unwrap();
        fs::write(
            root.join("docs/guide.mdx"),
            "---\ntitle: Guide (mdx wins)\n---\n\nCall `parse_wkt`.\n",
        )
        .unwrap();
        // `shadowedOnly` is mentioned ONLY by the two losers, so an edge to it
        // is proof a dropped doc still contributed to the graph.
        fs::write(
            root.join("docs/guide.md"),
            "# Guide (md loses)\n\nCall `shadowedOnly`.\n",
        )
        .unwrap();
        fs::write(
            root.join("docs/guide.rst"),
            "Guide (rst loses)\n=================\n\nSee :func:`shadowedOnly`.\n",
        )
        .unwrap();
        // A link written against the DROPPED `.md` spelling: link targets are
        // compared extension-stripped, so it must still reach the survivor.
        fs::write(
            root.join("README.md"),
            "# Project\nStart at the [guide](docs/guide.md).\n",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();

        // Exactly one `:Doc` for the colliding stem.
        assert_eq!(
            doc_ids(&g),
            BTreeSet::from(["README".to_string(), "docs/guide".to_string()])
        );
        // …and it is the `.mdx` file's node, properties and all.
        assert_eq!(
            doc_field(&g, "docs/guide", "title"),
            Some(Value::String("Guide (mdx wins)".into()))
        );
        assert_eq!(
            doc_field(&g, "docs/guide", "file_path"),
            Some(Value::String("docs/guide.mdx".into()))
        );
        // The dropped files contribute nothing at all — no mention of theirs
        // survives.
        let names = mention_target_names(&g, "MENTIONS");
        assert!(names.contains("parse_wkt"), "the survivor still mentions");
        assert!(
            !names.contains("shadowedOnly"),
            "a dropped collider must contribute no MENTIONS, got {names:?}"
        );
        // Doc→doc resolution still works across the collision.
        assert_eq!(count_conn(&g, "DOCUMENTS"), 1, "README -> docs/guide");
    }

    #[test]
    fn mdx_is_ingested_through_the_markdown_path() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub fn parse_wkt() {}\npub struct KnowledgeGraph;",
        )
        .unwrap();
        // Frontmatter + heading + a backtick mention, wrapped in the JSX/ESM an
        // `.mdx` file carries: an ESM import, a template-literal export and a
        // component block. None of those may contribute a mention or a link.
        fs::write(
            root.join("docs/guide.mdx"),
            "---\ntitle: Retry Guide\naudience: operators\n---\n\n\
             import { Tabs } from \"@astrojs/starlight/components\"\n\n\
             export const supportEmail = `mailto:ops@example.invalid`\n\n\
             ## Usage\n\n\
             <Tabs>\n  Call `parse_wkt` to build a `KnowledgeGraph`.\n</Tabs>\n",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();

        // The id is extension-stripped exactly like a `.md` doc's.
        assert_eq!(doc_ids(&g), BTreeSet::from(["docs/guide".to_string()]));

        let node = g
            .graph
            .node_indices()
            .filter_map(|n| g.node_view(n))
            .find(|nd| {
                nd.node_type_str(&g.interner) == "Doc"
                    && matches!(&*nd.id(), Value::String(s) if s == "docs/guide")
            })
            .expect("mdx doc node");
        // Frontmatter is parsed (title + a flattened custom key) and the
        // markdown heading outline is extracted from the JSX-bearing body.
        assert_eq!(
            node.title().into_owned(),
            Value::String("Retry Guide".into())
        );
        assert_eq!(
            node.get_field_ref("audience").map(|v| v.into_owned()),
            Some(Value::String("operators".into()))
        );
        assert_eq!(
            node.get_field_ref("headings").map(|v| v.into_owned()),
            Some(Value::String("[\"Usage\"]".into()))
        );

        // Mentions come from the prose inside the component, not from the JSX.
        let names = mention_target_names(&g, "MENTIONS");
        assert!(names.contains("parse_wkt"), "backtick mention in JSX prose");
        assert!(names.contains("KnowledgeGraph"));
        // `mailto` (from the ESM template literal) and the component/module
        // names resolve to nothing, so the ESM header adds no edges at all.
        assert_eq!(count_conn(&g, "DOCUMENTS"), 0, "no links in this doc");
    }

    #[test]
    fn mdx_is_a_first_class_doc_link_target() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn go() {}").unwrap();
        fs::write(root.join("docs/reference.mdx"), "# Reference\nFields.\n").unwrap();
        // A `.md` linking to an `.mdx` and an `.mdx` linking back: both sides
        // only resolve because the two flavours strip to the same id shape.
        fs::write(
            root.join("docs/overview.md"),
            "# Overview\nSee the [reference](./reference.mdx).\n",
        )
        .unwrap();
        fs::write(
            root.join("README.md"),
            "# Project\nStart at the [overview](docs/overview.md).\n",
        )
        .unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        assert_eq!(
            doc_ids(&g),
            BTreeSet::from([
                "README".to_string(),
                "docs/overview".to_string(),
                "docs/reference".to_string(),
            ])
        );
        // overview → reference (the `.mdx` target) and README → overview.
        assert_eq!(count_conn(&g, "DOCUMENTS"), 2);
    }

    #[test]
    fn txt_files_are_never_ingested_as_docs() {
        // The pin on the `.txt` decision (see `discover_docs`). This file is
        // markdown-shaped in every respect — heading, backtick symbol mention,
        // markdown link — so if `.txt` were ever admitted it would produce a
        // Doc node, a MENTIONS edge and a DOCUMENTS edge, and all three
        // assertions below would fail at once.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn drainTelemetry() {}").unwrap();
        fs::write(
            root.join("NOTES.txt"),
            "# Operator notes\nCall `drainTelemetry`. See [readme](README.md).\n",
        )
        .unwrap();
        // The indiscriminate half of the argument: these are `.txt` too.
        fs::write(root.join("requirements.txt"), "requests==2.31.0\n").unwrap();
        fs::write(root.join("LICENSE.txt"), "MIT\n").unwrap();
        fs::write(root.join("README.md"), "# Project\nReal docs.\n").unwrap();

        let g = run_with_options(&root, false, true, None, None, true).unwrap();
        assert_eq!(
            doc_ids(&g),
            BTreeSet::from(["README".to_string()]),
            "only the .md is a doc; no .txt may appear"
        );
        assert_eq!(
            count_conn(&g, "MENTIONS"),
            0,
            ".txt contributes no mentions"
        );
        assert_eq!(count_conn(&g, "DOCUMENTS"), 0, ".txt contributes no links");
    }
}
