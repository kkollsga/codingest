//! R language parser.
//!
//! Backed by the `tree-sitter-r` grammar (the r-lib packaging on crates.io,
//! v1.x). The grammar's root node is `program`; assignment is a plain
//! `binary_operator` with `lhs` / `operator` / `rhs` fields, and a function
//! literal — spelled `function(…)` or the 4.1 shorthand `\(…)` — is one
//! `function_definition` node either way, so lambda-assigned names come for
//! free through the same arm.
//!
//! Coverage (deliberately conservative — R definitions are *runtime calls*,
//! so everything call-based is extracted only from string-literal arguments):
//!   - `f <- function(…)`, `f = function(…)`, `f <<- function(…)` at the top
//!     level → FunctionInfo. The rhs may be a `\(…)` lambda.
//!   - S4 `setClass("Name", representation(… slots …))` → ClassInfo
//!     (kind="struct" → `Struct` nodes) + one AttributeInfo per named
//!     string-valued slot; `slots = c(…)` is accepted in the same shape.
//!   - S4 `setGeneric("name", function(…) …)` → top-level FunctionInfo;
//!     `setMethod("name", "Class", function(…) …)` → method FunctionInfo
//!     under `<module>.<Class>.<name>` plus the `inherent` TypeRelationship
//!     that becomes its HAS_METHOD edge.
//!   - `source("path.R")` → FileInfo.imports, string kept VERBATIM: it is a
//!     real file path, resolved by the builder's path-import route
//!     (`registry::uses_path_imports("r")`) against the actual file set —
//!     importing-file-relative first, then project-root-relative — so it can
//!     never invent a target. `source(<anything non-literal>)` extracts
//!     nothing.
//!   - `library(pkg)` / `require(pkg)` (identifier or string argument) →
//!     FileInfo.imports as a bare package name. These are namespace-shaped,
//!     NOT file-anchored: R is deliberately absent from the file-anchored
//!     allowlist in `other_edges::build_file_import_edges`, so a package
//!     name can never manufacture a File→File edge — and because a bare name
//!     resolves only on an EXACT known-module match (the Track-D root-prefix
//!     fallback is Python-only), a name colliding with a root-prefixed local
//!     module produces no File→Module edge either. Both halves are pinned by
//!     `builder::tests::r_library_naming_a_local_module_yields_no_file_edge`
//!     and the `r_basic` corpus bait (`library(tools)` vs `tools.R`).
//!   - Call sites → `FunctionInfo.calls`: bare-identifier callees only.
//!     `pkg::fn(…)` and `obj$method(…)` extract nothing (external /
//!     dynamic-dispatch — same-file resolution could only mis-land them).
//!
//! Stated exclusions (checked against the grammar, not guessed):
//!   - Right assignment of a function (`function(x) x -> f`): the grammar
//!     parses the `->` INSIDE the function body (`body: x -> f`), so there
//!     is no clean `fn_def -> name` shape to extract; excluded.
//!   - `assign("name", function(…))`: a string-keyed runtime effect;
//!     excluded to stay conservative.
//!   - R6/Reference classes (`R6Class(…)`, `setRefClass(…)`): method tables
//!     live inside `list(…)` arguments; excluded.
//!   - `pkg::fn` namespace references do not contribute imports (only
//!     `library`/`require` do).
//!   - Nested `g <- function(…)` inside a function body gets no node of its
//!     own; its calls are attributed to the enclosing top-level function
//!     (mirrors the Go `func_literal` / Rust closure handling).
//!   - `setGeneric("name")` without a function argument is skipped.

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

use super::shared::{
    compute_complexity, extract_comment_annotations, file_to_module_path, node_text,
};
use super::LanguageParser;
use crate::models::{
    AttributeInfo, ClassInfo, FileInfo, FunctionInfo, ParameterInfo, ParameterKind, ParseResult,
    TypeRelationship,
};

pub struct RParser;

thread_local! {
    static TS_PARSER: std::cell::RefCell<Parser> = {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_r::LANGUAGE.into())
            .expect("loading tree-sitter-r grammar");
        std::cell::RefCell::new(p)
    };
}

/// Branch node kinds counted by `compute_complexity`. R has no switch
/// statement node — `switch(…)` is an ordinary call.
const BRANCH_KINDS_R: &[&str] = &[
    "if_statement",
    "for_statement",
    "while_statement",
    "repeat_statement",
];

/// Complexity must not descend into a nested function literal — its branches
/// belong to the nested callable (which gets no node), not the parent's
/// cyclomatic count. The call walk DOES descend (see `extract_calls`).
const NESTED_SCOPES: &[&str] = &["function_definition"];

/// Comment node kinds scanned for TODO/FIXME-style annotations.
const R_COMMENT_TYPES: &[&str] = &["comment"];

/// Assignment operators that bind a name to a function literal. `->` / `->>`
/// are excluded: the grammar attaches them inside the function body (see
/// module doc), so they never present the `lhs ident / rhs function` shape.
const ASSIGN_OPS: &[&str] = &["<-", "=", "<<-"];

impl RParser {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        RParser
    }

    fn parse_tree(&self, source: &[u8]) -> Option<Tree> {
        TS_PARSER.with(|p| p.borrow_mut().parse(source, None))
    }

    /// R visibility is by convention: a leading `.` hides a name from `ls()`.
    fn visibility_from_name(name: &str) -> &'static str {
        if name.starts_with('.') {
            "private"
        } else {
            "public"
        }
    }

    /// Contiguous `#` / roxygen `#'` comment lines directly above `node`.
    fn get_doc_comment(node: Node, source: &[u8]) -> Option<String> {
        let mut doc_lines: Vec<String> = Vec::new();
        // Each accepted line must sit DIRECTLY above the previously accepted
        // one (comments are single-line in R), so a blank line ends the block
        // — checked against the last accepted row, not the anchor's, which
        // would truncate a multi-line block to its final line.
        let mut expected_row = node.start_position().row;
        let mut sibling = node.prev_named_sibling();
        while let Some(s) = sibling {
            if !matches!(s.kind(), "comment") || s.end_position().row + 1 != expected_row {
                break;
            }
            expected_row = s.start_position().row;
            let text = node_text(s, source).trim();
            let Some(rest) = text.strip_prefix('#') else {
                break;
            };
            let rest = rest.strip_prefix('\'').unwrap_or(rest);
            let content = rest.strip_prefix(' ').unwrap_or(rest);
            doc_lines.insert(0, content.to_string());
            sibling = s.prev_named_sibling();
        }
        if doc_lines.is_empty() {
            None
        } else {
            Some(doc_lines.join("\n"))
        }
    }

    /// Structured parameters from a `function_definition`'s `parameters`
    /// list. `...` (the `dots` node) becomes a variadic parameter.
    fn extract_parameters(fn_def: Node, source: &[u8]) -> Vec<ParameterInfo> {
        let mut out = Vec::new();
        let Some(params) = fn_def.child_by_field_name("parameters") else {
            return out;
        };
        let mut cursor = params.walk();
        for param in params.named_children(&mut cursor) {
            if !matches!(param.kind(), "parameter") {
                continue;
            }
            let Some(name_node) = param.child_by_field_name("name") else {
                continue;
            };
            let is_dots = name_node.kind() == "dots";
            out.push(ParameterInfo {
                name: node_text(name_node, source).to_string(),
                type_annotation: None,
                default: param
                    .child_by_field_name("default")
                    .map(|d| node_text(d, source).to_string()),
                kind: if is_dots {
                    ParameterKind::Variadic
                } else {
                    ParameterKind::Positional
                },
            });
        }
        out
    }

    /// Collect call sites inside a function body: bare-identifier callees
    /// only. Descends into nested function literals — they get no node of
    /// their own, so their calls belong to the enclosing named function.
    fn extract_calls(body: Node, source: &[u8]) -> Vec<(String, u32)> {
        fn walk(node: Node, source: &[u8], out: &mut Vec<(String, u32)>) {
            if node.kind() == "call" {
                if let Some(func) = node.child_by_field_name("function") {
                    if func.kind() == "identifier" {
                        out.push((
                            node_text(func, source).to_string(),
                            node.start_position().row as u32 + 1,
                        ));
                    }
                    // `namespace_operator` (pkg::fn) and `extract_operator`
                    // (obj$method) callees are deliberately not extracted.
                }
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                walk(child, source, out);
            }
        }
        let mut out = Vec::new();
        walk(body, source, &mut out);
        out
    }

    /// Build a FunctionInfo from a name + `function_definition` node.
    #[allow(clippy::too_many_arguments)]
    fn build_function(
        name: &str,
        fn_def: Node,
        doc_anchor: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        owner: Option<&str>,
    ) -> FunctionInfo {
        let qualified_name = match owner {
            Some(class) => format!("{module_path}.{class}.{name}"),
            None => format!("{module_path}.{name}"),
        };
        let params_text = fn_def
            .child_by_field_name("parameters")
            .map(|p| node_text(p, source).to_string())
            .unwrap_or_default();
        let signature = format!("{name} <- function{params_text}")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let body = fn_def.child_by_field_name("body");
        let calls = body
            .map(|b| Self::extract_calls(b, source))
            .unwrap_or_default();
        let parameters = Self::extract_parameters(fn_def, source);
        let param_count = Some(parameters.len() as u32);
        let (branch_count, max_nesting) = match body {
            Some(b) => {
                let (c, n) = compute_complexity(b, BRANCH_KINDS_R, NESTED_SCOPES);
                (Some(c), Some(n))
            }
            None => (None, None),
        };
        let is_recursive = Some(calls.iter().any(|(callee, _)| callee == name));
        FunctionInfo {
            visibility: Self::visibility_from_name(name).to_string(),
            is_async: false,
            is_method: owner.is_some(),
            signature,
            file_path: rel_path.to_string(),
            line_number: doc_anchor.start_position().row as u32 + 1,
            end_line: Some(fn_def.end_position().row as u32 + 1),
            docstring: Self::get_doc_comment(doc_anchor, source),
            return_type: None,
            calls,
            references: Vec::new(),
            function_refs: Vec::new(),
            type_parameters: None,
            decorators: Vec::new(),
            parameters,
            branch_count,
            param_count,
            max_nesting,
            is_recursive,
            procedure_names: Vec::new(),
            metadata: Default::default(),
            qualified_name,
            name: name.to_string(),
        }
    }

    /// Positional (unnamed) argument nodes of a call, in order.
    fn positional_args<'a>(call: Node<'a>) -> Vec<Node<'a>> {
        let mut out = Vec::new();
        let Some(args) = call.child_by_field_name("arguments") else {
            return out;
        };
        let mut cursor = args.walk();
        for arg in args.named_children(&mut cursor) {
            if !matches!(arg.kind(), "argument") || arg.child_by_field_name("name").is_some() {
                continue;
            }
            if let Some(value) = arg.child_by_field_name("value") {
                out.push(value);
            }
        }
        out
    }

    /// Named arguments of a call as `(name, value)` pairs.
    fn named_args<'a>(call: Node<'a>, source: &'a [u8]) -> Vec<(&'a str, Node<'a>)> {
        let mut out = Vec::new();
        let Some(args) = call.child_by_field_name("arguments") else {
            return out;
        };
        let mut cursor = args.walk();
        for arg in args.named_children(&mut cursor) {
            if !matches!(arg.kind(), "argument") {
                continue;
            }
            let Some(name) = arg.child_by_field_name("name") else {
                continue;
            };
            if let Some(value) = arg.child_by_field_name("value") {
                out.push((node_text(name, source), value));
            }
        }
        out
    }

    /// The literal content of a `string` node, or `None` for any other shape
    /// — this is the "string-literal first args only" gate for every
    /// call-based extraction below.
    fn string_literal_text(node: Node, source: &[u8]) -> Option<String> {
        if !matches!(node.kind(), "string") {
            return None;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "string_content" {
                return Some(node_text(child, source).to_string());
            }
        }
        // An empty string has no `string_content` child.
        Some(String::new())
    }

    /// One named string slot (`name = "character"`) → AttributeInfo.
    fn slot_attributes(
        rep_call: Node,
        source: &[u8],
        class_qname: &str,
        rel_path: &str,
        out: &mut Vec<AttributeInfo>,
    ) {
        for (slot_name, value) in Self::named_args(rep_call, source) {
            let Some(slot_type) = Self::string_literal_text(value, source) else {
                continue;
            };
            out.push(AttributeInfo {
                qualified_name: format!("{class_qname}.{slot_name}"),
                owner_qualified_name: class_qname.to_string(),
                type_annotation: Some(slot_type),
                visibility: Self::visibility_from_name(slot_name).to_string(),
                file_path: rel_path.to_string(),
                line_number: value.start_position().row as u32 + 1,
                default_value: None,
                name: slot_name.to_string(),
            });
        }
    }

    /// `setClass("Name", representation(…))` → ClassInfo + slot attributes.
    /// String-literal class name only; `contains =` inheritance is excluded
    /// (see module doc).
    fn parse_set_class(
        call: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let positional = Self::positional_args(call);
        let Some(name) = positional
            .first()
            .and_then(|n| Self::string_literal_text(*n, source))
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let qname = format!("{module_path}.{name}");
        result.classes.push(ClassInfo {
            qualified_name: qname.clone(),
            kind: "struct".to_string(),
            visibility: Self::visibility_from_name(&name).to_string(),
            file_path: rel_path.to_string(),
            line_number: call.start_position().row as u32 + 1,
            end_line: Some(call.end_position().row as u32 + 1),
            docstring: Self::get_doc_comment(call, source),
            bases: Vec::new(),
            type_parameters: None,
            metadata: Default::default(),
            name: name.clone(),
        });

        // Slots: a positional `representation(…)` call, or `representation =`
        // / `slots =` named arguments whose value is a call (`c(…)` /
        // `representation(…)`). Named string args inside become attributes.
        for value in positional.iter().skip(1) {
            if value.kind() == "call" {
                let is_representation = value
                    .child_by_field_name("function")
                    .is_some_and(|f| node_text(f, source) == "representation");
                if is_representation {
                    Self::slot_attributes(*value, source, &qname, rel_path, &mut result.attributes);
                }
            }
        }
        for (arg_name, value) in Self::named_args(call, source) {
            if matches!(arg_name, "representation" | "slots") && value.kind() == "call" {
                Self::slot_attributes(value, source, &qname, rel_path, &mut result.attributes);
            }
        }
    }

    /// `setGeneric("name", function(…) …)` → top-level FunctionInfo.
    fn parse_set_generic(
        call: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let positional = Self::positional_args(call);
        let Some(name) = positional
            .first()
            .and_then(|n| Self::string_literal_text(*n, source))
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let Some(fn_def) = positional
            .into_iter()
            .find(|n| n.kind() == "function_definition")
        else {
            return;
        };
        result.functions.push(Self::build_function(
            &name,
            fn_def,
            call,
            source,
            module_path,
            rel_path,
            None,
        ));
    }

    /// `setMethod("name", "Class", function(…) …)` → method FunctionInfo +
    /// the `inherent` TypeRelationship that becomes its HAS_METHOD edge.
    /// Both the generic name and the signature class must be string
    /// literals; a `c("A", "B")` multi-dispatch signature is skipped.
    fn parse_set_method(
        call: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let positional = Self::positional_args(call);
        let Some(name) = positional
            .first()
            .and_then(|n| Self::string_literal_text(*n, source))
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let Some(class) = positional
            .get(1)
            .and_then(|n| Self::string_literal_text(*n, source))
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let Some(fn_def) = positional
            .into_iter()
            .find(|n| n.kind() == "function_definition")
        else {
            return;
        };
        let fn_info = Self::build_function(
            &name,
            fn_def,
            call,
            source,
            module_path,
            rel_path,
            Some(&class),
        );
        result.type_relationships.push(TypeRelationship {
            source_type: format!("{module_path}.{class}"),
            target_type: None,
            relationship: "inherent".to_string(),
            methods: vec![fn_info.clone()],
        });
        result.functions.push(fn_info);
    }

    /// Top-level call dispatch: imports + call-based S4 definitions.
    fn parse_top_level_call(
        call: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
        file_info: &mut FileInfo,
    ) {
        let Some(func) = call.child_by_field_name("function") else {
            return;
        };
        if !matches!(func.kind(), "identifier") {
            return;
        }
        match node_text(func, source) {
            "source" => {
                // A FILE PATH, kept verbatim for the path-import route.
                // String-literal argument only: `source(variable)` and
                // `source(paste0(…))` extract nothing.
                if let Some(path) = Self::positional_args(call)
                    .first()
                    .and_then(|n| Self::string_literal_text(*n, source))
                    .filter(|s| !s.is_empty())
                {
                    file_info.imports.push(path);
                }
            }
            "library" | "require" => {
                // A PACKAGE NAME (namespace-shaped, never file-anchored).
                // `library(pkg)` takes an unquoted identifier by default and
                // a string under `character.only`; both spellings are the
                // same bare name.
                if let Some(value) = Self::positional_args(call).first() {
                    let name = if matches!(value.kind(), "identifier") {
                        Some(node_text(*value, source).to_string())
                    } else {
                        Self::string_literal_text(*value, source)
                    };
                    if let Some(name) = name.filter(|s| !s.is_empty()) {
                        file_info.imports.push(name);
                    }
                }
            }
            "setClass" => Self::parse_set_class(call, source, module_path, rel_path, result),
            "setGeneric" => Self::parse_set_generic(call, source, module_path, rel_path, result),
            "setMethod" => Self::parse_set_method(call, source, module_path, rel_path, result),
            _ => {}
        }
    }
}

impl LanguageParser for RParser {
    fn language_name(&self) -> &'static str {
        "r"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        // `.R` is the dominant convention (R itself, testthat, CRAN checks);
        // `.r` occurs in the wild. Extension matching in the registry walk is
        // exact / case-sensitive, so both spellings are registered.
        &["R", "r"]
    }

    fn parse_file(&self, filepath: &Path, src_root: &Path) -> ParseResult {
        let mut result = ParseResult::new();
        let Ok(source) = std::fs::read(filepath) else {
            return result;
        };
        let rel_path = filepath
            .strip_prefix(src_root)
            .unwrap_or(filepath)
            .to_string_lossy()
            .replace('\\', "/");
        let Some(tree) = self.parse_tree(&source) else {
            return result;
        };
        let root = tree.root_node();
        let module_path = file_to_module_path(filepath, src_root, '.');
        let filename = filepath
            .file_name()
            .and_then(|o| o.to_str())
            .unwrap_or("")
            .to_string();
        // testthat convention: `tests/testthat/test-*.R`. The directory
        // segment check catches `tests/`; the prefix check catches the file
        // naming on its own.
        let is_test = super::shared::is_test_path(&rel_path, &filename, &[])
            || filename.starts_with("test-")
            || filename.starts_with("test_");

        let mut file_info = FileInfo {
            path: rel_path.clone(),
            filename,
            loc: super::shared::count_lines(&source),
            module_path: module_path.clone(),
            language: "r".to_string(),
            submodule_declarations: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            annotations: extract_comment_annotations(root, &source, R_COMMENT_TYPES),
            is_test,
            skip_reason: None,
        };

        // `if`/`matches!` rather than a `match child.kind()` so every kind
        // literal stays inside the shapes `tests/grammar_kinds.rs` extracts —
        // match-arm literals are outside that guard by design.
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if matches!(child.kind(), "binary_operator") {
                let (Some(lhs), Some(op), Some(rhs)) = (
                    child.child_by_field_name("lhs"),
                    child.child_by_field_name("operator"),
                    child.child_by_field_name("rhs"),
                ) else {
                    continue;
                };
                if lhs.kind() == "identifier"
                    && rhs.kind() == "function_definition"
                    && ASSIGN_OPS.contains(&node_text(op, &source))
                {
                    let name = node_text(lhs, &source).to_string();
                    result.functions.push(Self::build_function(
                        &name,
                        rhs,
                        child,
                        &source,
                        &module_path,
                        &rel_path,
                        None,
                    ));
                }
            } else if matches!(child.kind(), "call") {
                Self::parse_top_level_call(
                    child,
                    &source,
                    &module_path,
                    &rel_path,
                    &mut result,
                    &mut file_info,
                );
            }
        }

        result.files.push(file_info);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_snippet(source: &str) -> ParseResult {
        parse_named_snippet("script.R", source)
    }

    fn parse_named_snippet(filename: &str, source: &str) -> ParseResult {
        let tmp = tempfile::Builder::new()
            .prefix("codingest-r-parser-")
            .tempdir()
            .expect("tempdir");
        let root = tmp.path().join("proj");
        let path: PathBuf = root.join(filename);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, source).expect("write snippet");
        RParser::new().parse_file(&path, &root)
    }

    #[test]
    fn both_assignment_shapes_become_functions() {
        let result = parse_snippet(
            "# doubles a value\n\
             double_it <- function(x) {\n  x * 2\n}\n\
             add = function(a, b = 2) a + b\n\
             counter <<- function() 1\n",
        );
        let names: Vec<(&str, &str)> = result
            .functions
            .iter()
            .map(|f| (f.name.as_str(), f.qualified_name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("double_it", "proj.script.double_it"),
                ("add", "proj.script.add"),
                ("counter", "proj.script.counter"),
            ]
        );
        let double_it = &result.functions[0];
        assert_eq!(double_it.docstring.as_deref(), Some("doubles a value"));
        assert_eq!(double_it.param_count, Some(1));
        let add = &result.functions[1];
        assert_eq!(add.parameters.len(), 2);
        assert_eq!(add.parameters[1].default.as_deref(), Some("2"));
    }

    #[test]
    fn backslash_lambda_assignment_becomes_a_function() {
        let result = parse_snippet("lam <- \\(x) x * 2\n");
        assert_eq!(result.functions.len(), 1);
        assert_eq!(result.functions[0].name, "lam");
        assert_eq!(result.functions[0].param_count, Some(1));
    }

    #[test]
    fn source_extracts_string_literals_only_and_verbatim() {
        let result = parse_snippet(
            "source(\"utils.R\")\n\
             source(\"sub/helpers.R\", local = TRUE)\n\
             source(paste0(dir, \"/dyn.R\"))\n\
             source(script_var)\n",
        );
        assert_eq!(
            result.files[0].imports,
            vec!["utils.R".to_string(), "sub/helpers.R".to_string()],
            "non-literal source() arguments must extract nothing"
        );
    }

    #[test]
    fn library_and_require_extract_package_names() {
        let result = parse_snippet(
            "library(stats)\n\
             require(utils)\n\
             library(\"jsonlite\", character.only = TRUE)\n",
        );
        assert_eq!(result.files[0].imports, vec!["stats", "utils", "jsonlite"]);
    }

    #[test]
    fn s4_class_generic_and_method_are_extracted() {
        let result = parse_snippet(
            "setClass(\"Person\", representation(name = \"character\", age = \"numeric\"))\n\
             setGeneric(\"greet\", function(object) standardGeneric(\"greet\"))\n\
             setMethod(\"greet\", \"Person\", function(object) {\n  cat(object@name)\n})\n",
        );
        assert_eq!(result.classes.len(), 1);
        let class = &result.classes[0];
        assert_eq!(class.qualified_name, "proj.script.Person");
        assert_eq!(class.kind, "struct");
        let slots: Vec<(&str, Option<&str>)> = result
            .attributes
            .iter()
            .map(|a| (a.name.as_str(), a.type_annotation.as_deref()))
            .collect();
        assert_eq!(
            slots,
            vec![("name", Some("character")), ("age", Some("numeric"))]
        );
        let fn_names: Vec<&str> = result
            .functions
            .iter()
            .map(|f| f.qualified_name.as_str())
            .collect();
        assert_eq!(
            fn_names,
            vec!["proj.script.greet", "proj.script.Person.greet"]
        );
        assert!(result.functions[1].is_method);
        // The inherent relationship carries the method for HAS_METHOD.
        assert_eq!(result.type_relationships.len(), 1);
        assert_eq!(result.type_relationships[0].relationship, "inherent");
        assert_eq!(
            result.type_relationships[0].methods[0].qualified_name,
            "proj.script.Person.greet"
        );
    }

    #[test]
    fn set_method_with_non_literal_signature_is_skipped() {
        let result = parse_snippet(
            "setMethod(\"show\", c(\"A\", \"B\"), function(object) object)\n\
             setClass(class_name_var, representation(x = \"numeric\"))\n",
        );
        assert!(result.functions.is_empty());
        assert!(result.classes.is_empty());
    }

    #[test]
    fn calls_are_bare_identifiers_only() {
        let result = parse_snippet(
            "driver <- function(x) {\n\
               helper(x)\n\
               stats::rnorm(x)\n\
               obj$method(x)\n\
               inner <- function(y) nested_callee(y)\n\
               inner(x)\n\
             }\n",
        );
        let calls: Vec<&str> = result.functions[0]
            .calls
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        // Namespace and `$` callees excluded; nested-lambda body calls are
        // attributed to the enclosing top-level function.
        assert_eq!(calls, vec!["helper", "nested_callee", "inner"]);
    }

    #[test]
    fn recursion_and_branches_are_measured() {
        let result = parse_snippet(
            "countdown <- function(n) {\n\
               if (n > 0) {\n    countdown(n - 1)\n  } else {\n    0\n  }\n\
             }\n",
        );
        let f = &result.functions[0];
        assert_eq!(f.is_recursive, Some(true));
        assert_eq!(f.branch_count, Some(1));
    }

    #[test]
    fn multi_line_doc_comment_is_kept_whole_and_blank_line_ends_it() {
        let result = parse_snippet(
            "# not part of the doc\n\
             \n\
             # line one\n\
             # line two\n\
             documented <- function() 1\n",
        );
        assert_eq!(
            result.functions[0].docstring.as_deref(),
            Some("line one\nline two")
        );
    }

    #[test]
    fn lowercase_extension_file_parses_with_same_language() {
        let result = parse_named_snippet("legacy.r", "old_fn <- function() 1\n");
        assert_eq!(result.files[0].language, "r");
        assert_eq!(result.files[0].module_path, "proj.legacy");
        assert_eq!(result.functions.len(), 1);
    }
}
