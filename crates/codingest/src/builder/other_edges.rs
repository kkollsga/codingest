//! USES_TYPE, CONTAINS, IMPORTS, FFI_EXPOSES edges.

use super::js_workspace::JsWorkspace;
use crate::models::{ClassInfo, ConstantInfo, EnumInfo, FileInfo, FunctionInfo, InterfaceInfo};
use crate::parsers::registry;
use aho_corasick::{AhoCorasick, MatchKind};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

fn get_separator(language: &str) -> &'static str {
    registry::edge_sep(language)
}

pub struct ContainsEdge {
    pub parent: String,
    pub child: String,
}

pub struct ImportEdge {
    pub file_path: String,
    pub module: String,
}

/// `File -[IMPORTS]-> File` — direct file-to-file dependency.
///
/// Sibling to `ImportEdge` (File → Module). Each source file's import strings
/// are resolved against the project's `module_path → file_path` reverse index
/// using the same longest-prefix walk as `build_import_edges`. Multiple
/// imports from the same source file resolving to the same target file are
/// aggregated into a single edge with `import_count` ≥ 1.
///
/// Powers transitive file-level impact analysis: "given changed files, which
/// other files are affected?" is one Cypher hop (e.g.
/// `MATCH (f:File)-[:IMPORTS]->(t:File {is_test: true}) ...`).
pub struct FileImportEdge {
    pub source: String,
    pub target: String,
    pub import_count: i64,
}

pub struct UsesTypeEdge {
    pub function: String,
    pub type_name: String,
    /// Target node type: "Struct" | "Class" | "Enum" | "Trait" | "Protocol" | "Interface"
    pub target_node_type: &'static str,
    /// Where in the function signature this type appears. Aggregates across
    /// all sites in the same function — a type used as both a parameter and
    /// a return value yields `"both"`. Values: `"parameter"` | `"return"` |
    /// `"both"` | `"signature"`. `"signature"` is the fallback when the
    /// parser couldn't extract structured parameters (typically the AC
    /// scanner found the type embedded in the signature string).
    ///
    /// Cypher: `WHERE r.position IN ['parameter','both']` for "consumes T",
    /// `WHERE r.position IN ['return','both']` for "produces T".
    pub position: &'static str,
}

pub struct FfiExposesEdge {
    pub module_fn: String,
    pub target_qname: String,
    pub target_type: &'static str,
    pub py_name: String,
}

/// `Module -[CONTAINS]-> File` edges — one per file pointing to its leaf module.
///
/// `build_modules` synthesizes a Module node for every prefix of a file's
/// `module_path`; the leaf module's qualified_name equals `file.module_path`
/// exactly. Without this edge, "what's in module X" requires a string
/// `STARTS WITH` filter; with it, the natural top-down walk works:
///
/// ```cypher
/// MATCH (m:Module {qualified_name: 'crate::graph::cypher'})
///       -[:HAS_SUBMODULE*0..]->(:Module)-[:CONTAINS]->(f:File)-[:DEFINES]->(fn:Function)
/// RETURN fn.qualified_name
/// ```
pub struct ModuleContainsFileEdge {
    pub module: String,
    pub file_path: String,
}

pub fn build_module_contains_file_edges(files: &[FileInfo]) -> Vec<ModuleContainsFileEdge> {
    files
        .iter()
        .filter(|f| !f.module_path.is_empty())
        .map(|f| ModuleContainsFileEdge {
            module: f.module_path.clone(),
            file_path: f.path.clone(),
        })
        .collect()
}

/// Module CONTAINS Module edges from file submodule declarations.
pub fn build_contains_edges(files: &[FileInfo]) -> Vec<ContainsEdge> {
    let mut out = Vec::new();
    let mut implicit_seen = HashSet::new();
    for f in files {
        if registry::has_implicit_module_hierarchy(&f.language) {
            let sep = registry::module_sep(&f.language);
            let parts: Vec<_> = f.module_path.split(sep).collect();
            for end in 2..=parts.len() {
                let parent = parts[..end - 1].join(sep);
                let child = parts[..end].join(sep);
                if implicit_seen.insert((parent.clone(), child.clone())) {
                    out.push(ContainsEdge { parent, child });
                }
            }
        }

        let sep = get_separator(&f.language);
        for sub in &f.submodule_declarations {
            out.push(ContainsEdge {
                parent: f.module_path.clone(),
                child: format!("{}{}{}", f.module_path, sep, sub),
            });
        }
    }
    out
}

fn normalize_import_path(base: &str, raw: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn path_import_candidates(file: &FileInfo, raw: &str) -> Vec<String> {
    if !registry::uses_path_imports(&file.language) {
        return Vec::new();
    }

    let trimmed = raw.trim();
    // A leading `<` is the C/C++ parser's system-include marker (`<vector>`
    // kept verbatim): the specifier names a toolchain header searched only on
    // the compiler's system path, so resolving it against project files would
    // manufacture an edge whenever a project path happens to share the name
    // (`<local.h>` vs a project-root `local.h`). Same no-edge outcome as any
    // other unresolvable import.
    if trimmed.is_empty()
        || trimmed.starts_with('<')
        || trimmed.starts_with('#')
        || trimmed.starts_with("//")
        || trimmed.contains("://")
        || trimmed.to_ascii_lowercase().starts_with("data:")
    {
        return Vec::new();
    }

    let without_suffix = trimmed
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .replace('\\', "/");
    if without_suffix.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let root_path = without_suffix.trim_start_matches('/');
    if !without_suffix.starts_with('/') {
        let parent = file.path.rsplit_once('/').map_or("", |(parent, _)| parent);
        if let Some(candidate) = normalize_import_path(parent, root_path) {
            candidates.push(candidate);
        }
    }
    if let Some(candidate) = normalize_import_path("", root_path) {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }

    if file.language == "css" {
        let extensionless: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .rsplit_once('/')
                    .map_or(candidate.as_str(), |(_, name)| name)
                    .rsplit_once('.')
                    .is_none()
            })
            .map(|candidate| format!("{candidate}.css"))
            .collect();
        candidates.extend(extensionless);
    }
    candidates
}

/// Module-path suffixes a TS/JS specifier may carry. Stripped before matching,
/// mirroring `JstsParser::file_to_module_path`. `.js`/`.mjs`/`.cjs` are in the
/// list because TS's NodeNext resolution has source files import each other by
/// their *emitted* name (`import "./util.js"` resolving to `util.ts`).
const MODULE_PATH_EXTENSIONS: &[&str] =
    &[".tsx", ".ts", ".jsx", ".mjs", ".cjs", ".mts", ".cts", ".js"];

/// Candidate module paths for one TS/JS import specifier, in fixed priority
/// order — first match wins, and the order IS the documented tie-break.
///
/// Relative specifiers only in this pass: `./x` and `../x` are normalized
/// against the importing file's directory, extension-stripped, and offered as
/// (1) the normalized path and (2) the same path with a trailing `/index`
/// segment removed, because `file_to_module_path` collapses `a/b/index.ts` to
/// module `a/b`. Bare and aliased specifiers fall through to the caller's
/// existing longest-prefix walk.
///
/// No filesystem probing and no extension guessing: the candidates are pure
/// string derivations, checked against the project's real module set by the
/// caller. That is what makes it impossible to manufacture an edge to a target
/// that does not exist.
fn module_path_import_candidates(
    file: &FileInfo,
    raw: &str,
    workspace: &JsWorkspace,
) -> Vec<String> {
    if !registry::uses_module_path_imports(&file.language) {
        return Vec::new();
    }
    let trimmed = raw.trim();
    let parent = file.path.rsplit_once('/').map_or("", |(parent, _)| parent);

    // Raw (still extension-carrying) paths, in priority order.
    let raw_paths: Vec<String> = if trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed == "."
        || trimmed == ".."
    {
        normalize_import_path(parent, trimmed).into_iter().collect()
    } else {
        // Alias before workspace package: a tsconfig `paths` entry is an
        // explicit, file-scoped instruction, while package-name matching is a
        // convention-based probe.
        let mut paths = workspace.alias_targets(parent, trimmed);
        paths.extend(workspace.package_targets(trimmed));
        paths
    };

    let mut candidates: Vec<String> = Vec::new();
    let mut push = |candidate: String| {
        if !candidate.is_empty() && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };
    for path in raw_paths {
        let stripped = MODULE_PATH_EXTENSIONS
            .iter()
            .find_map(|ext| path.strip_suffix(ext))
            .unwrap_or(path.as_str())
            .to_string();
        // A bare root-level `index` collapses to nothing resolvable, so the
        // emptiness guard in `push` is load-bearing, not defensive.
        let without_index = stripped.strip_suffix("/index").map(str::to_string);
        push(stripped);
        if let Some(without_index) = without_index {
            push(without_index);
        }
    }
    candidates
}

/// The leading segments a parser's `file_to_module_path` prepended to this
/// file's module path but which import specifiers never carry.
///
/// Python's `file_to_module_path` prefixes every module path with the source
/// root's own directory name, so a checkout of `pkg/app.py` under `py_basic/`
/// gets module path `py_basic.pkg.app` while its own `from pkg.util import
/// helper` names `pkg.util`. The two can never meet in a raw prefix walk. The
/// prefix is recovered rather than assumed: render the file path the way the
/// parser does (drop the extension, drop a trailing `__init__`), join it with
/// the module separator, and subtract that tail from the module path. What is
/// left is exactly the segments the parser added.
///
/// Returns `None` when nothing was added — a clone layout
/// (`xarray/core/dataset.py` → `xarray.core.dataset`) subtracts to nothing, so
/// that layout provably generates no extra candidates at all.
///
/// The second return value is the file's own directory rendered in module form
/// (`src.mypkg` for `src/mypkg/app.py`); see [`module_path_prefix_candidates`],
/// which walks its prefixes as candidate import roots.
fn module_path_root_prefix(file: &FileInfo, sep: &str) -> Option<(String, Vec<String>)> {
    let mut segments: Vec<String> = file
        .path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if let Some(last) = segments.last_mut() {
        if let Some((stem, _ext)) = last.rsplit_once('.') {
            if !stem.is_empty() {
                *last = stem.to_string();
            }
        }
    }
    if segments.last().map(String::as_str) == Some("__init__") {
        segments.pop();
    }
    let module = file.module_path.as_str();
    let tail = segments.join(sep);
    // Everything above the file's own leaf segment: the directories an import
    // could plausibly be resolved relative to.
    let dirs: Vec<String> = segments
        .split_last()
        .map_or_else(Vec::new, |(_, rest)| rest.to_vec());
    if tail.is_empty() {
        // A root-level `__init__.py`: the whole module path is prefix.
        return (!module.is_empty()).then(|| (module.to_string(), dirs));
    }
    if module == tail {
        return None;
    }
    module
        .strip_suffix(&format!("{sep}{tail}"))
        .filter(|prefix| !prefix.is_empty())
        .map(|prefix| (prefix.to_string(), dirs))
}

/// Extra candidates for an absolute import whose specifier is root-relative
/// while the project's module paths carry a parser-added root prefix.
///
/// Scoped to Python deliberately: it is the one language here whose module
/// paths are derived from the source root's directory name rather than from a
/// declaration in the file (`package` in Java/Kotlin) or from the path itself
/// (TS/JS). Widening it is a change of behaviour for another language, not a
/// generalisation, so the gate is an explicit language check rather than a
/// registry predicate — `registry::uses_module_path_imports` also gates
/// JsWorkspace discovery in `builder/mod.rs` and must stay TS/JS-only.
///
/// A specifier is resolved relative to some `sys.path` entry, which the graph
/// does not record. The plausible entries are the importing file's own ancestor
/// directories — the project root (the overwhelmingly common case), a `src/`
/// layout's `src` dir, and, exactly as `python pkg/app.py` puts `sys.path[0]`,
/// the script's own directory. So the roots tried are the recovered prefix
/// extended by each leading run of the importer's directory segments,
/// shallowest first: `proj`, `proj.src`, `proj.src.mypkg` for
/// `src/mypkg/app.py`. Shallowest-first makes the project root win ties, which
/// is what keeps the common layout's answer stable.
///
/// The specifier is walked longest-first across all roots before any shortening
/// (mirroring the caller's raw walk), and it never shortens below one segment.
/// Shortening further would offer a bare root, which every module in the
/// project is a descendant of — `import functools` would then resolve to the
/// project root module. That is the manufactured-edge failure this whole
/// resolver is built to avoid.
/// Candidates for a **Rust** `use` path, rewritten into the same coordinate
/// system `file_to_module_path` stamps on files — the conversion whose absence
/// meant a `crate::` path could never match a derived `crate::src::…` module
/// path (mcp-servers report 2026-08-14, finding 1).
///
/// The importing file's own `module_path` carries everything needed:
/// * its **crate-root prefix** — the derived path up to and including the last
///   `src` segment (`crate::src`, or `crate::crates::<pkg>::src` in a
///   workspace layout). `crate::X` rewrites to `{prefix}::X`, which is exactly
///   where the target file's derived path lives. Stored module ids stay
///   untouched (globally unique across workspace members); only matching
///   changes coordinates.
/// * its **module chain** for `super::`/`self::`: `self` is the file's own
///   module; each `super` pops one segment.
///
/// Emitted longest→shortest so the caller's `find_map` lands on the deepest
/// real file: `crate::alpha::AlphaThing` tries `…::alpha::AlphaThing` (an
/// item, usually no file) before `…::alpha` (the file). No candidate is ever
/// shorter than the crate-root prefix itself — trimming past it would resolve
/// every import to the crate-root module, which is precisely the old bug's
/// output shape (every Rust IMPORTS edge landing on one Module "crate").
fn rust_import_candidates(file: &FileInfo, raw: &str) -> Vec<String> {
    if file.language != "rust" {
        return Vec::new();
    }
    let own: Vec<&str> = file.module_path.split("::").collect();
    // Crate-root prefix: up to and including the LAST `src` segment; a layout
    // with no `src` dir (rare, but `build.rs` or a flat corpus root) uses the
    // leading `crate` alone.
    let root_len = own
        .iter()
        .rposition(|seg| *seg == "src")
        .map(|i| i + 1)
        .unwrap_or(1);

    let trimmed = raw.trim();
    let rebased: Option<Vec<String>> = if let Some(rest) = trimmed.strip_prefix("crate::") {
        Some(
            own[..root_len]
                .iter()
                .map(|s| s.to_string())
                .chain(rest.split("::").map(str::to_string))
                .collect(),
        )
    } else if trimmed == "crate" {
        Some(own[..root_len].iter().map(|s| s.to_string()).collect())
    } else if trimmed.starts_with("super::") || trimmed == "super" || trimmed.starts_with("self::")
    {
        // `self` = the file's own module; each `super` pops one segment.
        let mut base: Vec<String> = own.iter().map(|s| s.to_string()).collect();
        let mut rest = trimmed;
        while let Some(r) =
            rest.strip_prefix("super::")
                .or(if rest == "super" { Some("") } else { None })
        {
            if base.len() > root_len {
                base.pop();
            }
            rest = r;
        }
        if let Some(r) = rest.strip_prefix("self::") {
            rest = r;
        }
        if !rest.is_empty() {
            base.extend(rest.split("::").map(str::to_string));
        }
        Some(base)
    } else {
        // A bare path (`alpha::Item` via 2015-style or re-export) — try it
        // under the importer's crate root before the raw walk gives up.
        None
    };

    let mut out = Vec::new();
    if let Some(parts) = rebased {
        // Longest→shortest, never shorter than the crate-root prefix.
        for end in (root_len..=parts.len()).rev() {
            let candidate = parts[..end].join("::");
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

fn module_path_prefix_candidates(file: &FileInfo, raw: &str, sep: &str) -> Vec<String> {
    if file.language != "python" {
        return Vec::new();
    }
    let trimmed = raw.trim();
    // Relative imports (`from .util import x`) are dropped at parse time; the
    // guard keeps a leading dot from ever producing an empty first segment.
    if trimmed.is_empty() || trimmed.starts_with('.') {
        return Vec::new();
    }
    let Some((prefix, dirs)) = module_path_root_prefix(file, sep) else {
        return Vec::new();
    };
    let roots: Vec<String> = (0..=dirs.len())
        .map(|depth| {
            if depth == 0 {
                prefix.clone()
            } else {
                format!("{prefix}{sep}{}", dirs[..depth].join(sep))
            }
        })
        .collect();
    let parts: Vec<&str> = trimmed.split(sep).collect();
    let mut out = Vec::new();
    for end in (1..=parts.len()).rev() {
        let specifier = parts[..end].join(sep);
        for root in &roots {
            let candidate = format!("{root}{sep}{specifier}");
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

fn resolve_path_import(
    file: &FileInfo,
    raw: &str,
    file_to_module: &HashMap<&str, &str>,
) -> Option<(String, String)> {
    for candidate in path_import_candidates(file, raw) {
        if let Some(module) = file_to_module.get(candidate.as_str()) {
            return Some((candidate, (*module).to_string()));
        }
    }
    None
}

/// File IMPORTS Module edges — resolve each import string against known modules.
pub fn build_import_edges(
    files: &[FileInfo],
    known_modules: &HashSet<String>,
    workspace: &JsWorkspace,
) -> Vec<ImportEdge> {
    let mut out = Vec::new();
    let file_to_module: HashMap<&str, &str> = if files
        .iter()
        .any(|file| registry::uses_path_imports(&file.language))
    {
        files
            .iter()
            .map(|file| (file.path.as_str(), file.module_path.as_str()))
            .collect()
    } else {
        HashMap::new()
    };
    for f in files {
        let sep = get_separator(&f.language);
        for use_path in &f.imports {
            // C/C++ system include (`<...>` kept verbatim by the parser):
            // never resolvable to a project module or file, so it gets no
            // edge of either kind — not even via the raw prefix walk, whose
            // segment split could otherwise land `<sys/stat.h>` on a module
            // by accident. No other language emits a `<`-leading specifier.
            if use_path.starts_with('<') {
                continue;
            }
            if let Some((_, module)) = resolve_path_import(f, use_path, &file_to_module) {
                out.push(ImportEdge {
                    file_path: f.path.clone(),
                    module,
                });
                continue;
            }
            if let Some(module) = module_path_import_candidates(f, use_path, workspace)
                .into_iter()
                .find(|candidate| known_modules.contains(candidate))
            {
                out.push(ImportEdge {
                    file_path: f.path.clone(),
                    module,
                });
                continue;
            }
            let rust_candidates = rust_import_candidates(f, use_path);
            if !rust_candidates.is_empty() {
                // Same no-fallthrough rule as the file-edge pass: the raw walk
                // ends at the bare `crate` Module for every rewritable path.
                // And the same self-guard: `use self::helper` names the file's
                // own module — a dependency on yourself is not an edge.
                if let Some(module) = rust_candidates
                    .into_iter()
                    .find(|candidate| known_modules.contains(candidate))
                {
                    if module != f.module_path {
                        out.push(ImportEdge {
                            file_path: f.path.clone(),
                            module,
                        });
                    }
                }
                continue;
            }
            let parts: Vec<&str> = use_path.split(sep).collect();
            let mut resolved = false;
            for end in (1..=parts.len()).rev() {
                let candidate = parts[..end].join(sep);
                if known_modules.contains(&candidate) {
                    out.push(ImportEdge {
                        file_path: f.path.clone(),
                        module: candidate,
                    });
                    resolved = true;
                    break;
                }
            }
            if resolved {
                continue;
            }
            // Last resort: the specifier may be root-relative while this
            // project's module paths carry a parser-added root prefix. Tried
            // only here, so a layout the raw walk already resolves keeps its
            // existing answer byte-for-byte.
            if let Some(module) = module_path_prefix_candidates(f, use_path, sep)
                .into_iter()
                .find(|candidate| known_modules.contains(candidate))
            {
                out.push(ImportEdge {
                    file_path: f.path.clone(),
                    module,
                });
            }
        }
    }
    out
}

/// `File -[IMPORTS]-> File` edges — resolve each import string to a project
/// file via the `module_path → file_path` reverse index.
///
/// Walks the import path from longest to shortest prefix (mirroring
/// `build_import_edges`'s module resolution) and lands on the first file
/// whose `module_path` matches a prefix candidate. Self-imports are skipped.
/// Multiple imports from the same source resolving to the same target are
/// aggregated into a single edge whose `import_count` records the multiplicity.
pub fn build_file_import_edges(
    files: &[FileInfo],
    module_to_file: &HashMap<String, String>,
    workspace: &JsWorkspace,
) -> Vec<FileImportEdge> {
    // The rows are fed directly to `add_connections`, so their order becomes
    // part of the persisted graph topology. Keep aggregation key-sorted to
    // make independently-built `.kgl` files byte-identical.
    let mut counts: BTreeMap<(String, String), i64> = BTreeMap::new();
    let file_to_module: HashMap<&str, &str> = if files
        .iter()
        .any(|file| registry::uses_path_imports(&file.language))
    {
        files
            .iter()
            .map(|file| (file.path.as_str(), file.module_path.as_str()))
            .collect()
    } else {
        HashMap::new()
    };
    for f in files {
        let sep = get_separator(&f.language);
        for use_path in &f.imports {
            // Mirror of `build_import_edges`: a `<system>` include is
            // unresolvable by definition and forms no File→File edge.
            if use_path.starts_with('<') {
                continue;
            }
            if let Some((target_file, _)) = resolve_path_import(f, use_path, &file_to_module) {
                if target_file != f.path {
                    *counts.entry((f.path.clone(), target_file)).or_insert(0) += 1;
                }
                continue;
            }
            if let Some(target_file) = module_path_import_candidates(f, use_path, workspace)
                .into_iter()
                .find_map(|candidate| module_to_file.get(&candidate))
            {
                if target_file != &f.path {
                    *counts
                        .entry((f.path.clone(), target_file.clone()))
                        .or_insert(0) += 1;
                }
                continue;
            }
            let rust_candidates = rust_import_candidates(f, use_path);
            if !rust_candidates.is_empty() {
                // A rewritten Rust path either resolves here or not at all —
                // falling through to the raw prefix walk would re-land on the
                // crate-root Module match the rewrite exists to prevent.
                if let Some(target_file) = rust_candidates
                    .into_iter()
                    .find_map(|candidate| module_to_file.get(&candidate))
                {
                    if target_file != &f.path {
                        *counts
                            .entry((f.path.clone(), target_file.clone()))
                            .or_insert(0) += 1;
                    }
                }
                continue;
            }
            let parts: Vec<&str> = use_path.split(sep).collect();
            let mut resolved = false;
            for end in (1..=parts.len()).rev() {
                let candidate = parts[..end].join(sep);
                if let Some(target_file) = module_to_file.get(&candidate) {
                    if target_file != &f.path {
                        *counts
                            .entry((f.path.clone(), target_file.clone()))
                            .or_insert(0) += 1;
                    }
                    resolved = true;
                    break;
                }
            }
            if resolved {
                continue;
            }
            // Mirror of `build_import_edges`: root-prefixed candidates for a
            // root-relative specifier, tried only after the raw walk misses.
            // The self-import guard applies here exactly as above.
            if let Some(target_file) = module_path_prefix_candidates(f, use_path, sep)
                .into_iter()
                .find_map(|candidate| module_to_file.get(&candidate))
            {
                if target_file != &f.path {
                    *counts
                        .entry((f.path.clone(), target_file.clone()))
                        .or_insert(0) += 1;
                }
            }
        }
    }
    counts
        .into_iter()
        .map(|((source, target), import_count)| FileImportEdge {
            source,
            target,
            import_count,
        })
        .collect()
}

/// USES_TYPE edges: scan function signature/return_type for known type names.
///
/// Returns a map from target node type → list of edges, because add_connections
/// must be called separately for each distinct target type.
pub fn build_uses_type_edges(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    enums: &[EnumInfo],
    interfaces: &[InterfaceInfo],
) -> BTreeMap<&'static str, Vec<UsesTypeEdge>> {
    // Collect known type names → (qualified_name, node_type).
    let mut type_lookup: HashMap<String, (String, &'static str)> = HashMap::new();
    for c in classes {
        if c.name.chars().count() > 1 {
            let target = super::class_node_type(&c.kind);
            type_lookup.insert(c.name.clone(), (c.qualified_name.clone(), target));
        }
    }
    for e in enums {
        if e.name.chars().count() > 1 {
            type_lookup.insert(e.name.clone(), (e.qualified_name.clone(), "Enum"));
        }
    }
    for i in interfaces {
        if i.name.chars().count() > 1 {
            let target = match i.kind.as_str() {
                "trait" => "Trait",
                "protocol" => "Protocol",
                _ => "Interface",
            };
            type_lookup.insert(i.name.clone(), (i.qualified_name.clone(), target));
        }
    }

    if type_lookup.is_empty() {
        return BTreeMap::new();
    }

    // Flatten type names into a stable-ordered Vec so pattern IDs from
    // Aho-Corasick map back to the right (qname, node_type) tuple.
    // Longest-match-first so "MyCollection" wins over "Collection".
    let mut names: Vec<String> = type_lookup.keys().cloned().collect();
    names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    let pattern_meta: Vec<(String, &'static str)> = names
        .iter()
        .map(|n| {
            let (q, t) = type_lookup.get(n).unwrap();
            (q.clone(), *t)
        })
        .collect();

    let ac = match AhoCorasick::builder()
        .match_kind(MatchKind::LeftmostLongest)
        .build(&names)
    {
        Ok(ac) => ac,
        Err(_) => return BTreeMap::new(),
    };

    // Per-function scan in parallel. Scans signature/return_type/each parameter
    // separately, tracks the *set* of positions per pattern, then collapses to
    // a single position per (function, type) so we emit at most one USES_TYPE
    // edge per node pair. (The graph engine keys edges by (src, type, tgt) —
    // multiple edges with same nodes would overwrite.)
    //
    // Position bitset: bit 0 = parameter, bit 1 = return, bit 2 = signature,
    // bit 3 = receiver. Receiver is treated as an input position distinct
    // from `parameter` because `func (c *Call) lock()`-style methods consume
    // their receiver type implicitly — users querying "who consumes T" want
    // both parameter and receiver matches, but they're semantically different.
    const POS_PARAM: u8 = 1 << 0;
    const POS_RETURN: u8 = 1 << 1;
    const POS_SIGNATURE: u8 = 1 << 2;
    const POS_RECEIVER: u8 = 1 << 3;

    let per_fn: Vec<Vec<(u32, &'static str, String, &'static str)>> = functions
        .par_iter()
        .map(|fn_info| {
            // pat_id → bitset of positions seen in this function.
            let mut seen: HashMap<u32, u8> = HashMap::new();

            let scan = |text: &str, pos_bit: u8, seen: &mut HashMap<u32, u8>| {
                if text.is_empty() {
                    return;
                }
                let bytes = text.as_bytes();
                for m in ac.find_iter(text) {
                    let start = m.start();
                    let end = m.end();
                    let before_ok = start == 0
                        || !bytes[start - 1].is_ascii_alphanumeric() && bytes[start - 1] != b'_';
                    let after_ok = end == text.len()
                        || !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_';
                    if !before_ok || !after_ok {
                        continue;
                    }
                    let pat_id = m.pattern().as_usize() as u32;
                    *seen.entry(pat_id).or_insert(0) |= pos_bit;
                }
            };

            // 1. Each structured parameter type — clean per-position attribution.
            //    Receivers (Go `(c *Call)`, Rust `&self`) get POS_RECEIVER instead
            //    of POS_PARAM so the resulting edge is labeled `position="receiver"`.
            for p in &fn_info.parameters {
                if let Some(t) = &p.type_annotation {
                    let pos_bit = if p.kind == crate::models::ParameterKind::Receiver {
                        POS_RECEIVER
                    } else {
                        POS_PARAM
                    };
                    scan(t, pos_bit, &mut seen);
                }
            }
            // 2. Return type.
            if let Some(rt) = &fn_info.return_type {
                scan(rt, POS_RETURN, &mut seen);
            }
            // 3. Signature fallback — only when structured parameters are
            // empty (parser couldn't extract them). Without this, legacy
            // parses lose USES_TYPE coverage entirely. Tagged "signature"
            // so callers know it's a less-precise attribution.
            let has_param_types = fn_info
                .parameters
                .iter()
                .any(|p| p.type_annotation.is_some());
            if !has_param_types && !fn_info.signature.is_empty() {
                scan(&fn_info.signature, POS_SIGNATURE, &mut seen);
            }

            // Collapse bitset to a single label. {param, return, receiver} are
            // semantic positions; signature is the fallback. If two or more
            // semantic positions fire (e.g. receiver + return on a chaining
            // method), collapse to "both". Pure receiver-only stays "receiver".
            let mut matches: Vec<_> = seen
                .into_iter()
                .map(|(pat_id, bits)| {
                    let semantic_count = (bits & POS_PARAM != 0) as u8
                        + (bits & POS_RETURN != 0) as u8
                        + (bits & POS_RECEIVER != 0) as u8;
                    let position = if semantic_count >= 2 {
                        "both"
                    } else if bits & POS_RECEIVER != 0 {
                        "receiver"
                    } else if bits & POS_PARAM != 0 {
                        "parameter"
                    } else if bits & POS_RETURN != 0 {
                        "return"
                    } else if bits & POS_SIGNATURE != 0 {
                        "signature"
                    } else {
                        unreachable!("at least one position bit must be set");
                    };
                    let (qname, target) = &pattern_meta[pat_id as usize];
                    (pat_id, *target, qname.clone(), position)
                })
                .collect();
            // `seen` is a HashMap. Stable pattern order is required because
            // these rows are inserted into the graph in this sequence.
            matches.sort_unstable_by_key(|(pat_id, _, _, _)| *pat_id);
            matches
        })
        .collect();

    let mut by_target_type: BTreeMap<&'static str, Vec<UsesTypeEdge>> = BTreeMap::new();
    for (fn_info, matches) in functions.iter().zip(per_fn) {
        for (_pat_id, target, qname, position) in matches {
            by_target_type
                .entry(target)
                .or_default()
                .push(UsesTypeEdge {
                    function: fn_info.qualified_name.clone(),
                    type_name: qname,
                    target_node_type: target,
                    position,
                });
        }
    }

    by_target_type
}

pub struct ReferencesEdge {
    pub function: String,
    pub constant: String,
    /// Line number in the function body where the reference appears.
    pub line: u32,
    /// Optional aggregate site detail from typed parser references. Ordinary
    /// language references retain the legacy scalar `line` only.
    pub reference_lines: Option<String>,
    pub reference_count: Option<i64>,
    pub opcodes: Option<String>,
    pub accesses: Option<String>,
    pub has_read: Option<bool>,
    pub has_write: Option<bool>,
    pub has_address: Option<bool>,
}

pub struct ReferencesFnEdge {
    pub caller: String,
    pub callee: String,
    pub line: u32,
}

/// `Function -[DECORATES]-> Function` — resolved decorator-to-decoratee edges.
///
/// Per-language parsers already populate `FunctionInfo.decorators` with the
/// raw decorator strings (`"property"`, `"functools.wraps"`, `"app.route('/x')"`).
/// This pass strips any call-args, extracts the terminal segment as a bare
/// name, and resolves it against the project's Function set the same way
/// `build_call_edges` does for CALLS.
///
/// Direction: `decorator -[DECORATES]-> function` reads naturally as
/// "this decorator decorates that function". Third-party decorators
/// (`@pytest.fixture`, `@app.route` from a Flask app) that don't have a
/// matching Function node are silently dropped — the absence of an edge
/// is correct (we can't resolve into code we don't parse) and mirrors
/// `build_call_edges`'s same-file/global-fallback handling.
pub struct DecoratesEdge {
    pub decorator: String,
    pub function: String,
    /// Raw decorator string from source (e.g. `"functools.wraps"` or
    /// `"app.route('/users/{id}')"`). Preserved on the edge so callers
    /// who want the original literal don't have to reparse the
    /// Function.decorators property.
    pub decorator_name: String,
}

/// REFERENCES edges from `Function` to `Constant` — emit one row per
/// `(function, constant)` pair where the constant's terminal name
/// appears in the function body's identifier stream.
///
/// Per-language parsers populate `FunctionInfo.references` with
/// constant-style identifier candidates (the Rust parser uses
/// `SCREAMING_SNAKE_CASE` as the heuristic). This pass resolves each
/// candidate against the project's constant set and dedupes per
/// `(function, constant)` pair so a constant referenced N times in
/// one function still produces a single edge.
pub fn build_references_edges(
    functions: &[FunctionInfo],
    constants: &[ConstantInfo],
) -> Vec<ReferencesEdge> {
    if constants.is_empty() {
        return Vec::new();
    }

    // Name-keyed lookup: constant short-name → qualified_name. When two
    // constants share the same name (cross-module), we keep both —
    // emit edges to all matches. This mirrors how the type-name resolver
    // handles ambiguity (it doesn't disambiguate by import scope yet).
    let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
    for c in constants {
        by_name
            .entry(c.name.as_str())
            .or_default()
            .push(c.qualified_name.as_str());
    }

    let mut out: Vec<ReferencesEdge> = Vec::new();
    for f in functions {
        if f.references.is_empty() {
            continue;
        }
        // Dedup per (function, constant_qname) — a function that uses
        // the same constant on three lines emits one edge.
        let mut seen: HashSet<&str> = HashSet::new();
        for (ident, line) in &f.references {
            let Some(matches) = by_name.get(ident.as_str()) else {
                continue;
            };
            for &qname in matches {
                if seen.insert(qname) {
                    out.push(ReferencesEdge {
                        function: f.qualified_name.clone(),
                        constant: qname.to_string(),
                        line: *line,
                        reference_lines: None,
                        reference_count: None,
                        opcodes: None,
                        accesses: None,
                        has_read: None,
                        has_write: None,
                        has_address: None,
                    });
                }
            }
        }
    }
    out
}

/// `Function -[REFERENCES_FN]-> Function` — bare/scoped identifiers
/// passed as arguments to higher-order calls (`iter.and_then(some_fn)`).
/// The referenced function isn't invoked at the reference site, so this
/// is intentionally a different edge type from `CALLS`. Dead-code
/// analysis can union the two: a function with `CALLS ∪ REFERENCES_FN`
/// = 0 inbound is genuinely uncalled.
///
/// Resolution mirrors `build_call_edges`'s name-keyed lookup: only
/// emit an edge when the identifier matches exactly one function in
/// the project (skip ambiguous matches to avoid noise from
/// argument-name collisions with unrelated functions) — and, per D3,
/// a closure-scoped definition is offered only to referrers in its own
/// file. `wrapper`, `inner` and `decorator` are the most common nested
/// names there are; without the gate a globally unique one of them
/// becomes a cross-file REFERENCES_FN target, which is the same
/// false-positive class D3 keeps out of CALLS.
pub fn build_references_fn_edges(functions: &[FunctionInfo]) -> Vec<ReferencesFnEdge> {
    if functions.is_empty() {
        return Vec::new();
    }
    let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut nested_by_file: HashMap<&str, HashMap<&str, Vec<&str>>> = HashMap::new();
    for f in functions {
        let bucket = if super::call_edges::is_nested_function(f) {
            nested_by_file
                .entry(f.file_path.as_str())
                .or_default()
                .entry(f.name.as_str())
                .or_default()
        } else {
            by_name.entry(f.name.as_str()).or_default()
        };
        bucket.push(f.qualified_name.as_str());
    }

    let mut out: Vec<ReferencesFnEdge> = Vec::new();
    for f in functions {
        if f.function_refs.is_empty() {
            continue;
        }
        let caller = f.qualified_name.as_str();
        let local = nested_by_file.get(f.file_path.as_str());
        let mut seen: HashSet<&str> = HashSet::new();
        for (ident, line) in &f.function_refs {
            let mut storage: Vec<&str> = Vec::new();
            let matches = super::call_edges::merge_candidates(
                by_name.get(ident.as_str()),
                local.and_then(|m| m.get(ident.as_str())),
                &mut storage,
            );
            // Only emit on unambiguous matches — if the bare name maps
            // to multiple functions, skip rather than guess. Function
            // pointers passed as arguments don't carry receiver-type
            // info that the call-edge resolver could use to narrow.
            if matches.len() != 1 {
                continue;
            }
            let target = matches[0];
            if target == caller {
                continue;
            }
            if seen.insert(target) {
                out.push(ReferencesFnEdge {
                    caller: caller.to_string(),
                    callee: target.to_string(),
                    line: *line,
                });
            }
        }
    }
    out
}

/// `Function -[BINDS]-> Function` — Python wrapper to its underlying Rust impl.
///
/// PyO3 exposes a `#[pyclass]` Rust struct (e.g. `KnowledgeGraph`) and its
/// `#[pymethods]` block to Python. The Python class shows up in a `.pyi`
/// stub like `kglite.KnowledgeGraph.add_nodes`, while the Rust side has
/// `crate::graph::pyapi::*::KnowledgeGraph::add_nodes` (with
/// `metadata.is_pymethod == true`). Method names are 1:1 by PyO3 contract.
///
/// Closes the cross-language graph: `MATCH (py)-[:BINDS]->(rs)-[:CALLS*]->(impl)`
/// traces a request from the Python entry point down to deep Rust impl.
///
/// Resolution rules:
/// - Python side: Function whose `qualified_name` matches `<pkg>.<Class>.<method>`
///   *and* whose `is_method == true`.
/// - Rust side: Function whose name == `<method>`, owner == `<Class>`, and
///   `metadata["is_pymethod"] == true`.
/// - Skip ambiguous Rust matches (multiple pymethods with same `(Class, method)`)
///   to avoid guessing — wouldn't compile under PyO3 anyway, but be defensive.
pub struct PyO3BindsEdge {
    pub py_function: String,
    pub rust_function: String,
}

pub fn build_pyo3_binds_edges(functions: &[FunctionInfo]) -> Vec<PyO3BindsEdge> {
    // Index Rust pymethods by (parent_struct_short_name, method_short_name).
    let mut rust_idx: HashMap<(String, String), Vec<&str>> = HashMap::new();
    for f in functions {
        if !f
            .metadata
            .get("is_pymethod")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        // Derive parent struct short name from the qualified name.
        // Rust pymethod qnames look like `crate::a::b::KnowledgeGraph::add_nodes`.
        let parts: Vec<&str> = f.qualified_name.split("::").collect();
        if parts.len() < 2 {
            continue;
        }
        let parent = parts[parts.len() - 2].to_string();
        let method = parts[parts.len() - 1].to_string();
        rust_idx
            .entry((parent, method))
            .or_default()
            .push(f.qualified_name.as_str());
    }

    let mut out = Vec::new();
    for f in functions {
        // Python class methods come from `.pyi` stubs and look like
        // `kglite.KnowledgeGraph.add_nodes`. The split separator is `.`.
        if !f.qualified_name.contains('.') || !f.is_method {
            continue;
        }
        let parts: Vec<&str> = f.qualified_name.split('.').collect();
        if parts.len() < 3 {
            continue;
        }
        let py_class = parts[parts.len() - 2].to_string();
        let py_method = parts[parts.len() - 1].to_string();
        let Some(matches) = rust_idx.get(&(py_class, py_method)) else {
            continue;
        };
        if matches.len() != 1 {
            continue; // ambiguous — skip
        }
        out.push(PyO3BindsEdge {
            py_function: f.qualified_name.clone(),
            rust_function: matches[0].to_string(),
        });
    }
    out
}

/// `Function -[DECORATES]-> Function` — resolve each parsed decorator
/// string to its target function. Strips call-args (`@app.route('/x')` →
/// `app.route`) and the namespace prefix (`functools.wraps` → `wraps`)
/// before consulting a bare-name index built from every project Function.
///
/// Unambiguous matches (exactly one qualified-name candidate) emit an
/// edge. Ambiguous bare names are skipped — duplicating the call-edge
/// resolver's stance: without import-scope info we'd guess, and a wrong
/// edge is worse than a missing one for downstream queries that count
/// `WHERE (dec)-[:DECORATES]->(fn) RETURN dec.name`. Self-decoration is
/// suppressed (would only happen on malformed input).
pub fn build_decorates_edges(functions: &[FunctionInfo]) -> Vec<DecoratesEdge> {
    if functions.is_empty() {
        return Vec::new();
    }
    // bare name → list of qualified_names that share that short name.
    // D3, as in `build_call_edges` and `build_references_fn_edges`: a
    // closure-scoped definition is a candidate decorator only for functions
    // in its own file. A decorator factory's `def wrapper` is the archetypal
    // nested name, and resolving `@wrapper` in an unrelated file to it would
    // be a pure false positive.
    let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut nested_by_file: HashMap<&str, HashMap<&str, Vec<&str>>> = HashMap::new();
    for f in functions {
        let bucket = if super::call_edges::is_nested_function(f) {
            nested_by_file
                .entry(f.file_path.as_str())
                .or_default()
                .entry(f.name.as_str())
                .or_default()
        } else {
            by_name.entry(f.name.as_str()).or_default()
        };
        bucket.push(f.qualified_name.as_str());
    }

    let mut out: Vec<DecoratesEdge> = Vec::new();
    for f in functions {
        if f.decorators.is_empty() {
            continue;
        }
        let function_qname = f.qualified_name.as_str();
        let local = nested_by_file.get(f.file_path.as_str());
        // Dedup per (decorator_qname → function) — a function with two
        // decorators that happen to resolve to the same target only
        // emits one edge. Carries the *first* raw decorator_name we
        // saw so the property remains stable.
        let mut seen: HashSet<&str> = HashSet::new();
        for raw in &f.decorators {
            // Strip call args: `app.route('/x', methods=['GET'])` → `app.route`.
            let head = raw.split('(').next().unwrap_or(raw).trim();
            if head.is_empty() {
                continue;
            }
            // Take the terminal segment after the last `.` or `::` — that's
            // the bare function name we look up. `functools.wraps` → `wraps`.
            let bare = head
                .rsplit_once("::")
                .map(|(_, t)| t)
                .or_else(|| head.rsplit_once('.').map(|(_, t)| t))
                .unwrap_or(head);
            let mut storage: Vec<&str> = Vec::new();
            let candidates = super::call_edges::merge_candidates(
                by_name.get(bare),
                local.and_then(|m| m.get(bare)),
                &mut storage,
            );
            if candidates.len() != 1 {
                continue; // ambiguous bare name — skip rather than guess
            }
            let target = candidates[0];
            if target == function_qname {
                continue; // self-decoration — defensive
            }
            if seen.insert(target) {
                out.push(DecoratesEdge {
                    decorator: target.to_string(),
                    function: function_qname.to_string(),
                    decorator_name: raw.clone(),
                });
            }
        }
    }
    out
}

/// FFI EXPOSES edges — #[pymodule] fn → each #[pyclass]/#[pyfunction] item.
pub fn build_ffi_exposes_edges(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
) -> Vec<FfiExposesEdge> {
    let pymodule_fns: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|f| {
            f.metadata
                .get("is_pymodule")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .collect();
    if pymodule_fns.is_empty() {
        return Vec::new();
    }

    let pyclass_items: Vec<(&ClassInfo, String)> = classes
        .iter()
        .filter(|c| {
            c.metadata
                .get("is_pyclass")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .map(|c| {
            let py_name = c
                .metadata
                .get("py_name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| c.name.clone());
            (c, py_name)
        })
        .collect();

    let pyfunc_items: Vec<(&FunctionInfo, String)> = functions
        .iter()
        .filter(|f| {
            !f.is_method
                && !f
                    .metadata
                    .get("is_pymodule")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                && f.metadata.get("ffi_kind").and_then(|v| v.as_str()) == Some("pyo3")
        })
        .map(|f| {
            let py_name = f
                .metadata
                .get("py_name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| f.name.clone());
            (f, py_name)
        })
        .collect();

    let mut out = Vec::new();
    for mod_fn in &pymodule_fns {
        for (c, py_name) in &pyclass_items {
            out.push(FfiExposesEdge {
                module_fn: mod_fn.qualified_name.clone(),
                target_qname: c.qualified_name.clone(),
                target_type: "Struct",
                py_name: py_name.clone(),
            });
        }
        for (f, py_name) in &pyfunc_items {
            out.push(FfiExposesEdge {
                module_fn: mod_fn.qualified_name.clone(),
                target_qname: f.qualified_name.clone(),
                target_type: "Function",
                py_name: py_name.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod determinism_tests {
    use super::*;
    use crate::models::{ClassInfo, FileInfo, FunctionInfo, ParameterInfo};

    fn source_file(path: &str, module: &str, language: &str, imports: &[&str]) -> FileInfo {
        FileInfo {
            path: path.into(),
            module_path: module.into(),
            language: language.into(),
            imports: imports.iter().map(|value| (*value).to_string()).collect(),
            ..FileInfo::default()
        }
    }

    #[test]
    fn implicit_module_hierarchy_uses_parser_module_separator() {
        let files = vec![
            source_file("Foo/Bar.php", "Foo\\Bar", "php", &[]),
            source_file("Foo/Other.php", "Foo\\Bar", "php", &[]),
            source_file("src/net/client.c", "src/net/client", "c", &[]),
        ];
        let pairs: Vec<_> = build_contains_edges(&files)
            .into_iter()
            .map(|edge| (edge.parent, edge.child))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("Foo".into(), "Foo\\Bar".into()),
                ("src".into(), "src/net".into()),
                ("src/net".into(), "src/net/client".into()),
            ]
        );
    }

    #[test]
    fn path_imports_resolve_relative_root_and_extensionless_css() {
        let files = vec![
            source_file(
                "examples/main.c",
                "examples/main",
                "c",
                &["../include/header.h"],
            ),
            source_file("include/header.h", "include/header", "c", &[]),
            source_file(
                "index.html",
                "site.index",
                "html",
                &["styles/site.css?rev=1", "https://cdn.example/x.css"],
            ),
            source_file("styles/site.css", "site.site", "css", &["theme"]),
            source_file("styles/theme.css", "site.theme", "css", &[]),
        ];
        let known_modules: HashSet<_> = files.iter().map(|file| file.module_path.clone()).collect();
        let module_edges = build_import_edges(&files, &known_modules, &JsWorkspace::default());
        let module_pairs: Vec<_> = module_edges
            .iter()
            .map(|edge| (edge.file_path.as_str(), edge.module.as_str()))
            .collect();
        assert_eq!(
            module_pairs,
            vec![
                ("examples/main.c", "include/header"),
                ("index.html", "site.site"),
                ("styles/site.css", "site.theme"),
            ]
        );

        let module_to_file = files
            .iter()
            .map(|file| (file.module_path.clone(), file.path.clone()))
            .collect();
        let file_edges = build_file_import_edges(&files, &module_to_file, &JsWorkspace::default());
        let file_pairs: Vec<_> = file_edges
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect();
        assert_eq!(
            file_pairs,
            vec![
                ("examples/main.c", "include/header.h"),
                ("index.html", "styles/site.css"),
                ("styles/site.css", "styles/theme.css"),
            ]
        );
    }

    /// C/C++ `#include` resolution, asserted as exact edge sets. Mirrors the
    /// `cpp_include` corpus plus the two shapes it cannot pin without moving
    /// its golden: a root-level decoy proving the including file's own
    /// directory wins (the compiler's quoted-include order), and an angle
    /// include *colliding* with a real project file, which must still form no
    /// edge — the `<...>` marker, not luck, is what excludes it.
    #[test]
    fn cpp_quoted_includes_resolve_dir_first_and_angle_includes_never_do() {
        let files = vec![
            source_file(
                "main.cpp",
                "main",
                "cpp",
                // `<local.h>` collides with the project's own `local.h`;
                // `<vector>` collides with nothing. Neither may resolve.
                &["local.h", "util/helper.h", "<vector>", "<local.h>"],
            ),
            source_file("local.h", "local", "cpp", &[]),
            // Decoy: same basename as util/helper.h, at the root.
            source_file("helper.h", "helper", "cpp", &[]),
            source_file(
                "util/helper.cpp",
                "util/helper",
                "cpp",
                // Same-dir resolution from inside a subdirectory, and a
                // parent-relative path that must normalize the `..` away.
                &["helper.h", "../local.h"],
            ),
            source_file("util/helper.h", "util/helper", "cpp", &[]),
        ];

        let known_modules: HashSet<_> = files.iter().map(|file| file.module_path.clone()).collect();
        let module_edges = build_import_edges(&files, &known_modules, &JsWorkspace::default());
        let module_pairs: Vec<_> = module_edges
            .iter()
            .map(|edge| (edge.file_path.as_str(), edge.module.as_str()))
            .collect();
        assert_eq!(
            module_pairs,
            vec![
                ("main.cpp", "local"),
                ("main.cpp", "util/helper"),
                // Dir-first: `"helper.h"` from util/ lands on util/helper,
                // not the root decoy's `helper` module.
                ("util/helper.cpp", "util/helper"),
                ("util/helper.cpp", "local"),
            ]
        );

        let module_to_file = files
            .iter()
            .map(|file| (file.module_path.clone(), file.path.clone()))
            .collect();
        let file_edges = build_file_import_edges(&files, &module_to_file, &JsWorkspace::default());
        let file_pairs: Vec<_> = file_edges
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect();
        assert_eq!(
            file_pairs,
            vec![
                ("main.cpp", "local.h"),
                ("main.cpp", "util/helper.h"),
                ("util/helper.cpp", "local.h"),
                ("util/helper.cpp", "util/helper.h"),
            ]
        );
    }

    /// The TS/JS module-path branch, asserted as exact edge sets. Mirrors the
    /// `ts_monorepo` corpus in miniature so a break shows up here with a
    /// readable diff before it shows up as a moved golden digest.
    #[test]
    fn ts_relative_imports_resolve_against_the_module_set() {
        let files = vec![
            source_file(
                "packages/core/src/util.ts",
                "packages/core/src/util",
                "typescript",
                &[],
            ),
            source_file(
                "packages/core/src/nested/deep.ts",
                "packages/core/src/nested/deep",
                "typescript",
                &[],
            ),
            // A barrel: `index.ts` collapses to the directory's module path.
            source_file(
                "packages/core/src/index.ts",
                "packages/core/src",
                "typescript",
                &["./util", "./nested/deep"],
            ),
            source_file(
                "packages/core/src/consumer.ts",
                "packages/core/src/consumer",
                "typescript",
                // `./util.js` is TS NodeNext spelling for `util.ts`.
                &["./util.js"],
            ),
            // `/index`-strip case: the specifier names the barrel file, whose
            // module path has the `index` segment collapsed away.
            source_file(
                "packages/app/src/main.ts",
                "packages/app/src/main",
                "typescript",
                &["../../core/src/index"],
            ),
            // Directory specifier: matches the barrel's module path directly.
            source_file(
                "packages/app/src/widget.tsx",
                "packages/app/src/widget",
                "typescript",
                &["../../core/src"],
            ),
            // A specifier naming no real module must produce NO edge — the
            // resolver may not invent a target.
            source_file(
                "packages/app/src/dangling.ts",
                "packages/app/src/dangling",
                "typescript",
                &["./does-not-exist", "../../core/src/nope"],
            ),
            // Bare/scoped specifiers are not the relative branch's business.
            source_file(
                "packages/app/src/bare.ts",
                "packages/app/src/bare",
                "typescript",
                &["zod", "@scope/core"],
            ),
        ];
        let known_modules: HashSet<_> = files.iter().map(|file| file.module_path.clone()).collect();
        let module_pairs: Vec<_> =
            build_import_edges(&files, &known_modules, &JsWorkspace::default())
                .into_iter()
                .map(|edge| (edge.file_path, edge.module))
                .collect();
        assert_eq!(
            module_pairs,
            vec![
                (
                    "packages/core/src/index.ts".to_string(),
                    "packages/core/src/util".to_string()
                ),
                (
                    "packages/core/src/index.ts".to_string(),
                    "packages/core/src/nested/deep".to_string()
                ),
                (
                    "packages/core/src/consumer.ts".to_string(),
                    "packages/core/src/util".to_string()
                ),
                (
                    "packages/app/src/main.ts".to_string(),
                    "packages/core/src".to_string()
                ),
                (
                    "packages/app/src/widget.tsx".to_string(),
                    "packages/core/src".to_string()
                ),
            ]
        );

        let module_to_file: HashMap<String, String> = files
            .iter()
            .map(|file| (file.module_path.clone(), file.path.clone()))
            .collect();
        let file_pairs: Vec<_> =
            build_file_import_edges(&files, &module_to_file, &JsWorkspace::default())
                .into_iter()
                .map(|edge| (edge.source, edge.target))
                .collect();
        assert_eq!(
            file_pairs,
            vec![
                (
                    "packages/app/src/main.ts".to_string(),
                    "packages/core/src/index.ts".to_string()
                ),
                (
                    "packages/app/src/widget.tsx".to_string(),
                    "packages/core/src/index.ts".to_string()
                ),
                (
                    "packages/core/src/consumer.ts".to_string(),
                    "packages/core/src/util.ts".to_string()
                ),
                (
                    "packages/core/src/index.ts".to_string(),
                    "packages/core/src/nested/deep.ts".to_string()
                ),
                (
                    "packages/core/src/index.ts".to_string(),
                    "packages/core/src/util.ts".to_string()
                ),
            ]
        );
    }

    /// Alias + workspace-package resolution, asserted as exact edge sets.
    /// Mirrors the `ts_monorepo` corpus's config layer.
    #[test]
    fn ts_alias_and_workspace_specifiers_resolve() {
        use super::super::js_workspace::TsPathsConfig;

        let files = vec![
            source_file(
                "packages/core/src/util.ts",
                "packages/core/src/util",
                "typescript",
                &[],
            ),
            // The core package's barrel: module path is the directory.
            source_file(
                "packages/core/src/index.ts",
                "packages/core/src",
                "typescript",
                &[],
            ),
            source_file(
                "packages/app/src/main.ts",
                "packages/app/src/main",
                "typescript",
                &[],
            ),
            source_file(
                "packages/app/src/consumer.ts",
                "packages/app/src/consumer",
                "typescript",
                &[
                    // tsconfig alias, governed by packages/app's own config
                    "@/main",
                    // bare workspace package -> the package's barrel
                    "@scope/core",
                    // workspace package subpath -> <pkgdir>/src/<rest>
                    "@scope/core/util",
                    // a package that does not exist
                    "@scope/nope/thing",
                    // an alias that resolves to no real module
                    "@/ghost",
                ],
            ),
        ];
        let workspace = JsWorkspace::from_raw(
            &[(
                "packages/app",
                TsPathsConfig {
                    base: "packages/app".into(),
                    paths: [("@/*".to_string(), vec!["./src/*".to_string()])]
                        .into_iter()
                        .collect(),
                },
            )],
            &[("@scope/core", "packages/core")],
        );

        let known_modules: HashSet<_> = files
            .iter()
            .map(|file| file.module_path.clone())
            // The prefix modules `build_modules` synthesizes for every path
            // segment — `<pkgdir>` is one of them, and it is what a bare
            // package specifier lands on at File→Module granularity.
            .chain(["packages/core".to_string(), "packages/app".to_string()])
            .collect();
        let module_pairs: Vec<_> = build_import_edges(&files, &known_modules, &workspace)
            .into_iter()
            .map(|edge| (edge.file_path, edge.module))
            .collect();
        assert_eq!(
            module_pairs,
            vec![
                (
                    "packages/app/src/consumer.ts".to_string(),
                    "packages/app/src/main".to_string()
                ),
                (
                    "packages/app/src/consumer.ts".to_string(),
                    "packages/core".to_string()
                ),
                (
                    "packages/app/src/consumer.ts".to_string(),
                    "packages/core/src/util".to_string()
                ),
            ]
        );

        let module_to_file: HashMap<String, String> = files
            .iter()
            .map(|file| (file.module_path.clone(), file.path.clone()))
            .collect();
        let file_pairs: Vec<_> = build_file_import_edges(&files, &module_to_file, &workspace)
            .into_iter()
            .map(|edge| (edge.source, edge.target))
            .collect();
        assert_eq!(
            file_pairs,
            // File→File rows are (source, target)-sorted by construction, so
            // this order is the persisted topology, not encounter order.
            vec![
                (
                    "packages/app/src/consumer.ts".to_string(),
                    "packages/app/src/main.ts".to_string()
                ),
                (
                    "packages/app/src/consumer.ts".to_string(),
                    // `@scope/core` -> <pkgdir> is not a file's module, so the
                    // File→File probe falls through to the barrel.
                    "packages/core/src/index.ts".to_string()
                ),
                (
                    "packages/app/src/consumer.ts".to_string(),
                    "packages/core/src/util.ts".to_string()
                ),
            ]
        );
    }

    /// Without a workspace table the alias/package specifiers must resolve to
    /// nothing — proving the new edges come from the table, not from the bare
    /// prefix walk accidentally matching.
    #[test]
    fn alias_and_package_specifiers_need_the_workspace_table() {
        let files = vec![
            source_file(
                "packages/core/src/util.ts",
                "packages/core/src/util",
                "typescript",
                &[],
            ),
            source_file(
                "packages/app/src/consumer.ts",
                "packages/app/src/consumer",
                "typescript",
                &["@/main", "@scope/core/util"],
            ),
        ];
        let known_modules: HashSet<_> = files.iter().map(|f| f.module_path.clone()).collect();
        assert!(build_import_edges(&files, &known_modules, &JsWorkspace::default()).is_empty());
    }

    /// A plain absolute import (`from pkg.util import helper`) resolves even
    /// though the Python parser prefixes module paths with the source root's
    /// directory name while the specifier is root-relative. Asserted as exact
    /// edge sets on both edge builders.
    #[test]
    fn python_absolute_imports_resolve_across_the_root_prefix() {
        let files = vec![
            source_file("pkg/app.py", "root.pkg.app", "python", &["pkg.util"]),
            source_file("pkg/util.py", "root.pkg.util", "python", &[]),
        ];
        let known_modules: HashSet<_> = files.iter().map(|f| f.module_path.clone()).collect();
        let module_pairs: Vec<_> =
            build_import_edges(&files, &known_modules, &JsWorkspace::default())
                .into_iter()
                .map(|edge| (edge.file_path, edge.module))
                .collect();
        assert_eq!(
            module_pairs,
            vec![("pkg/app.py".to_string(), "root.pkg.util".to_string())]
        );

        let module_to_file: HashMap<String, String> = files
            .iter()
            .map(|f| (f.module_path.clone(), f.path.clone()))
            .collect();
        let file_pairs: Vec<_> =
            build_file_import_edges(&files, &module_to_file, &JsWorkspace::default())
                .into_iter()
                .map(|edge| (edge.source, edge.target, edge.import_count))
                .collect();
        assert_eq!(
            file_pairs,
            vec![("pkg/app.py".to_string(), "pkg/util.py".to_string(), 1)]
        );
    }

    /// The prefixed candidates resolve against the real module set or not at
    /// all: an in-project module that does not exist and a stdlib module both
    /// yield nothing. No target is manufactured, and the walk never shortens
    /// far enough to land on the project root module.
    #[test]
    fn python_absolute_imports_do_not_manufacture_targets() {
        let files = vec![
            source_file(
                "pkg/app.py",
                "root.pkg.app",
                "python",
                &["pkg.nonexistent", "functools"],
            ),
            source_file("pkg/util.py", "root.pkg.util", "python", &[]),
        ];
        let mut known_modules: HashSet<_> = files.iter().map(|f| f.module_path.clone()).collect();
        // The ancestor modules a real build materialises, including the root
        // prefix itself — the shape that makes an over-eager walk visible.
        known_modules.insert("root".into());
        known_modules.insert("root.pkg".into());

        let module_edges = build_import_edges(&files, &known_modules, &JsWorkspace::default());
        assert_eq!(
            module_edges
                .iter()
                .map(|e| e.module.as_str())
                .collect::<Vec<_>>(),
            // `pkg.nonexistent` shortens to the real package `root.pkg`, the
            // same answer the raw walk gives every other language; `functools`
            // resolves to nothing at all.
            vec!["root.pkg"]
        );

        let module_to_file: HashMap<String, String> = files
            .iter()
            .map(|f| (f.module_path.clone(), f.path.clone()))
            .collect();
        // No file owns `root.pkg`, so the file-level pass emits nothing.
        assert!(
            build_file_import_edges(&files, &module_to_file, &JsWorkspace::default()).is_empty()
        );
    }

    /// A manifest src-layout adds TWO junk segments (`myproj.src.mypkg.util`);
    /// subtracting the file's own rendering recovers both.
    #[test]
    fn python_src_layout_imports_resolve() {
        let files = vec![
            source_file(
                "src/mypkg/app.py",
                "proj.src.mypkg.app",
                "python",
                &["mypkg.util"],
            ),
            source_file("src/mypkg/util.py", "proj.src.mypkg.util", "python", &[]),
        ];
        let known_modules: HashSet<_> = files.iter().map(|f| f.module_path.clone()).collect();
        let module_pairs: Vec<_> =
            build_import_edges(&files, &known_modules, &JsWorkspace::default())
                .into_iter()
                .map(|edge| edge.module)
                .collect();
        assert_eq!(module_pairs, vec!["proj.src.mypkg.util".to_string()]);
    }

    /// The clone layout the parser already handles (`xarray/core/dataset.py` →
    /// `xarray.core.dataset`) has no root prefix to subtract, so it resolves on
    /// the raw walk alone and gains exactly nothing — no second, doubled edge.
    #[test]
    fn python_clone_layout_resolves_exactly_once() {
        let files = vec![
            source_file(
                "xarray/core/dataset.py",
                "xarray.core.dataset",
                "python",
                &["xarray.core"],
            ),
            source_file("xarray/core/__init__.py", "xarray.core", "python", &[]),
        ];
        assert!(module_path_root_prefix(&files[0], ".").is_none());
        assert!(module_path_prefix_candidates(&files[0], "xarray.core", ".").is_empty());

        let known_modules: HashSet<_> = files.iter().map(|f| f.module_path.clone()).collect();
        let module_edges = build_import_edges(&files, &known_modules, &JsWorkspace::default());
        assert_eq!(module_edges.len(), 1);
        assert_eq!(module_edges[0].module, "xarray.core");

        let module_to_file: HashMap<String, String> = files
            .iter()
            .map(|f| (f.module_path.clone(), f.path.clone()))
            .collect();
        let file_edges = build_file_import_edges(&files, &module_to_file, &JsWorkspace::default());
        assert_eq!(
            file_edges
                .iter()
                .map(|e| (e.source.as_str(), e.target.as_str(), e.import_count))
                .collect::<Vec<_>>(),
            vec![("xarray/core/dataset.py", "xarray/core/__init__.py", 1)]
        );
    }

    /// The branch is language-gated: a relative specifier in a language that
    /// does not use module-path imports must not pick up the new behaviour.
    #[test]
    fn module_path_candidates_are_language_gated() {
        let ts = source_file("a/b.ts", "a/b", "typescript", &["./c"]);
        assert_eq!(
            module_path_import_candidates(&ts, "./c", &JsWorkspace::default()),
            vec!["a/c"]
        );

        // Python is not in the TS/JS relative-specifier branch and never joins
        // it: `./c` is not Python spelling, and relative imports are dropped at
        // parse time.
        let py = source_file("a/b.py", "root.a.b", "python", &["./c"]);
        assert!(module_path_import_candidates(&py, "./c", &JsWorkspace::default()).is_empty());
        assert!(module_path_prefix_candidates(&py, "./c", ".").is_empty());
        // What Python DOES get is the separate root-prefix pass, on absolute
        // specifiers only: the full specifier under every candidate import root
        // (shallowest first) before any shortening, and never shortened below
        // one segment — the bare `root` is not a candidate.
        assert_eq!(
            module_path_prefix_candidates(&py, "c.d", "."),
            vec![
                "root.c.d".to_string(),
                "root.a.c.d".to_string(),
                "root.c".to_string(),
                "root.a.c".to_string(),
            ]
        );
        // TS/JS get nothing from that pass, at any separator.
        assert!(module_path_prefix_candidates(&ts, "a/c", "/").is_empty());

        let js = source_file("a/b.js", "a/b", "javascript", &["../d/index.js"]);
        assert_eq!(
            module_path_import_candidates(&js, "../d/index.js", &JsWorkspace::default()),
            vec!["d/index", "d"]
        );

        // Non-relative specifiers are left to the caller's prefix walk.
        assert!(module_path_import_candidates(&ts, "zod", &JsWorkspace::default()).is_empty());
        assert!(
            module_path_import_candidates(&ts, "@scope/core", &JsWorkspace::default()).is_empty()
        );
        // Escaping the project root yields nothing rather than a bogus path.
        assert!(
            module_path_import_candidates(&ts, "../../../outside", &JsWorkspace::default())
                .is_empty()
        );
    }

    #[test]
    fn swift_target_import_resolves_as_a_namespace() {
        let files = vec![source_file(
            "Sources/Helpers/Test.swift",
            "Helpers.Test",
            "swift",
            &["ArgumentParser"],
        )];
        let known_modules = HashSet::from(["ArgumentParser".to_string()]);
        let edges = build_import_edges(&files, &known_modules, &JsWorkspace::default());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].module, "ArgumentParser");
    }

    #[test]
    fn file_import_edges_are_key_sorted() {
        // RE-FOUNDED 2026-08-15 (program B1). The old fixture hand-built
        // `module_to_file` as `"crate::alpha" -> "src/alpha.rs"` — a shape the
        // real builder never emitted (it derives `crate::src::alpha`), so the
        // test was green while production resolved nothing (mcp-servers
        // report, finding 8). Every module path below now comes from the REAL
        // `RustParser::file_to_module_path`, so this fixture cannot drift from
        // production again: if the derivation changes, this test sees it.
        use crate::parsers::rust_lang::RustParser;
        use std::path::Path;
        let root = Path::new("");
        let mp = |p: &str| RustParser::file_to_module_path(Path::new(p), root);
        let source = FileInfo {
            path: "src/lib.rs".into(),
            language: "rust".into(),
            module_path: mp("src/lib.rs"),
            imports: vec!["crate::beta".into(), "crate::alpha".into()],
            ..FileInfo::default()
        };
        let module_to_file = HashMap::from([
            (mp("src/alpha.rs"), "src/alpha.rs".to_string()),
            (mp("src/beta.rs"), "src/beta.rs".to_string()),
        ]);

        let edges = build_file_import_edges(&[source], &module_to_file, &JsWorkspace::default());
        let pairs: Vec<_> = edges
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("src/lib.rs", "src/alpha.rs"),
                ("src/lib.rs", "src/beta.rs"),
            ]
        );
    }

    #[test]
    fn uses_type_edges_are_pattern_sorted() {
        let function = FunctionInfo {
            qualified_name: "crate::combine".into(),
            parameters: vec![
                ParameterInfo {
                    type_annotation: Some("Beta".into()),
                    ..ParameterInfo::default()
                },
                ParameterInfo {
                    type_annotation: Some("Alpha".into()),
                    ..ParameterInfo::default()
                },
            ],
            ..FunctionInfo::default()
        };
        let classes = vec![
            ClassInfo {
                name: "Beta".into(),
                qualified_name: "crate::beta::Beta".into(),
                kind: "struct".into(),
                ..ClassInfo::default()
            },
            ClassInfo {
                name: "Alpha".into(),
                qualified_name: "crate::alpha::Alpha".into(),
                kind: "struct".into(),
                ..ClassInfo::default()
            },
        ];

        let edges = build_uses_type_edges(&[function], &classes, &[], &[]);
        let targets: Vec<_> = edges["Struct"]
            .iter()
            .map(|edge| edge.type_name.as_str())
            .collect();
        assert_eq!(targets, vec!["crate::alpha::Alpha", "crate::beta::Beta"]);
    }
}

/// D3 is a property of every bare-name index over the function population,
/// not just `build_call_edges`. These two builders resolve a bare name to a
/// *globally unique* function, which is exactly the shape a nested `wrapper`
/// / `inner` / `decorator` satisfies — so before the gate they minted
/// cross-file edges into closure-scoped definitions that no caller in that
/// file can even see.
#[cfg(test)]
mod nested_visibility_tests {
    use super::*;
    use crate::models::FunctionInfo;

    fn top_level(qname: &str, file: &str, name: &str) -> FunctionInfo {
        FunctionInfo {
            name: name.into(),
            qualified_name: qname.into(),
            file_path: file.into(),
            ..FunctionInfo::default()
        }
    }

    fn nested(qname: &str, file: &str, name: &str) -> FunctionInfo {
        let mut f = top_level(qname, file, name);
        f.metadata
            .insert("nesting_depth".into(), serde_json::json!(1));
        f
    }

    #[test]
    fn a_nested_definition_is_not_a_cross_file_references_fn_target() {
        let mut referrer = top_level("b.consume", "b.py", "consume");
        referrer.function_refs = vec![("wrapper".into(), 4)];
        let functions = vec![referrer, nested("a.deco.wrapper", "a.py", "wrapper")];
        assert!(
            build_references_fn_edges(&functions).is_empty(),
            "a closure-scoped `wrapper` in another file is not referable"
        );
    }

    #[test]
    fn a_nested_definition_is_a_same_file_references_fn_target() {
        let mut referrer = top_level("a.deco", "a.py", "deco");
        referrer.function_refs = vec![("wrapper".into(), 4)];
        let functions = vec![referrer, nested("a.deco.wrapper", "a.py", "wrapper")];
        let pairs: Vec<_> = build_references_fn_edges(&functions)
            .into_iter()
            .map(|e| (e.caller, e.callee))
            .collect();
        assert_eq!(pairs, vec![("a.deco".into(), "a.deco.wrapper".into())]);
    }

    #[test]
    fn a_nested_definition_is_not_a_cross_file_decorator() {
        let mut decorated = top_level("b.handler", "b.py", "handler");
        decorated.decorators = vec!["wrapper".into()];
        let functions = vec![decorated, nested("a.deco.wrapper", "a.py", "wrapper")];
        assert!(
            build_decorates_edges(&functions).is_empty(),
            "a closure-scoped `wrapper` in another file cannot decorate"
        );
    }

    #[test]
    fn a_nested_definition_still_decorates_within_its_file() {
        let mut decorated = top_level("a.handler", "a.py", "handler");
        decorated.decorators = vec!["wrapper".into()];
        let functions = vec![decorated, nested("a.deco.wrapper", "a.py", "wrapper")];
        let pairs: Vec<_> = build_decorates_edges(&functions)
            .into_iter()
            .map(|e| (e.decorator, e.function))
            .collect();
        assert_eq!(pairs, vec![("a.deco.wrapper".into(), "a.handler".into())]);
    }

    /// The gate is by visibility, not by deletion: a nested name must not
    /// shadow an identically named top-level export for a cross-file
    /// referrer, nor make it look ambiguous.
    #[test]
    fn a_nested_name_does_not_disturb_the_global_index() {
        let mut referrer = top_level("b.consume", "b.py", "consume");
        referrer.function_refs = vec![("wrapper".into(), 4)];
        let functions = vec![
            referrer,
            top_level("c.wrapper", "c.py", "wrapper"),
            nested("a.deco.wrapper", "a.py", "wrapper"),
        ];
        let pairs: Vec<_> = build_references_fn_edges(&functions)
            .into_iter()
            .map(|e| (e.caller, e.callee))
            .collect();
        assert_eq!(pairs, vec![("b.consume".into(), "c.wrapper".into())]);
    }
}
