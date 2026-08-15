//! Python language parser (ported from parsers/python.py).

use regex::Regex;
use serde_json::json;
use std::path::Path;
use std::sync::OnceLock;
use tree_sitter::{Node, Parser, Tree};

use super::shared::{
    break_qualified_name_ties, compute_complexity, count_lines, extend_scope_chain,
    extract_comment_annotations, extract_procedure_annotations, get_type_parameters,
    is_generated_or_minified, node_text, scope_qualify, tag_scope, BRANCH_KINDS_PYTHON,
    DEFAULT_COMMENT_TYPES,
};
use super::LanguageParser;
use crate::models::{
    AttributeInfo, ClassInfo, ConstantInfo, EnumInfo, FileInfo, FunctionInfo, InterfaceInfo,
    ParameterInfo, ParameterKind, ParseResult, TypeRelationship,
};

const ENUM_BASES: &[&str] = &["Enum", "IntEnum", "StrEnum", "Flag", "IntFlag", "auto"];

/// Identifier is ALL_CAPS (module-level constant).
fn constant_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Z][A-Z_0-9]+$").expect("constant regex compiles"))
}

/// Framework-specific identifiers that break SCREAMING_SNAKE convention
/// but warrant treatment as top-level constants. Currently:
///   - `urlpatterns` — the canonical Django URL-routing list. Picked up
///     by the route-extractor pass.
fn is_framework_constant(name: &str) -> bool {
    matches!(name, "urlpatterns")
}

pub const PYTHON_NOISE_NAMES: &[&str] = &[
    "len",
    "str",
    "int",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "print",
    "isinstance",
    "issubclass",
    "type",
    "range",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sorted",
    "reversed",
    "any",
    "all",
    "min",
    "max",
    "sum",
    "abs",
    "round",
    "hash",
    "id",
    "repr",
    "super",
    "getattr",
    "setattr",
    "hasattr",
    "delattr",
    "callable",
    "iter",
    "next",
    "open",
    "format",
    "append",
    "extend",
    "update",
    "pop",
    "get",
    "keys",
    "values",
    "items",
    "join",
    "split",
    "strip",
    "replace",
    "startswith",
    "endswith",
];

const NESTED_SCOPES: &[&str] = &["function_definition", "lambda", "decorated_definition"];

/// Scopes that own their *own* graph node — the call-walk stops here so a
/// nested definition's calls aren't attributed to the enclosing function.
/// Unlike [`NESTED_SCOPES`] (used by complexity) this excludes `lambda`:
/// a lambda gets no node, so calls inside it belong to the enclosing fn
/// (mirrors the Rust closure handling).
///
/// D4 — this comment used to be a *promise* the parser did not keep. Until
/// the nested-scope walk landed, nothing node-ified a `def` inside a function
/// body, so the calls skipped here were skipped by the extractor **and**
/// attributed to nobody: they left the graph entirely. Every kind listed here
/// is now node-ified whenever its enclosing chain is named (which, in Python,
/// is always — see [`ANONYMOUS_SCOPES`]), so each call site belongs to exactly
/// one `Function`: the nearest enclosing definition. TS needs an explicit node
/// id skip set on top of its equivalent list because it node-ifies literals
/// the list does not name; Python does not, because every definition it
/// node-ifies is a `function_definition` or a `decorated_definition` and is
/// therefore already a member of this set. The
/// `nested_scope_tests::every_skipped_scope_is_node_ified` test is what keeps
/// that true.
const NAMED_NESTED_SCOPES: &[&str] = &["function_definition", "decorated_definition"];

/// D1 clause 5 for Python — the scopes the grammar cannot name.
///
/// In TS this is where the rule earns its keep: anonymous callbacks are
/// everywhere, and refusing a node to a binding underneath one is worth 713
/// nodes on opencode. Python is the opposite case, and it is worth being
/// precise about why rather than porting the TS shape on faith:
///
///   * Every construct that can lexically *contain* a `def` — the module, a
///     `class_definition`, a `function_definition` — is named by the grammar.
///     Python has no anonymous function *statement*.
///   * The two scopes Python does leave unnamed are the ones listed here: a
///     `lambda` and the four comprehension forms (which really are separate
///     scopes in Python 3). Both have **expression-only** bodies, so no `def`
///     or `class` statement can ever appear inside one.
///   * `if` / `for` / `while` / `with` / `try` / `match` blocks are not scopes
///     at all in Python — they share the enclosing function's namespace — so
///     they are *transparent*: they contribute neither a name segment nor a
///     nesting level, and `def a()` inside an `if` inside `outer` is
///     `module.outer.a` at depth 1 exactly as if the `if` were not there.
///     (That is also why same-named `def`s in an `if`/`else` or `try`/`except`
///     pair are a routine duplicate-qualified-name collision in Python where
///     they are a rarity in TS — see `break_qualified_name_ties`.)
///
/// So the clause is structurally vacuous here: no definition can ever sit
/// below an unnamed scope. It is implemented anyway, as the same absorbing
/// `Option<&[String]>` chain TS uses, so that `parent_scope` means the same
/// thing in both parsers — and entering one of these prunes the walk, which
/// `holds_a_definition` asserts is lossless on every debug build.
const ANONYMOUS_SCOPES: &[&str] = &[
    "lambda",
    "list_comprehension",
    "set_comprehension",
    "dictionary_comprehension",
    "generator_expression",
];

pub struct PythonParser;

thread_local! {
    static PY_PARSER: std::cell::RefCell<Parser> = {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("loading tree-sitter-python grammar");
        std::cell::RefCell::new(p)
    };
}

impl PythonParser {
    pub fn new() -> Self {
        PythonParser
    }

    fn parse_tree(&self, source: &[u8]) -> Option<Tree> {
        PY_PARSER.with(|p| p.borrow_mut().parse(source, None))
    }

    // ── Small helpers ───────────────────────────────────────────────

    fn get_visibility(name: &str) -> &'static str {
        if name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__")) {
            "private"
        } else {
            "public"
        }
    }

    fn get_name<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                return node_text(child, source);
            }
        }
        "unknown"
    }

    fn get_block<'a>(node: Node<'a>) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        let child = node
            .children(&mut cursor)
            .find(|child| child.kind() == "block");
        child
    }

    fn get_docstring(node: Node, source: &[u8]) -> Option<String> {
        let block = Self::get_block(node)?;
        let mut cursor = block.walk();
        for child in block.children(&mut cursor) {
            match child.kind() {
                "expression_statement" => {
                    let mut sub_cursor = child.walk();
                    for sub in child.children(&mut sub_cursor) {
                        if sub.kind() == "string" {
                            let raw = node_text(sub, source);
                            for delim in ["\"\"\"", "'''", "\"", "'"] {
                                if raw.starts_with(delim) && raw.ends_with(delim) {
                                    let inner = &raw[delim.len()..raw.len() - delim.len()];
                                    return Some(inner.trim().to_string());
                                }
                            }
                            return Some(raw.to_string());
                        }
                    }
                    return None;
                }
                "comment" => continue,
                _ => return None,
            }
        }
        None
    }

    fn get_bases(node: Node, source: &[u8]) -> Vec<String> {
        let mut bases = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "argument_list" {
                continue;
            }
            let mut arg_cursor = child.walk();
            for arg in child.children(&mut arg_cursor) {
                match arg.kind() {
                    "identifier" | "attribute" => {
                        bases.push(node_text(arg, source).to_string());
                    }
                    "subscript" => {
                        // e.g. Generic[T], Protocol[T] → take the base name
                        let mut sub_cursor = arg.walk();
                        for sub in arg.children(&mut sub_cursor) {
                            if matches!(sub.kind(), "identifier" | "attribute") {
                                bases.push(node_text(sub, source).to_string());
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        bases
    }

    fn get_decorators(decorated: Node, source: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = decorated.walk();
        for child in decorated.children(&mut cursor) {
            if child.kind() == "decorator" {
                let text = node_text(child, source).trim();
                let stripped = text.strip_prefix('@').unwrap_or(text);
                out.push(stripped.to_string());
            }
        }
        out
    }

    fn get_decorated_inner<'a>(node: Node<'a>) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "function_definition" | "class_definition") {
                return Some(child);
            }
        }
        None
    }

    fn get_signature(node: Node, source: &[u8]) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "block" => break,
                "comment" => continue,
                _ => parts.push(node_text(child, source)),
            }
        }
        parts.join(" ").trim_end_matches([' ', ':']).to_string()
    }

    fn get_return_type(node: Node, source: &[u8]) -> Option<String> {
        let mut saw_arrow = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() && node_text(child, source) == "->" {
                saw_arrow = true;
            } else if saw_arrow {
                match child.kind() {
                    ":" | "block" => return None,
                    _ => return Some(node_text(child, source).to_string()),
                }
            }
        }
        None
    }

    fn is_async(node: Node, source: &[u8]) -> bool {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                let text = node_text(child, source);
                if text == "async" {
                    return true;
                }
                if text == "def" {
                    break;
                }
            }
        }
        false
    }

    /// Extract structured parameters from a Python function definition.
    /// Excludes implicit `self`/`cls`. Distinguishes `*args` (Variadic) and
    /// `**kwargs` (KwVariadic). `default` carries the raw default expression text.
    fn extract_parameters(node: Node, source: &[u8]) -> Vec<ParameterInfo> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        let Some(params_node) = node
            .children(&mut cursor)
            .find(|c| c.kind() == "parameters")
        else {
            return out;
        };
        let mut pcursor = params_node.walk();
        for child in params_node.children(&mut pcursor) {
            let kind = child.kind();
            let (name, type_ann, default, param_kind) = match kind {
                "identifier" => {
                    let n = node_text(child, source).to_string();
                    if matches!(n.as_str(), "self" | "cls") {
                        continue;
                    }
                    (n, None, None, ParameterKind::Positional)
                }
                "typed_parameter" => {
                    let mut name: Option<String> = None;
                    let mut type_ann: Option<String> = None;
                    let mut tcursor = child.walk();
                    for sub in child.children(&mut tcursor) {
                        match sub.kind() {
                            "identifier" if name.is_none() => {
                                name = Some(node_text(sub, source).to_string())
                            }
                            "type" => type_ann = Some(node_text(sub, source).to_string()),
                            _ => {}
                        }
                    }
                    let Some(n) = name else { continue };
                    if matches!(n.as_str(), "self" | "cls") {
                        continue;
                    }
                    (n, type_ann, None, ParameterKind::Positional)
                }
                "default_parameter" | "typed_default_parameter" => {
                    let mut name: Option<String> = None;
                    let mut type_ann: Option<String> = None;
                    let mut default: Option<String> = None;
                    let mut saw_eq = false;
                    let mut dcursor = child.walk();
                    for sub in child.children(&mut dcursor) {
                        if !sub.is_named() && node_text(sub, source) == "=" {
                            saw_eq = true;
                            continue;
                        }
                        match sub.kind() {
                            "identifier" if name.is_none() && !saw_eq => {
                                name = Some(node_text(sub, source).to_string())
                            }
                            "type" if !saw_eq => {
                                type_ann = Some(node_text(sub, source).to_string())
                            }
                            _ if saw_eq && sub.is_named() && default.is_none() => {
                                default = Some(node_text(sub, source).to_string());
                            }
                            _ => {}
                        }
                    }
                    let Some(n) = name else { continue };
                    if matches!(n.as_str(), "self" | "cls") {
                        continue;
                    }
                    (n, type_ann, default, ParameterKind::Positional)
                }
                "list_splat_pattern" => {
                    // *args
                    let n = node_text(child, source).trim_start_matches('*').to_string();
                    if n.is_empty() {
                        continue;
                    }
                    (n, None, None, ParameterKind::Variadic)
                }
                "dictionary_splat_pattern" => {
                    // **kwargs
                    let n = node_text(child, source).trim_start_matches('*').to_string();
                    if n.is_empty() {
                        continue;
                    }
                    (n, None, None, ParameterKind::KwVariadic)
                }
                _ => continue,
            };
            out.push(ParameterInfo {
                name,
                type_annotation: type_ann,
                default,
                kind: param_kind,
            });
        }
        out
    }

    fn extract_calls(body: Node, source: &[u8]) -> Vec<(String, u32)> {
        let mut calls: Vec<(String, u32)> = Vec::new();
        fn walk(node: Node, source: &[u8], out: &mut Vec<(String, u32)>) {
            if node.kind() == "call" {
                let line = node.start_position().row as u32 + 1;
                if let Some(func) = node.child(0) {
                    match func.kind() {
                        "identifier" => {
                            out.push((node_text(func, source).to_string(), line));
                        }
                        "attribute" => {
                            let mut parts: Vec<&str> = Vec::new();
                            let mut cursor = func.walk();
                            for child in func.children(&mut cursor) {
                                if child.kind() == "identifier" {
                                    parts.push(node_text(child, source));
                                }
                            }
                            if parts.len() >= 2 {
                                let receiver = parts[parts.len() - 2];
                                let method = parts[parts.len() - 1];
                                if receiver == "self" || receiver == "cls" {
                                    out.push((method.to_string(), line));
                                } else {
                                    out.push((format!("{}.{}", receiver, method), line));
                                }
                            } else if let Some(last) = parts.last() {
                                out.push(((*last).to_string(), line));
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if !NAMED_NESTED_SCOPES.contains(&child.kind()) {
                    walk(child, source, out);
                }
            }
        }
        walk(body, source, &mut calls);
        calls
    }

    /// Function-value arguments — `map(helper, xs)`, `sorted(xs, key=helper)`,
    /// `register(callback)` — are *references*, not call sites: the function
    /// is passed by value, not invoked here. Recorded separately so the
    /// builder can emit `REFERENCES_FN` edges (and so a function only ever
    /// passed as a callback isn't reported as dead code).
    ///
    /// Unlike Rust, Python can't use case to tell a function name from a
    /// variable (both are snake_case), so we record every bare-identifier
    /// argument and let the builder keep only those that resolve to a known
    /// project function. Descends like `extract_calls` (into lambdas, not
    /// into nested named definitions).
    fn extract_function_pointer_refs(body: Node, source: &[u8]) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = Vec::new();
        fn walk(node: Node, source: &[u8], out: &mut Vec<(String, u32)>) {
            if node.kind() == "call" {
                if let Some(args) = node.child_by_field_name("arguments") {
                    let mut cursor = args.walk();
                    for arg in args.children(&mut cursor) {
                        let ident = match arg.kind() {
                            "identifier" => Some(arg),
                            // `key=helper` — the value side of a keyword arg.
                            "keyword_argument" => arg
                                .child_by_field_name("value")
                                .filter(|v| v.kind() == "identifier"),
                            _ => None,
                        };
                        if let Some(id) = ident {
                            let text = node_text(id, source);
                            if text.len() >= 2 {
                                out.push((text.to_string(), id.start_position().row as u32 + 1));
                            }
                        }
                    }
                }
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if !NAMED_NESTED_SCOPES.contains(&child.kind()) {
                    walk(child, source, out);
                }
            }
        }
        walk(body, source, &mut out);
        out.sort();
        out.dedup();
        out
    }

    fn file_to_module_path(filepath: &Path, src_root: &Path) -> String {
        let rel = filepath.strip_prefix(src_root).unwrap_or(filepath);
        let mut parts: Vec<String> = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
            .collect();
        if let Some(last) = parts.last_mut() {
            if let Some(stem) = last.strip_suffix(".pyi") {
                *last = stem.to_string();
            } else if let Some(stem) = last.strip_suffix(".py") {
                *last = stem.to_string();
            }
            if last == "__init__" {
                parts.pop();
            }
        }
        let pkg = src_root.file_name().and_then(|o| o.to_str()).unwrap_or("");
        if parts.is_empty() {
            pkg.to_string()
        } else if pkg.is_empty() || parts.first().map(String::as_str) == Some(pkg) {
            // Don't double the package name. In the common clone layout the
            // source root's own directory name *is* the top-level package
            // (`<repo>/xarray/xarray/core/...`, where the clone dir and the
            // package share a name), so the relative path already begins with
            // `pkg`. Prepending it again yields `xarray.xarray.core`; the file
            // path stays clean (`xarray/core/...`) and the import-derived root
            // Module node is the single `xarray`, so the doubled form is the
            // odd one out. Emit the relative path as-is when it already leads
            // with the package.
            parts.join(".")
        } else {
            format!("{}.{}", pkg, parts.join("."))
        }
    }

    fn get_enum_variants(node: Node, source: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let Some(block) = Self::get_block(node) else {
            return out;
        };
        let mut cursor = block.walk();
        for child in block.children(&mut cursor) {
            if child.kind() == "expression_statement" {
                let mut sub_cursor = child.walk();
                for sub in child.children(&mut sub_cursor) {
                    if sub.kind() == "assignment" {
                        let mut tgt_cursor = sub.walk();
                        for target in sub.children(&mut tgt_cursor) {
                            if target.kind() == "identifier" {
                                out.push(node_text(target, source).to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Every origin a single import statement names (mcp-servers report
    /// 2026-08-14, findings 3+4 — the old `Option<String>` shape kept only the
    /// first `dotted_name` and dropped relative and aliased forms entirely).
    ///
    /// * `import a, b` → `["a", "b"]`; `import a.b as c` → `["a.b"]` — an
    ///   alias renames the binding, never the origin.
    /// * `from pkg import sub` → `["pkg.sub"]`: the full dotted origin, so
    ///   the edge can land on `pkg/sub.py` when `sub` is a module; the
    ///   resolver's longest→shortest walk falls back to `pkg` when `sub` is
    ///   a symbol. `from pkg import *` → `["pkg"]`.
    /// * A MULTI-name from-import (`from pkg import a, b, c`) deliberately
    ///   keeps the legacy single `["pkg"]` — byte-identical to what the old
    ///   extractor emitted for that shape. Expanding it per name would count
    ///   one statement as N imports (`import_count` 1 → 3 on the frozen
    ///   `py_nested_defs` golden) and emit N parallel File→Module edges; if
    ///   per-name expansion is ever wanted so `from pkg import mod_a, mod_b`
    ///   can land on both module files, that is a conscious golden-moving
    ///   change of its own, not part of the findings-3–5 fix.
    /// * Relative forms keep their dots verbatim (`from . import util` →
    ///   `[".util"]`, `from ..util import helper` → `["..util.helper"]`);
    ///   `python_relative_import_candidates` in `builder/other_edges.rs`
    ///   rewrites them against the importing file's own `module_path`, the
    ///   same division of labour as Rust's `use`-path handling (B1).
    fn parse_imports(node: Node, source: &[u8]) -> Vec<String> {
        // The `name` field covers both statement kinds in tree-sitter-python:
        // each is a `dotted_name` or an `aliased_import` whose own `name`
        // field is the origin.
        let mut names: Vec<String> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children_by_field_name("name", &mut cursor) {
            let origin = if child.kind() == "aliased_import" {
                child.child_by_field_name("name")
            } else {
                Some(child)
            };
            if let Some(origin) = origin {
                names.push(node_text(origin, source).to_string());
            }
        }
        match node.kind() {
            "import_statement" => names,
            "import_from_statement" => {
                let Some(module) = node.child_by_field_name("module_name") else {
                    return Vec::new();
                };
                let module_text = node_text(module, source).to_string();
                // Wildcard and multi-name forms: the module itself is the
                // origin (see the doc comment for why multi-name is NOT
                // expanded per name).
                if names.len() != 1 {
                    return vec![module_text];
                }
                // A relative prefix already ends in `.`; don't double it.
                let joiner = if module_text.ends_with('.') { "" } else { "." };
                vec![format!("{module_text}{joiner}{}", names[0])]
            }
            _ => Vec::new(),
        }
    }

    /// Collect every import statement in the file, wherever it sits
    /// (mcp-servers report 2026-08-14, finding 5 — the root `children()` loop
    /// never saw imports under `if TYPE_CHECKING:`, `try/except ImportError:`
    /// or inside function bodies, all of which are real dependencies).
    ///
    /// A single depth-first walk visits each statement exactly once, so an
    /// import is never double-counted by overlapping traversals; two distinct
    /// statements naming the same origin still count twice, which is what the
    /// file-edge `import_count` aggregation records.
    fn collect_imports(node: Node, source: &[u8], out: &mut Vec<String>) {
        match node.kind() {
            "import_statement" | "import_from_statement" => {
                out.extend(Self::parse_imports(node, source));
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    Self::collect_imports(child, source, out);
                }
            }
        }
    }

    fn classify_decorators(decorators: &[String]) -> Vec<(&'static str, serde_json::Value)> {
        let mut flags: Vec<(&'static str, serde_json::Value)> = Vec::new();
        for dec in decorators {
            let base = dec.split('(').next().unwrap_or("");
            let base = base.rsplit('.').next().unwrap_or("");
            let flag = match base {
                "abstractmethod" => "is_abstract",
                "property" => "is_property",
                "staticmethod" => "is_static",
                "classmethod" => "is_classmethod",
                "overload" => "is_overload",
                _ => continue,
            };
            flags.push((flag, json!(true)));
        }
        flags
    }

    // ── Class body extraction ──────────────────────────────────────

    fn extract_class_attributes(
        class_node: Node,
        source: &[u8],
        owner_qname: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let Some(block) = Self::get_block(class_node) else {
            return;
        };

        // 1. Class-body assignments: x = value, x: type = value
        let mut cursor = block.walk();
        for child in block.children(&mut cursor) {
            if child.kind() != "expression_statement" {
                continue;
            }
            let mut sub_cursor = child.walk();
            for sub in child.children(&mut sub_cursor) {
                if sub.kind() != "assignment" {
                    continue;
                }
                let mut attr_name: Option<String> = None;
                let mut type_ann: Option<String> = None;
                let mut default_val: Option<String> = None;

                let mut scan = sub.walk();
                for sc in sub.children(&mut scan) {
                    if sc.kind() == "identifier" && attr_name.is_none() {
                        attr_name = Some(node_text(sc, source).to_string());
                    } else if sc.kind() == "type" {
                        type_ann = Some(node_text(sc, source).to_string());
                    }
                }

                let Some(attr_name) = attr_name else {
                    continue;
                };
                if constant_re().is_match(&attr_name) {
                    continue;
                }

                let mut saw_eq = false;
                let mut scan2 = sub.walk();
                for sc in sub.children(&mut scan2) {
                    if !sc.is_named() && node_text(sc, source) == "=" {
                        saw_eq = true;
                    } else if saw_eq && sc.is_named() {
                        let val = node_text(sc, source);
                        let take = val
                            .char_indices()
                            .nth(100)
                            .map(|(i, _)| i)
                            .unwrap_or(val.len());
                        default_val = Some(val[..take].to_string());
                        break;
                    }
                }

                result.attributes.push(AttributeInfo {
                    qualified_name: format!("{}.{}", owner_qname, attr_name),
                    owner_qualified_name: owner_qname.to_string(),
                    visibility: Self::get_visibility(&attr_name).to_string(),
                    name: attr_name,
                    type_annotation: type_ann,
                    file_path: rel_path.to_string(),
                    line_number: child.start_position().row as u32 + 1,
                    default_value: default_val,
                });
            }
        }

        // 2. self.x assignments in __init__
        let mut seen_names: std::collections::HashSet<String> = result
            .attributes
            .iter()
            .filter(|a| a.owner_qualified_name == owner_qname)
            .map(|a| a.name.clone())
            .collect();

        let mut cursor2 = block.walk();
        for child in block.children(&mut cursor2) {
            let fn_node = match child.kind() {
                "function_definition" => Some(child),
                "decorated_definition" => {
                    Self::get_decorated_inner(child).filter(|n| n.kind() == "function_definition")
                }
                _ => None,
            };
            if let Some(fn_node) = fn_node {
                if Self::get_name(fn_node, source) == "__init__" {
                    if let Some(init_block) = Self::get_block(fn_node) {
                        Self::walk_self_attrs(
                            init_block,
                            source,
                            owner_qname,
                            rel_path,
                            result,
                            &mut seen_names,
                        );
                    }
                    break;
                }
            }
        }
    }

    fn walk_self_attrs(
        node: Node,
        source: &[u8],
        owner_qname: &str,
        rel_path: &str,
        result: &mut ParseResult,
        seen_names: &mut std::collections::HashSet<String>,
    ) {
        if node.kind() == "assignment" {
            if let Some(left) = node.child(0) {
                if left.kind() == "attribute" {
                    let text = node_text(left, source);
                    if let Some(attr_name) = text.strip_prefix("self.") {
                        if !attr_name.contains('.') && !seen_names.contains(attr_name) {
                            let owned = attr_name.to_string();
                            seen_names.insert(owned.clone());
                            let mut default_val: Option<String> = None;
                            let mut saw_eq = false;
                            let mut cursor = node.walk();
                            for sc in node.children(&mut cursor) {
                                if !sc.is_named() && node_text(sc, source) == "=" {
                                    saw_eq = true;
                                } else if saw_eq && sc.is_named() {
                                    let val = node_text(sc, source);
                                    let take = val
                                        .char_indices()
                                        .nth(100)
                                        .map(|(i, _)| i)
                                        .unwrap_or(val.len());
                                    default_val = Some(val[..take].to_string());
                                    break;
                                }
                            }
                            result.attributes.push(AttributeInfo {
                                qualified_name: format!("{}.{}", owner_qname, owned),
                                owner_qualified_name: owner_qname.to_string(),
                                visibility: Self::get_visibility(&owned).to_string(),
                                name: owned,
                                type_annotation: None,
                                file_path: rel_path.to_string(),
                                line_number: node.start_position().row as u32 + 1,
                                default_value: default_val,
                            });
                        }
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for c in node.children(&mut cursor) {
            Self::walk_self_attrs(c, source, owner_qname, rel_path, result, seen_names);
        }
    }

    // ── Nested scope walk (D1 as amended / D2) ──────────────────────
    //
    // `parse_file` used to look only at the direct children of the module
    // root, so a `def` inside a function body was never visited: decorator
    // factories, closure factories and nested helpers had no node, and — see
    // `NAMED_NESTED_SCOPES` — their calls were dropped rather than
    // re-attributed. The walk below descends into definition bodies.
    //
    // What gets a node is D1 clause 1 (`function_definition`, named by the
    // grammar) plus clause 5 (`ANONYMOUS_SCOPES`). Clauses 2–4 are TS-only:
    // Python binds functions by `def`, so there is no fn-literal binding to
    // recognise and no factory unwrap to narrow — a `x = functools.wraps(f)(g)`
    // RHS unwrap is explicitly deferred, not forgotten.

    /// The `FunctionInfo` for a nested definition: `parse_function` plus the
    /// D2 identity (chain-qualified name) and the two scope properties.
    /// `chain` is the enclosing chain and excludes the definition's own name.
    fn nested_function(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: &[String],
        depth: u32,
    ) -> FunctionInfo {
        let mut info = Self::parse_function(node, source, module_path, rel_path, false, None);
        info.qualified_name = scope_qualify(module_path, chain, &info.name);
        tag_scope(&mut info, module_path, chain, depth);
        info
    }

    /// Walk the statements of `block` for nested definitions, emitting them
    /// into `out` in source order.
    ///
    /// `chain` is the scope chain *including* the block owner's own name;
    /// `depth` is the nesting depth of whatever the block declares.
    fn descend_block(
        block: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
    ) {
        // A `None` chain is *absorbing*: `extend_scope_chain` maps `None` to
        // `None` and no arm below ever rebuilds a `Some`, so nothing under an
        // unnamed scope can be node-ified. Returning here is observably
        // identical to descending and declining everything.
        if chain.is_none() {
            return;
        }
        let mut cursor = block.walk();
        for statement in block.named_children(&mut cursor) {
            Self::walk_scope(statement, source, module_path, rel_path, chain, depth, out);
        }
    }

    /// The recursive scope walk. `node` is any node inside some scope;
    /// `chain` / `depth` describe that scope.
    fn walk_scope(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
    ) {
        match node.kind() {
            "function_definition" => {
                Self::walk_definition(node, None, source, module_path, rel_path, chain, depth, out);
            }
            "decorated_definition" => {
                let Some(inner) = Self::get_decorated_inner(node) else {
                    return;
                };
                match inner.kind() {
                    "function_definition" => Self::walk_definition(
                        inner,
                        Some(node),
                        source,
                        module_path,
                        rel_path,
                        chain,
                        depth,
                        out,
                    ),
                    "class_definition" => Self::walk_class_scope(
                        inner,
                        source,
                        module_path,
                        rel_path,
                        chain,
                        depth,
                        out,
                    ),
                    _ => {}
                }
            }
            "class_definition" => {
                Self::walk_class_scope(node, source, module_path, rel_path, chain, depth, out);
            }
            // D1 clause 5. The chain would become `None` here, which is
            // absorbing, so the subtree is pruned instead of descended —
            // provably lossless, because the grammar cannot put a definition
            // inside an expression-bodied scope.
            kind if ANONYMOUS_SCOPES.contains(&kind) => {
                debug_assert!(
                    !Self::holds_a_definition(node),
                    "an unnamed Python scope ({kind}) held a definition — D1 \
                     clause 5 is no longer vacuous and the prune is lossy"
                );
            }
            // Everything else — `if`, `for`, `while`, `with`, `try`, `match`,
            // plain expressions — is not a Python scope, so it is transparent:
            // same chain, same depth.
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    Self::walk_scope(child, source, module_path, rel_path, chain, depth, out);
                }
            }
        }
    }

    /// One nested `def`, optionally wrapped in a `decorated_definition`.
    #[allow(clippy::too_many_arguments)]
    fn walk_definition(
        node: Node,
        decorated: Option<Node>,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
    ) {
        let name = Self::get_name(node, source).to_string();
        if let Some(chain) = chain {
            let mut info = Self::nested_function(node, source, module_path, rel_path, chain, depth);
            if let Some(decorated) = decorated {
                let decorators = Self::get_decorators(decorated, source);
                for (flag, value) in Self::classify_decorators(&decorators) {
                    info.metadata.insert(flag.to_string(), value);
                }
                info.decorators = decorators;
            }
            out.push(info);
        }
        // Emitted parent-first, so `out` stays in source order for the
        // duplicate tie-break. Declining a definition does not decline the
        // scope below it — but here it cannot: a `None` chain only ever comes
        // from an anonymous scope, which is pruned before it gets this far.
        let inner_chain = extend_scope_chain(chain, &name);
        if let Some(block) = Self::get_block(node) {
            Self::descend_block(
                block,
                source,
                module_path,
                rel_path,
                inner_chain.as_deref(),
                depth + 1,
                out,
            );
        }
    }

    /// A `class` inside a function contributes a **name segment but no node**
    /// — a function-local `Class` node is out of scope for this phase, and D2
    /// is explicit that `parent_scope` is a property precisely because the
    /// enclosing scope is often not a node — and **no nesting level**, because
    /// a class body is a namespace, not a closure. Its methods are ordinary
    /// grammar-named definitions on a fully named chain, so D1 gives each of
    /// them a node and their calls land somewhere.
    fn walk_class_scope(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
    ) {
        let name = Self::get_name(node, source).to_string();
        let inner_chain = extend_scope_chain(chain, &name);
        let Some(block) = Self::get_block(node) else {
            return;
        };
        Self::descend_block(
            block,
            source,
            module_path,
            rel_path,
            inner_chain.as_deref(),
            depth,
            out,
        );
    }

    /// Descend into a *top-level* definition's body. Its own chain is just
    /// its name, and what it declares is at depth 1.
    fn descend_definition(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        out: &mut Vec<FunctionInfo>,
    ) {
        let chain = [Self::get_name(node, source).to_string()];
        if let Some(block) = Self::get_block(node) {
            Self::descend_block(block, source, module_path, rel_path, Some(&chain), 1, out);
        }
    }

    /// Does this subtree contain a definition? Debug-assertion support for the
    /// `ANONYMOUS_SCOPES` prune: the claim that a lambda or a comprehension
    /// cannot hold a `def` is a claim about the grammar, so the parser checks
    /// it on every debug/test build instead of leaving it in a comment.
    fn holds_a_definition(node: Node) -> bool {
        if matches!(node.kind(), "function_definition" | "class_definition") {
            return true;
        }
        let mut cursor = node.walk();
        let found = node
            .named_children(&mut cursor)
            .any(Self::holds_a_definition);
        found
    }

    // ── Top-level parsers ───────────────────────────────────────────

    fn parse_function(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        is_method: bool,
        owner: Option<&str>,
    ) -> FunctionInfo {
        let name = Self::get_name(node, source).to_string();
        let prefix = match owner {
            Some(o) => format!("{}.{}", module_path, o),
            None => module_path.to_string(),
        };
        let qualified_name = format!("{}.{}", prefix, name);
        let block = Self::get_block(node);
        let calls = block
            .map(|b| Self::extract_calls(b, source))
            .unwrap_or_default();
        let function_refs = block
            .map(|b| Self::extract_function_pointer_refs(b, source))
            .unwrap_or_default();
        let parameters = Self::extract_parameters(node, source);
        let param_count = Some(
            parameters
                .iter()
                .filter(|p| p.kind != ParameterKind::Receiver)
                .count() as u32,
        );
        let (branch_count, max_nesting) = match block {
            Some(b) => {
                let (c, n) = compute_complexity(b, BRANCH_KINDS_PYTHON, NESTED_SCOPES);
                (Some(c), Some(n))
            }
            None => (None, None),
        };
        let is_recursive = Some(calls.iter().any(|(n, _)| n == &name));
        let docstring = Self::get_docstring(node, source);
        let procedure_names = extract_procedure_annotations(docstring.as_deref());

        FunctionInfo {
            visibility: Self::get_visibility(&name).to_string(),
            name,
            qualified_name,
            is_async: Self::is_async(node, source),
            is_method,
            signature: Self::get_signature(node, source),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring,
            return_type: Self::get_return_type(node, source),
            calls,
            references: Vec::new(),
            function_refs,
            type_parameters: get_type_parameters(node, source, "type_parameter"),
            decorators: Vec::new(),
            parameters,
            branch_count,
            param_count,
            max_nesting,
            is_recursive,
            procedure_names,
            metadata: Default::default(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_class(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
        decorators: Option<Vec<String>>,
    ) {
        let name = Self::get_name(node, source).to_string();
        let qualified_name = format!("{}.{}", module_path, name);
        let bases = Self::get_bases(node, source);
        let docstring = Self::get_docstring(node, source);

        let is_enum = bases.iter().any(|b| ENUM_BASES.contains(&b.as_str()));
        let is_protocol = bases.iter().any(|b| b == "Protocol");

        if is_enum {
            result.enums.push(EnumInfo {
                visibility: Self::get_visibility(&name).to_string(),
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                file_path: rel_path.to_string(),
                line_number: node.start_position().row as u32 + 1,
                end_line: Some(node.end_position().row as u32 + 1),
                docstring,
                variants: Self::get_enum_variants(node, source),
                variant_details: None,
            });
            return;
        }

        if is_protocol {
            result.interfaces.push(InterfaceInfo {
                visibility: Self::get_visibility(&name).to_string(),
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                kind: "protocol".to_string(),
                file_path: rel_path.to_string(),
                line_number: node.start_position().row as u32 + 1,
                end_line: Some(node.end_position().row as u32 + 1),
                docstring,
                type_parameters: get_type_parameters(node, source, "type_parameter"),
            });
        } else {
            let mut metadata = crate::models::MetadataMap::new();
            let decs = decorators.clone().unwrap_or_default();
            metadata.insert("decorators".to_string(), json!(decs));
            result.classes.push(ClassInfo {
                visibility: Self::get_visibility(&name).to_string(),
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                kind: "class".to_string(),
                file_path: rel_path.to_string(),
                line_number: node.start_position().row as u32 + 1,
                end_line: Some(node.end_position().row as u32 + 1),
                docstring,
                bases: bases.clone(),
                type_parameters: get_type_parameters(node, source, "type_parameter"),
                metadata,
            });
        }

        // Inheritance edges
        for base in bases.iter().filter(|b| b.as_str() != "Protocol") {
            result.type_relationships.push(TypeRelationship {
                source_type: name.clone(),
                target_type: Some(base.clone()),
                relationship: "extends".to_string(),
                methods: Vec::new(),
            });
        }

        // Methods in the class body
        let mut method_rel = TypeRelationship {
            source_type: qualified_name.clone(),
            target_type: None,
            relationship: "inherent".to_string(),
            methods: Vec::new(),
        };

        if let Some(block) = Self::get_block(node) {
            let mut cursor = block.walk();
            for child in block.children(&mut cursor) {
                let (fn_node, fn_decorators): (Option<Node>, Vec<String>) = match child.kind() {
                    "function_definition" => (Some(child), Vec::new()),
                    "decorated_definition" => {
                        if let Some(inner) = Self::get_decorated_inner(child) {
                            match inner.kind() {
                                "function_definition" => {
                                    (Some(inner), Self::get_decorators(child, source))
                                }
                                "class_definition" => {
                                    Self::parse_class(
                                        inner,
                                        source,
                                        &qualified_name,
                                        rel_path,
                                        result,
                                        Some(Self::get_decorators(child, source)),
                                    );
                                    (None, Vec::new())
                                }
                                _ => (None, Vec::new()),
                            }
                        } else {
                            (None, Vec::new())
                        }
                    }
                    "class_definition" => {
                        Self::parse_class(child, source, &qualified_name, rel_path, result, None);
                        (None, Vec::new())
                    }
                    _ => (None, Vec::new()),
                };

                if let Some(fn_node) = fn_node {
                    let mut fn_info = Self::parse_function(
                        fn_node,
                        source,
                        module_path,
                        rel_path,
                        true,
                        Some(&name),
                    );
                    fn_info.decorators = fn_decorators.clone();
                    for (flag, val) in Self::classify_decorators(&fn_decorators) {
                        fn_info.metadata.insert(flag.to_string(), val);
                    }
                    method_rel.methods.push(fn_info.clone());
                    result.functions.push(fn_info);
                    // A method body is a scope like any other: a `def` inside
                    // it is `module.Class.method.helper` at depth 1. The
                    // method itself keeps its untagged top-level identity.
                    let chain = [name.clone(), Self::get_name(fn_node, source).to_string()];
                    if let Some(block) = Self::get_block(fn_node) {
                        Self::descend_block(
                            block,
                            source,
                            module_path,
                            rel_path,
                            Some(&chain),
                            1,
                            &mut result.functions,
                        );
                    }
                }
            }
        }

        if !method_rel.methods.is_empty() {
            result.type_relationships.push(method_rel);
        }

        Self::extract_class_attributes(node, source, &qualified_name, rel_path, result);
    }
}

impl Default for PythonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for PythonParser {
    fn language_name(&self) -> &'static str {
        "python"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["py", "pyi"]
    }

    fn parse_file(&self, filepath: &Path, src_root: &Path) -> ParseResult {
        let Ok(source) = std::fs::read(filepath) else {
            return ParseResult::new();
        };

        let rel_path = filepath
            .strip_prefix(src_root)
            .unwrap_or(filepath)
            .to_string_lossy()
            .replace('\\', "/");
        let module_path = Self::file_to_module_path(filepath, src_root);
        let loc = count_lines(&source);

        let filename = filepath
            .file_name()
            .and_then(|o| o.to_str())
            .unwrap_or("")
            .to_string();
        let stem = filepath
            .file_stem()
            .and_then(|o| o.to_str())
            .unwrap_or("")
            .to_string();

        let is_test = filename.starts_with("test_")
            || stem.ends_with("_test")
            || rel_path.contains("/tests/")
            || rel_path.starts_with("tests/");

        if let Some(reason) = is_generated_or_minified(&source) {
            let mut r = ParseResult::new();
            r.files.push(FileInfo {
                path: rel_path,
                filename,
                loc,
                module_path,
                language: "python".to_string(),
                submodule_declarations: Vec::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                annotations: None,
                is_test,
                skip_reason: Some(reason.to_string()),
            });
            return r;
        }

        let Some(tree) = self.parse_tree(&source) else {
            return ParseResult::new();
        };
        let root = tree.root_node();

        let mut file_info = FileInfo {
            path: rel_path.clone(),
            filename,
            loc,
            module_path: module_path.clone(),
            language: "python".to_string(),
            submodule_declarations: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            annotations: None,
            is_test,
            skip_reason: None,
        };

        let mut result = ParseResult::new();

        let mut root_cursor = root.walk();
        for child in root.children(&mut root_cursor) {
            match child.kind() {
                "function_definition" => {
                    result.functions.push(Self::parse_function(
                        child,
                        &source,
                        &module_path,
                        &rel_path,
                        false,
                        None,
                    ));
                    Self::descend_definition(
                        child,
                        &source,
                        &module_path,
                        &rel_path,
                        &mut result.functions,
                    );
                }
                "decorated_definition" => {
                    if let Some(inner) = Self::get_decorated_inner(child) {
                        match inner.kind() {
                            "function_definition" => {
                                let decorators = Self::get_decorators(child, &source);
                                let mut fn_info = Self::parse_function(
                                    inner,
                                    &source,
                                    &module_path,
                                    &rel_path,
                                    false,
                                    None,
                                );
                                fn_info.decorators = decorators.clone();
                                for (flag, val) in Self::classify_decorators(&decorators) {
                                    fn_info.metadata.insert(flag.to_string(), val);
                                }
                                result.functions.push(fn_info);
                                Self::descend_definition(
                                    inner,
                                    &source,
                                    &module_path,
                                    &rel_path,
                                    &mut result.functions,
                                );
                            }
                            "class_definition" => {
                                let decs = Self::get_decorators(child, &source);
                                Self::parse_class(
                                    inner,
                                    &source,
                                    &module_path,
                                    &rel_path,
                                    &mut result,
                                    Some(decs),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                "class_definition" => {
                    Self::parse_class(child, &source, &module_path, &rel_path, &mut result, None);
                }
                // Imports are collected by the dedicated whole-tree walk
                // below the loop (`collect_imports`), not here: the root loop
                // only sees module-level statements, which is finding 5's bug.
                "expression_statement" => {
                    let mut sub_cursor = child.walk();
                    for sub in child.children(&mut sub_cursor) {
                        if sub.kind() != "assignment" {
                            continue;
                        }
                        let mut first_id: Option<String> = None;
                        let mut scan = sub.walk();
                        for sc in sub.children(&mut scan) {
                            if sc.kind() == "identifier" {
                                first_id = Some(node_text(sc, &source).to_string());
                                break;
                            }
                        }
                        let Some(first_id) = first_id else { continue };

                        if first_id == "__all__" {
                            let mut scan2 = sub.walk();
                            for sc in sub.children(&mut scan2) {
                                if sc.kind() == "list" {
                                    let mut list_cursor = sc.walk();
                                    for item in sc.children(&mut list_cursor) {
                                        if item.kind() == "string" {
                                            let text = node_text(item, &source)
                                                .trim_matches(|c| c == '"' || c == '\'');
                                            file_info.exports.push(text.to_string());
                                        }
                                    }
                                }
                            }
                        } else if constant_re().is_match(&first_id)
                            || is_framework_constant(&first_id)
                        {
                            let mut type_ann: Option<String> = None;
                            let mut default_val: Option<String> = None;
                            let mut scan3 = sub.walk();
                            for sc in sub.children(&mut scan3) {
                                if sc.kind() == "type" {
                                    type_ann = Some(node_text(sc, &source).to_string());
                                }
                            }
                            let mut saw_eq = false;
                            let mut scan4 = sub.walk();
                            for sc in sub.children(&mut scan4) {
                                if !sc.is_named() && node_text(sc, &source) == "=" {
                                    saw_eq = true;
                                } else if saw_eq && sc.is_named() {
                                    let val = node_text(sc, &source);
                                    let take = val
                                        .char_indices()
                                        .nth(100)
                                        .map(|(i, _)| i)
                                        .unwrap_or(val.len());
                                    default_val = Some(val[..take].to_string());
                                    break;
                                }
                            }
                            result.constants.push(ConstantInfo {
                                qualified_name: format!("{}.{}", module_path, first_id),
                                visibility: Self::get_visibility(&first_id).to_string(),
                                name: first_id,
                                kind: "constant".to_string(),
                                type_annotation: type_ann,
                                value_preview: default_val,
                                file_path: rel_path.to_string(),
                                line_number: child.start_position().row as u32 + 1,
                            });
                        }
                    }
                }
                "type_alias_statement" => {
                    let mut alias_name: Option<String> = None;
                    let mut scan = child.walk();
                    for sc in child.children(&mut scan) {
                        if sc.kind() == "identifier" {
                            alias_name = Some(node_text(sc, &source).to_string());
                            break;
                        }
                    }
                    if let Some(alias_name) = alias_name {
                        let mut val_text: Option<String> = None;
                        let mut saw_eq = false;
                        let mut scan2 = child.walk();
                        for sc in child.children(&mut scan2) {
                            if !sc.is_named() && node_text(sc, &source) == "=" {
                                saw_eq = true;
                            } else if saw_eq && sc.is_named() {
                                let val = node_text(sc, &source);
                                let take = val
                                    .char_indices()
                                    .nth(100)
                                    .map(|(i, _)| i)
                                    .unwrap_or(val.len());
                                val_text = Some(val[..take].to_string());
                                break;
                            }
                        }
                        result.constants.push(ConstantInfo {
                            qualified_name: format!("{}.{}", module_path, alias_name),
                            visibility: Self::get_visibility(&alias_name).to_string(),
                            name: alias_name,
                            kind: "type_alias".to_string(),
                            type_annotation: val_text,
                            value_preview: None,
                            file_path: rel_path.to_string(),
                            line_number: child.start_position().row as u32 + 1,
                        });
                    }
                }
                _ => {}
            }
        }

        // D2 tie-break, after every definition in the file is known and in
        // source order (each walk emits a definition before the definitions
        // inside it, so the vector is already ordered by start position).
        break_qualified_name_ties(&mut result.functions);

        // Submodule declarations from __init__ files.
        if matches!(file_info.filename.as_str(), "__init__.py" | "__init__.pyi") {
            if let Some(parent) = filepath.parent() {
                let mut entries: Vec<_> = std::fs::read_dir(parent)
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .collect();
                entries.sort_by_key(|e| e.file_name());
                for entry in entries {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    let path = entry.path();
                    if path.is_dir() {
                        if path.join("__init__.py").exists() {
                            file_info.submodule_declarations.push(name_str.to_string());
                        }
                    } else if path.is_file() {
                        let ext = path.extension().and_then(|o| o.to_str()).unwrap_or("");
                        if (ext == "py" || ext == "pyi")
                            && name_str != "__init__.py"
                            && name_str != "__init__.pyi"
                        {
                            let stem = path
                                .file_stem()
                                .and_then(|o| o.to_str())
                                .unwrap_or("")
                                .to_string();
                            file_info.submodule_declarations.push(stem);
                        }
                    }
                }
            }
        }

        Self::collect_imports(root, &source, &mut file_info.imports);
        file_info.annotations = extract_comment_annotations(root, &source, DEFAULT_COMMENT_TYPES);
        result.files.push(file_info);

        result
    }
}

#[cfg(test)]
mod module_path_tests {
    use super::PythonParser;
    use std::path::Path;

    fn module_of(file: &str, root: &str) -> String {
        PythonParser::file_to_module_path(Path::new(file), Path::new(root))
    }

    #[test]
    fn package_name_not_doubled_when_root_dir_is_the_package() {
        // The standard `<repo>/<pkg>/<pkg>/...` clone layout: the clone dir
        // and the package share a name. The relative path already starts
        // with the package, so the module path must not be doubled.
        assert_eq!(
            module_of("/clone/xarray/xarray/core/dataset.py", "/clone/xarray"),
            "xarray.core.dataset"
        );
        // The package root __init__ collapses to the bare package name, not
        // `xarray.xarray`.
        assert_eq!(
            module_of("/clone/xarray/xarray/__init__.py", "/clone/xarray"),
            "xarray"
        );
        // A nested sub-package likewise stays single-rooted.
        assert_eq!(
            module_of("/clone/xarray/xarray/tutorial.py", "/clone/xarray"),
            "xarray.tutorial"
        );
    }

    #[test]
    fn package_name_still_prepended_when_root_dir_differs() {
        // When the root dir name is genuinely the top package and the
        // relative path does not repeat it, prepend as before. (Mirrors the
        // demo_proj/demo_mod smoke-test expectation.)
        assert_eq!(
            module_of("/tmp/demo_proj/demo_mod.py", "/tmp/demo_proj"),
            "demo_proj.demo_mod"
        );
        assert_eq!(
            module_of("/tmp/demo_proj/sub/mod.py", "/tmp/demo_proj"),
            "demo_proj.sub.mod"
        );
    }
}

/// Import extraction — mcp-servers report 2026-08-14, findings 3–5.
#[cfg(test)]
mod import_extraction_tests {
    use super::*;
    use crate::parsers::LanguageParser;

    /// Parse `src` as `pkg/a.py` with `pkg` as the source root and return the
    /// extracted import strings in document order.
    fn imports_of(src: &str) -> Vec<String> {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("pkg");
        std::fs::create_dir(&root).expect("mkdir");
        let path = root.join("a.py");
        std::fs::write(&path, src).expect("write fixture");
        let result = PythonParser::new().parse_file(&path, &root);
        result.files[0].imports.clone()
    }

    /// Finding 4: every origin a statement names survives extraction —
    /// multi-name `import a, b`, the aliased form's origin (not its alias),
    /// and the full dotted origin of a single-name `from pkg import name`.
    /// A multi-name from-import stays the legacy bare module and a wildcard
    /// is the module itself (see the `parse_imports` doc comment).
    #[test]
    fn absolute_forms_keep_every_origin() {
        let imports = imports_of(concat!(
            "import os, sys\n",
            "import pkg.util as u\n",
            "from pkg import util\n",
            "from pkg.sub import deeper\n",
            "from pkg.deps import audit, emit, notify\n",
            "from x import *\n",
        ));
        assert_eq!(
            imports,
            vec![
                "os",
                "sys",
                "pkg.util",
                "pkg.util",
                "pkg.sub.deeper",
                "pkg.deps",
                "x",
            ]
        );
    }

    /// Finding 3: relative forms come through verbatim, dots intact, with the
    /// imported name appended — the resolver owns the rewrite.
    #[test]
    fn relative_forms_keep_their_dots() {
        let imports = imports_of(concat!(
            "from . import util\n",
            "from .util import helper as h\n",
            "from ..util import helper\n",
            "from .sub.deeper import deep_thing\n",
            "from . import *\n",
        ));
        assert_eq!(
            imports,
            vec![
                ".util",
                ".util.helper",
                "..util.helper",
                ".sub.deeper.deep_thing",
                "."
            ]
        );
    }

    /// Finding 5: imports inside `if TYPE_CHECKING:`, `try/except
    /// ImportError:` and function bodies are all real dependencies and are
    /// all collected — exactly once each.
    #[test]
    fn nested_blocks_and_function_bodies_are_walked() {
        let imports = imports_of(concat!(
            "from typing import TYPE_CHECKING\n",
            "\n",
            "if TYPE_CHECKING:\n",
            "    from .b import b_fn\n",
            "\n",
            "try:\n",
            "    import json\n",
            "except ImportError:\n",
            "    json = None\n",
            "\n",
            "\n",
            "def run():\n",
            "    import functools\n",
            "    return functools\n",
        ));
        assert_eq!(
            imports,
            vec!["typing.TYPE_CHECKING", ".b.b_fn", "json", "functools"]
        );
    }
}

/// The Python nested-scope walk — D1 (clauses 1 and 5), D2, D3, D4.
#[cfg(test)]
mod nested_scope_tests {
    use super::*;
    use crate::parsers::LanguageParser;

    /// Parse `src` as `pkg/a.py` with `pkg` as the source root, so the module
    /// path is the fixed `pkg.a` rather than a tempdir name.
    fn parse_py(src: &str) -> ParseResult {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("pkg");
        std::fs::create_dir(&root).expect("mkdir");
        let path = root.join("a.py");
        std::fs::write(&path, src).expect("write fixture");
        PythonParser::new().parse_file(&path, &root)
    }

    fn qnames(src: &str) -> Vec<String> {
        parse_py(src)
            .functions
            .iter()
            .map(|f| f.qualified_name.clone())
            .collect()
    }

    /// Every emitted Function as `(qualified_name, nesting_depth,
    /// parent_scope)`, in emission order. Both properties are absent — not
    /// zero, not empty — at top level.
    fn scopes(src: &str) -> Vec<(String, Option<u64>, Option<String>)> {
        parse_py(src)
            .functions
            .iter()
            .map(|f| {
                (
                    f.qualified_name.clone(),
                    f.metadata.get("nesting_depth").and_then(|v| v.as_u64()),
                    f.metadata
                        .get("parent_scope")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                )
            })
            .collect()
    }

    /// The call names attributed to one function, by qualified name.
    fn calls_of(src: &str, qualified_name: &str) -> Vec<String> {
        parse_py(src)
            .functions
            .iter()
            .find(|f| f.qualified_name == qualified_name)
            .unwrap_or_else(|| panic!("no function {qualified_name} in {:?}", qnames(src)))
            .calls
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }

    // ── D2: the qualified-name chain ─────────────────────────────────────

    #[test]
    fn a_nested_def_is_qualified_by_its_scope_chain() {
        let src = "\
def outer(n):
    def inner(x):
        return x + n

    return inner(1)
";
        assert_eq!(
            scopes(src),
            vec![
                ("pkg.a.outer".into(), None, None),
                (
                    "pkg.a.outer.inner".into(),
                    Some(1),
                    Some("pkg.a.outer".into())
                ),
            ]
        );
    }

    #[test]
    fn the_chain_extends_to_any_depth() {
        let src = "\
def retrying(attempts):
    def decorate(fn):
        def wrapper(*args):
            return fn(*args)

        return wrapper

    return decorate
";
        assert_eq!(
            scopes(src),
            vec![
                ("pkg.a.retrying".into(), None, None),
                (
                    "pkg.a.retrying.decorate".into(),
                    Some(1),
                    Some("pkg.a.retrying".into())
                ),
                (
                    "pkg.a.retrying.decorate.wrapper".into(),
                    Some(2),
                    Some("pkg.a.retrying.decorate".into())
                ),
            ]
        );
    }

    #[test]
    fn a_method_body_is_a_scope_like_any_other() {
        let src = "\
class Registry:
    def install(self, hook):
        def adapter(value):
            return hook(value)

        return adapter
";
        assert_eq!(
            scopes(src),
            vec![
                ("pkg.a.Registry.install".into(), None, None),
                (
                    "pkg.a.Registry.install.adapter".into(),
                    Some(1),
                    Some("pkg.a.Registry.install".into())
                ),
            ]
        );
    }

    /// A decorated nested def keeps its decorators and its classification
    /// flags, and its identity is the inner `def`'s line, not the `@`'s.
    #[test]
    fn a_decorated_nested_def_keeps_its_decorators() {
        let src = "\
def outer():
    @staticmethod
    def helper():
        return 1

    return helper
";
        let result = parse_py(src);
        let helper = result
            .functions
            .iter()
            .find(|f| f.qualified_name == "pkg.a.outer.helper")
            .expect("nested decorated def");
        assert_eq!(helper.decorators, vec!["staticmethod".to_string()]);
        assert_eq!(helper.metadata.get("is_static"), Some(&json!(true)));
        assert_eq!(helper.line_number, 3);
    }

    // ── D1 clause 5, Python edition ──────────────────────────────────────

    /// Block statements are not scopes in Python — they share the enclosing
    /// function's namespace — so they contribute neither a chain segment nor
    /// a nesting level. Copying TS's model without this would either bury the
    /// def a level deeper or drop it.
    #[test]
    fn block_statements_are_transparent() {
        let src = "\
def outer(flag):
    if flag:
        def a():
            return 1

    for _ in range(3):
        def b():
            return 2

    with open('x') as handle:
        def c():
            return handle
";
        assert_eq!(
            qnames(src),
            vec![
                "pkg.a.outer",
                "pkg.a.outer.a",
                "pkg.a.outer.b",
                "pkg.a.outer.c"
            ]
        );
    }

    /// A `lambda` is one of Python's two unnamed scopes: it names nothing and
    /// gets no node, so the calls in its body belong to the enclosing def.
    #[test]
    fn a_lambda_names_no_scope_and_keeps_its_calls_with_the_enclosing_def() {
        let src = "\
def report(rows):
    def normalize(row):
        return row.strip()

    strip_all = lambda row: normalize(row)
    return strip_all
";
        assert_eq!(qnames(src), vec!["pkg.a.report", "pkg.a.report.normalize"]);
        assert_eq!(calls_of(src, "pkg.a.report"), vec!["normalize".to_string()]);
    }

    /// The grammar claim behind the `ANONYMOUS_SCOPES` prune: an unnamed
    /// Python scope has an expression body, so it cannot hold a definition.
    /// (The parser itself asserts this on every debug build; this pins the
    /// comprehension forms specifically.)
    #[test]
    fn no_unnamed_python_scope_can_hold_a_definition() {
        let src = "\
def outer(ys):
    xs = [(lambda: 1)() for y in ys]
    zs = {y: (lambda: 2) for y in ys}
    ws = ((lambda: 3) for y in ys)
    return xs, zs, ws
";
        assert_eq!(qnames(src), vec!["pkg.a.outer"]);
    }

    /// A function-local class contributes a name segment but no node and no
    /// nesting level; its methods are grammar-named definitions on a fully
    /// named chain, so they do get nodes.
    #[test]
    fn a_function_local_class_names_the_scope_without_becoming_one() {
        let src = "\
def local_class():
    class Inner:
        def run(self):
            def deepest():
                return 1

            return deepest()

    return Inner
";
        assert_eq!(
            scopes(src),
            vec![
                ("pkg.a.local_class".into(), None, None),
                (
                    "pkg.a.local_class.Inner.run".into(),
                    Some(1),
                    Some("pkg.a.local_class.Inner".into())
                ),
                (
                    "pkg.a.local_class.Inner.run.deepest".into(),
                    Some(2),
                    Some("pkg.a.local_class.Inner.run".into())
                ),
            ]
        );
        assert!(
            parse_py(src).classes.is_empty(),
            "a function-local class is not a node in this phase"
        );
    }

    // ── D2: the duplicate tie-break ──────────────────────────────────────

    /// Because blocks are transparent, the conditional-definition idiom puts
    /// two identical qualified names in one scope. The second and subsequent
    /// take a `#{line}` suffix; the first keeps the bare name.
    #[test]
    fn same_named_defs_in_sibling_blocks_are_tie_broken_by_line() {
        let src = "\
def pick(flag):
    if flag:
        def choose():
            return 1

    else:
        def choose():
            return 2

    return choose
";
        assert_eq!(
            qnames(src),
            vec!["pkg.a.pick", "pkg.a.pick.choose", "pkg.a.pick.choose#7"]
        );
    }

    #[test]
    fn a_third_duplicate_also_gets_its_own_line_suffix() {
        let src = "\
def outer():
    def dup():
        return 1

    def dup():
        return 2

    def dup():
        return 3
";
        assert_eq!(
            qnames(src),
            vec![
                "pkg.a.outer",
                "pkg.a.outer.dup",
                "pkg.a.outer.dup#5",
                "pkg.a.outer.dup#8"
            ]
        );
    }

    /// The tie-break is conditional on purpose: a line number in every
    /// qualified name would move every CALLS target whenever an unrelated
    /// line above it changed.
    #[test]
    fn a_unique_nested_qualified_name_is_never_suffixed() {
        let src = "\
def outer():
    def only():
        return 1
";
        assert_eq!(qnames(src), vec!["pkg.a.outer", "pkg.a.outer.only"]);
    }

    /// Top-level identities are the addressable public surface and are never
    /// rewritten — a collision there predates this walk.
    #[test]
    fn a_top_level_collision_is_left_alone() {
        let src = "\
def dup():
    return 1


def dup():
    return 2
";
        assert_eq!(qnames(src), vec!["pkg.a.dup", "pkg.a.dup"]);
    }

    // ── D4: one call site, one Function ──────────────────────────────────

    /// The corroborating defect. `extract_calls` skipped nested definitions
    /// on the theory that they were node-ified elsewhere; nothing node-ified
    /// them, so their calls left the graph entirely.
    #[test]
    fn a_nested_defs_calls_attach_to_it_and_not_to_its_parent() {
        let src = "\
def outer():
    def helper():
        audit(1)
        return 2

    return helper()
";
        assert_eq!(
            calls_of(src, "pkg.a.outer.helper"),
            vec!["audit".to_string()]
        );
        assert_eq!(calls_of(src, "pkg.a.outer"), vec!["helper".to_string()]);
    }

    /// Nothing is counted twice either: every scope the call extractor skips
    /// is now a node of its own. This is the invariant that lets the Python
    /// walk do without TS's explicit node-id skip set — if a future change
    /// node-ifies something outside `NAMED_NESTED_SCOPES`, or stops
    /// node-ifying something inside it, this test is what notices.
    #[test]
    fn every_skipped_scope_is_node_ified() {
        let src = "\
def outer():
    @staticmethod
    def decorated():
        alpha()

    def plain():
        def deeper():
            beta()

        gamma()

    class Local:
        def method(self):
            delta()

    epsilon()
";
        let result = parse_py(src);
        let attributed: Vec<(String, Vec<String>)> = result
            .functions
            .iter()
            .map(|f| {
                (
                    f.qualified_name.clone(),
                    f.calls.iter().map(|(n, _)| n.clone()).collect(),
                )
            })
            .collect();
        assert_eq!(
            attributed,
            vec![
                ("pkg.a.outer".into(), vec!["epsilon".to_string()]),
                ("pkg.a.outer.decorated".into(), vec!["alpha".to_string()]),
                ("pkg.a.outer.plain".into(), vec!["gamma".to_string()]),
                ("pkg.a.outer.plain.deeper".into(), vec!["beta".to_string()]),
                ("pkg.a.outer.Local.method".into(), vec!["delta".to_string()]),
            ]
        );
    }
}
