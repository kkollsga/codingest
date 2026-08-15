//! Julia language parser.
//!
//! Backed by the official `tree-sitter/tree-sitter-julia` grammar (0.23).
//! Julia has no per-file namespace: module coordinates come from
//! `module X … end` blocks, and files are stitched together *textually* via
//! `include("relative/path.jl")`. That split drives the import model:
//!
//!   - `include("…")` is a **file path** — recorded verbatim in
//!     `FileInfo.imports` and resolved by the builder's path-import route
//!     (`registry::uses_path_imports("julia")`), which checks candidates
//!     against the real file set and so cannot invent a target.
//!   - `using Foo` / `import Foo.Bar` are **module references** — recorded as
//!     their dotted text so the dependency is visible on the File node, but
//!     they are namespace-shaped: julia is deliberately NOT in the
//!     file-anchored allowlist of `other_edges.rs`'s raw prefix walk, so a
//!     `using` of an external package whose name collides with a project file
//!     can never manufacture a File→File edge.
//!
//! Coverage:
//!   - `function f() … end`, short-form `f(x) = …` (an `assignment` whose LHS
//!     is a call), `function foo end` stubs, typed (`f(x)::T = …`) and
//!     `where`-clause signatures → FunctionInfo. Multiple dispatch (same name,
//!     different signatures) is preserved by the builder's overload pass,
//!     which decorates colliding qualified names with a signature hash.
//!   - Qualified extension methods (`function Base.show(io, x) … end`) keep
//!     the leaf name (`show`), so same-file calls resolve to them.
//!   - `struct` / `mutable struct` → ClassInfo kind="struct" (`Struct` nodes)
//!     with fields (typed, untyped, defaulted) as AttributeInfo;
//!     `abstract type` → ClassInfo kind="abstract_type"; `<:` supertypes →
//!     `extends` TypeRelationships (EXTENDS edges).
//!   - `module X … end` → members qualified as `<file-module>.X.<name>`;
//!     top-level modules are recorded as submodule declarations
//!     (Module HAS_SUBMODULE Module).
//!   - `const NAME = …` → ConstantInfo; `export a, b` → FileInfo.exports.
//!   - Call sites (identifier and `receiver.method` field calls) →
//!     FunctionInfo.calls; cyclomatic branch metrics; docstrings from the
//!     immediately preceding string literal; TODO/FIXME comment annotations.
//!   - One level of macro unwrapping: a definition inside a top-level macro
//!     call (`@inline f(x) = …`, `Base.@kwdef struct …`) is extracted as if
//!     unwrapped.
//!
//! Deliberately out of scope (recorded here so absence reads as a decision,
//! not an accident): `macro` definitions get no nodes; anonymous functions
//! (`x -> …`) and `do`-blocks get no nodes (calls inside them belong to the
//! enclosing function, mirroring the Rust-closure/Go-func-literal rule);
//! nested/local named functions get no nodes and their bodies' calls are
//! excluded from the parent; non-`const` global assignments, `primitive type`,
//! selective-import symbols (`using Foo: bar` records only `Foo`), import
//! aliases (`import JSON as J` records `JSON`), and inner constructors are
//! not extracted; `&&`/`||` do not count toward branch metrics.

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

use super::shared::{
    compute_complexity, count_lines, extract_comment_annotations, extract_procedure_annotations,
    file_to_module_path, is_generated_or_minified, is_test_path, make_qualified, node_text,
    BRANCH_KINDS_JULIA, DEFAULT_COMMENT_TYPES,
};
use super::LanguageParser;
use crate::models::{
    AttributeInfo, ClassInfo, ConstantInfo, FileInfo, FunctionInfo, MetadataMap, ParameterInfo,
    ParameterKind, ParseResult, TypeRelationship,
};

/// Base builtins that would otherwise dominate CALLS noise. Deliberately
/// omits `show`, `print`, `string` and `parse`: extending `Base.show` /
/// `Base.print` is the standard user pattern, and suppressing the bare name
/// would orphan calls to those user-defined methods.
pub const JULIA_NOISE_NAMES: &[&str] = &[
    "include",
    "println",
    "error",
    "throw",
    "length",
    "size",
    "push!",
    "pop!",
    "sqrt",
    "abs",
    "min",
    "max",
    "clamp",
    "typeof",
    "convert",
    "collect",
    "isempty",
    "isnothing",
    "zeros",
    "ones",
    "rand",
    "get",
    "haskey",
    "keys",
    "values",
    "getindex",
    "setindex!",
];

/// Scopes whose interior belongs to a *different* (unextracted) callable —
/// both the call walk and the complexity walk stop at these. Arrow functions
/// and `do`-blocks are intentionally absent from the call list: like Rust
/// closures and Go `func` literals they get no node of their own, so calls
/// inside them belong to the enclosing function.
const CALL_NESTED_SCOPES: &[&str] = &["function_definition", "macro_definition"];

/// Complexity additionally skips anonymous callables — their branches would
/// inflate the enclosing function's metrics.
const COMPLEXITY_NESTED_SCOPES: &[&str] = &[
    "function_definition",
    "macro_definition",
    "arrow_function_expression",
    "do_clause",
];

pub struct JuliaParser;

thread_local! {
    static JULIA_PARSER: std::cell::RefCell<Parser> = {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_julia::LANGUAGE.into())
            .expect("loading tree-sitter-julia grammar");
        std::cell::RefCell::new(p)
    };
}

/// What a signature expression unwraps to.
struct SignatureParts<'a> {
    name: String,
    call: Option<Node<'a>>,
    return_type: Option<String>,
    type_parameters: Option<String>,
}

impl JuliaParser {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JuliaParser
    }

    fn parse_tree(&self, source: &[u8]) -> Option<Tree> {
        JULIA_PARSER.with(|p| p.borrow_mut().parse(source, None))
    }

    /// Julia has no access modifiers; the community convention is a leading
    /// underscore for internal names.
    fn get_visibility(name: &str) -> &'static str {
        if name.starts_with('_') {
            "private"
        } else {
            "public"
        }
    }

    /// The immediately preceding bare string literal is Julia's docstring
    /// mechanism (`"doc" ↵ function f() … end`). Adjacency is required so an
    /// unrelated string constant further up is not claimed.
    fn get_docstring(node: Node, source: &[u8]) -> Option<String> {
        let sib = node.prev_named_sibling()?;
        if sib.kind() != "string_literal" {
            return None;
        }
        if sib.end_position().row + 1 != node.start_position().row {
            return None;
        }
        let text = node_text(sib, source).trim_matches('"').trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    /// Peel `where` clauses and a `::T` return annotation off a signature
    /// expression down to the underlying call (or bare identifier for a
    /// `function foo end` stub).
    fn unwrap_signature<'a>(sig_expr: Node<'a>, source: &'a [u8]) -> Option<SignatureParts<'a>> {
        let mut expr = sig_expr;
        let mut return_type = None;
        let mut type_parameters = None;
        loop {
            match expr.kind() {
                "where_expression" => {
                    if let Some(clause) = expr.named_child(1) {
                        type_parameters = Some(node_text(clause, source).to_string());
                    }
                    expr = expr.named_child(0)?;
                }
                "typed_expression" => {
                    if let Some(ty) = expr.named_child(1) {
                        return_type = Some(node_text(ty, source).to_string());
                    }
                    expr = expr.named_child(0)?;
                }
                _ => break,
            }
        }
        let (name, call) = match expr.kind() {
            "call_expression" => {
                let callee = expr.named_child(0)?;
                let name = match callee.kind() {
                    "identifier" => node_text(callee, source).to_string(),
                    // `function Base.show(io, x)` — keep the leaf name so
                    // same-file calls to `show` resolve to the extension.
                    "field_expression" => {
                        let leaf = Self::last_named_child(callee)?;
                        node_text(leaf, source).to_string()
                    }
                    _ => return None,
                };
                (name, Some(expr))
            }
            // `function foo end` dispatch stub.
            "identifier" => (node_text(expr, source).to_string(), None),
            _ => return None,
        };
        Some(SignatureParts {
            name,
            call,
            return_type,
            type_parameters,
        })
    }

    /// True when an `assignment` is Julia's short-form method definition:
    /// its LHS is a call (optionally `where`/`::T`-wrapped).
    fn is_short_form_function(node: Node) -> bool {
        debug_assert!(node.kind() == "assignment");
        let Some(mut lhs) = node.named_child(0) else {
            return false;
        };
        loop {
            match lhs.kind() {
                "call_expression" => return true,
                "where_expression" | "typed_expression" => {
                    let Some(inner) = lhs.named_child(0) else {
                        return false;
                    };
                    lhs = inner;
                }
                _ => return false,
            }
        }
    }

    fn extract_parameters(call: Node, source: &[u8]) -> Vec<ParameterInfo> {
        let mut out = Vec::new();
        let Some(args) = Self::find_child(call, "argument_list") else {
            return out;
        };
        let mut cursor = args.walk();
        for child in args.named_children(&mut cursor) {
            match child.kind() {
                "identifier" => out.push(ParameterInfo {
                    name: node_text(child, source).to_string(),
                    type_annotation: None,
                    default: None,
                    kind: ParameterKind::Positional,
                }),
                "typed_expression" => {
                    let name = child
                        .named_child(0)
                        .map(|n| node_text(n, source).to_string())
                        .unwrap_or_default();
                    let ty = child
                        .named_child(1)
                        .map(|n| node_text(n, source).to_string());
                    out.push(ParameterInfo {
                        name,
                        type_annotation: ty,
                        default: None,
                        kind: ParameterKind::Positional,
                    });
                }
                // Optional/keyword argument with a default (`b=1`, `c::Int=2`).
                "named_argument" => {
                    let mut name = String::new();
                    let mut ty = None;
                    if let Some(lhs) = child.named_child(0) {
                        match lhs.kind() {
                            "typed_expression" => {
                                name = lhs
                                    .named_child(0)
                                    .map(|n| node_text(n, source).to_string())
                                    .unwrap_or_default();
                                ty = lhs.named_child(1).map(|n| node_text(n, source).to_string());
                            }
                            _ => name = node_text(lhs, source).to_string(),
                        }
                    }
                    let default = Self::last_named_child(child)
                        .filter(|n| {
                            n.kind() != "operator"
                                && n.id() != child.named_child(0).map_or(0, |c| c.id())
                        })
                        .map(|n| node_text(n, source).to_string());
                    out.push(ParameterInfo {
                        name,
                        type_annotation: ty,
                        default,
                        kind: ParameterKind::Positional,
                    });
                }
                // `xs...` varargs (positional or keyword — not distinguished).
                "splat_expression" => {
                    let (name, ty) = match child.named_child(0) {
                        Some(inner) if inner.kind() == "typed_expression" => (
                            inner
                                .named_child(0)
                                .map(|n| node_text(n, source).to_string())
                                .unwrap_or_default(),
                            inner
                                .named_child(1)
                                .map(|n| node_text(n, source).to_string()),
                        ),
                        Some(inner) => (node_text(inner, source).to_string(), None),
                        None => continue,
                    };
                    out.push(ParameterInfo {
                        name,
                        type_annotation: ty,
                        default: None,
                        kind: ParameterKind::Variadic,
                    });
                }
                _ => {}
            }
        }
        out
    }

    fn extract_calls(body: Node, source: &[u8], out: &mut Vec<(String, u32)>) {
        if body.kind() == "call_expression" {
            let line = body.start_position().row as u32 + 1;
            if let Some(callee) = body.child(0) {
                match callee.kind() {
                    "identifier" => out.push((node_text(callee, source).to_string(), line)),
                    "field_expression" => {
                        // `obj.method(x)` / `Mod.func(x)` — keep the go-style
                        // `hint.method` form so the resolver can use the last
                        // qualifier segment as a hint.
                        let count = callee.named_child_count() as u32;
                        if count >= 2 {
                            if let (Some(operand), Some(field)) =
                                (callee.named_child(count - 2), callee.named_child(count - 1))
                            {
                                let op_text = node_text(operand, source);
                                let hint = op_text.rsplit('.').next().unwrap_or(op_text);
                                out.push((format!("{}.{}", hint, node_text(field, source)), line));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            if CALL_NESTED_SCOPES.contains(&child.kind()) {
                continue;
            }
            // A nested short-form definition (`g(x) = …` inside a function)
            // defines a local method; its body's calls are not the parent's.
            if child.kind() == "assignment" && Self::is_short_form_function(child) {
                continue;
            }
            Self::extract_calls(child, source, out);
        }
    }

    /// Sum branch metrics over the statement nodes of a body (Julia
    /// definitions have no body wrapper node).
    fn body_metrics<'a>(nodes: impl Iterator<Item = Node<'a>>) -> (u32, u32) {
        let mut branches = 0;
        let mut nesting = 0;
        for node in nodes {
            if COMPLEXITY_NESTED_SCOPES.contains(&node.kind()) {
                continue;
            }
            let (b, n) = compute_complexity(node, BRANCH_KINDS_JULIA, COMPLEXITY_NESTED_SCOPES);
            branches += b;
            nesting = nesting.max(n);
        }
        (branches, nesting)
    }

    fn find_child<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        let found = node.named_children(&mut cursor).find(|c| c.kind() == kind);
        found
    }

    fn last_named_child(node: Node) -> Option<Node> {
        let count = u32::try_from(node.named_child_count()).ok()?;
        count.checked_sub(1).and_then(|i| node.named_child(i))
    }

    /// Named children strictly after `marker` in source order.
    fn children_after<'a>(node: Node<'a>, marker: Node) -> Vec<Node<'a>> {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|c| c.start_byte() >= marker.end_byte())
            .collect()
    }

    /// Parse a long-form `function_definition` or a short-form definition
    /// `assignment`. Returns `None` for shapes that are not functions
    /// (e.g. a signature that unwraps to nothing callable).
    fn parse_function(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
    ) -> Option<FunctionInfo> {
        let long_form = node.kind() == "function_definition";
        let (parts, signature_text, body_nodes) = if long_form {
            let sig_node = Self::find_child(node, "signature")?;
            let parts = Self::unwrap_signature(sig_node.named_child(0)?, source)?;
            let signature_text = format!("function {}", node_text(sig_node, source));
            (parts, signature_text, Self::children_after(node, sig_node))
        } else {
            let lhs = node.named_child(0)?;
            let parts = Self::unwrap_signature(lhs, source)?;
            // A bare identifier LHS is a plain assignment, not a function.
            parts.call?;
            let operator = Self::find_child(node, "operator")?;
            let signature_text = node_text(lhs, source).to_string();
            (parts, signature_text, Self::children_after(node, operator))
        };

        let name = parts.name;
        let qualified_name = make_qualified(module_path, "", &name, '.');
        let parameters = parts
            .call
            .map(|call| Self::extract_parameters(call, source))
            .unwrap_or_default();
        let param_count = Some(parameters.len() as u32);

        let mut calls: Vec<(String, u32)> = Vec::new();
        for body in &body_nodes {
            if CALL_NESTED_SCOPES.contains(&body.kind()) {
                continue;
            }
            if body.kind() == "assignment" && Self::is_short_form_function(*body) {
                continue;
            }
            Self::extract_calls(*body, source, &mut calls);
        }
        let (branch_count, max_nesting) = Self::body_metrics(body_nodes.into_iter());
        let is_recursive = Some(calls.iter().any(|(n, _)| n == &name));
        let docstring = Self::get_docstring(node, source);
        let procedure_names = extract_procedure_annotations(docstring.as_deref());

        Some(FunctionInfo {
            visibility: Self::get_visibility(&name).to_string(),
            is_async: false,
            is_method: false,
            signature: signature_text,
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring,
            return_type: parts.return_type,
            calls,
            references: Vec::new(),
            function_refs: Vec::new(),
            type_parameters: parts.type_parameters,
            decorators: Vec::new(),
            parameters,
            branch_count: Some(branch_count),
            param_count,
            max_nesting: Some(max_nesting),
            is_recursive,
            procedure_names,
            metadata: MetadataMap::new(),
            qualified_name,
            name,
        })
    }

    /// `type_head` payload → (name, type_parameters, supertype).
    /// Handles `Circle`, `Pair{T}`, `Square <: Shape`, `Pair{T} <: Shape`.
    fn type_head_parts(
        head: Node,
        source: &[u8],
    ) -> Option<(String, Option<String>, Option<String>)> {
        let mut expr = head.named_child(0)?;
        let mut supertype = None;
        if expr.kind() == "binary_expression" {
            let rhs = Self::last_named_child(expr)?;
            let raw = node_text(rhs, source);
            let bare = raw[..raw.find('{').unwrap_or(raw.len())].trim();
            supertype = (!bare.is_empty()).then(|| bare.to_string());
            expr = expr.named_child(0)?;
        }
        let (name, type_parameters) = match expr.kind() {
            "parametrized_type_expression" => {
                let name = node_text(expr.named_child(0)?, source).to_string();
                let params = expr
                    .named_child(1)
                    .map(|c| node_text(c, source).to_string());
                (name, params)
            }
            "identifier" => (node_text(expr, source).to_string(), None),
            _ => return None,
        };
        Some((name, type_parameters, supertype))
    }

    fn parse_struct(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let Some(head) = Self::find_child(node, "type_head") else {
            return;
        };
        let Some((name, type_parameters, supertype)) = Self::type_head_parts(head, source) else {
            return;
        };
        let qname = make_qualified(module_path, "", &name, '.');
        let is_mutable = {
            let mut cursor = node.walk();
            let found = node
                .children(&mut cursor)
                .any(|c| !c.is_named() && c.kind() == "mutable");
            found
        };
        let mut metadata = MetadataMap::new();
        if is_mutable {
            metadata.insert("is_mutable".into(), serde_json::Value::Bool(true));
        }

        // Fields: statement nodes after the head. `x::T`, bare `x`, and
        // `x::T = default` (@kwdef style). An assignment whose LHS is a call
        // is an inner constructor, not a field — skipped, as are nested
        // long-form constructors.
        for member in Self::children_after(node, head) {
            let (field_node, default_value) = match member.kind() {
                "typed_expression" | "identifier" => (member, None),
                "assignment" if !Self::is_short_form_function(member) => {
                    let Some(lhs) = member.named_child(0) else {
                        continue;
                    };
                    let default = Self::last_named_child(member)
                        .filter(|n| n.kind() != "operator" && n.id() != lhs.id())
                        .map(|n| truncate_chars(node_text(n, source), 100));
                    (lhs, default)
                }
                _ => continue,
            };
            let (field_name, type_ann) = match field_node.kind() {
                "typed_expression" => (
                    field_node
                        .named_child(0)
                        .map(|n| node_text(n, source).to_string())
                        .unwrap_or_default(),
                    field_node
                        .named_child(1)
                        .map(|n| node_text(n, source).to_string()),
                ),
                "identifier" => (node_text(field_node, source).to_string(), None),
                _ => continue,
            };
            if field_name.is_empty() {
                continue;
            }
            result.attributes.push(AttributeInfo {
                qualified_name: format!("{}.{}", qname, field_name),
                owner_qualified_name: qname.clone(),
                type_annotation: type_ann,
                visibility: Self::get_visibility(&field_name).to_string(),
                file_path: rel_path.to_string(),
                line_number: member.start_position().row as u32 + 1,
                default_value,
                name: field_name,
            });
        }

        result.classes.push(ClassInfo {
            qualified_name: qname,
            kind: "struct".into(),
            visibility: Self::get_visibility(&name).to_string(),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring: Self::get_docstring(node, source),
            bases: supertype.iter().cloned().collect(),
            type_parameters,
            metadata,
            name: name.clone(),
        });
        if let Some(target) = supertype {
            result.type_relationships.push(TypeRelationship {
                source_type: name,
                target_type: Some(target),
                relationship: "extends".into(),
                methods: Vec::new(),
            });
        }
    }

    fn parse_abstract_type(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let Some(head) = Self::find_child(node, "type_head") else {
            return;
        };
        let Some((name, type_parameters, supertype)) = Self::type_head_parts(head, source) else {
            return;
        };
        result.classes.push(ClassInfo {
            qualified_name: make_qualified(module_path, "", &name, '.'),
            kind: "abstract_type".into(),
            visibility: Self::get_visibility(&name).to_string(),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring: Self::get_docstring(node, source),
            bases: supertype.iter().cloned().collect(),
            type_parameters,
            metadata: MetadataMap::new(),
            name: name.clone(),
        });
        if let Some(target) = supertype {
            result.type_relationships.push(TypeRelationship {
                source_type: name,
                target_type: Some(target),
                relationship: "extends".into(),
                methods: Vec::new(),
            });
        }
    }

    fn parse_const(
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let Some(assignment) = Self::find_child(node, "assignment") else {
            return;
        };
        let Some(lhs) = assignment.named_child(0) else {
            return;
        };
        let (name, type_ann) = match lhs.kind() {
            "identifier" => (node_text(lhs, source).to_string(), None),
            "typed_expression" => (
                lhs.named_child(0)
                    .map(|n| node_text(n, source).to_string())
                    .unwrap_or_default(),
                lhs.named_child(1).map(|n| node_text(n, source).to_string()),
            ),
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        let value_preview = Self::last_named_child(assignment)
            .filter(|n| n.kind() != "operator" && n.id() != lhs.id())
            .map(|n| truncate_chars(node_text(n, source), 100));
        result.constants.push(ConstantInfo {
            qualified_name: make_qualified(module_path, "", &name, '.'),
            kind: "constant".into(),
            type_annotation: type_ann,
            value_preview,
            visibility: Self::get_visibility(&name).to_string(),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            name,
        });
    }

    /// Module references from a `using_statement` / `import_statement`,
    /// recorded as dotted text. Relative forms (`using ..Sibling`) keep their
    /// leading dots; selective imports and aliases record the module only.
    fn collect_import_refs(node: Node, source: &[u8], out: &mut Vec<String>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" | "scoped_identifier" | "import_path" => {
                    out.push(node_text(child, source).to_string());
                }
                "selected_import" | "import_alias" => {
                    if let Some(module) = child.named_child(0) {
                        out.push(node_text(module, source).to_string());
                    }
                }
                _ => {}
            }
        }
    }

    /// `include("relative/path.jl")` at statement level → the file path,
    /// verbatim, for the builder's path-import route.
    fn include_target(node: Node, source: &[u8]) -> Option<String> {
        debug_assert!(node.kind() == "call_expression");
        let callee = node.named_child(0)?;
        if callee.kind() != "identifier" || node_text(callee, source) != "include" {
            return None;
        }
        let args = Self::find_child(node, "argument_list")?;
        let literal = args.named_child(0)?;
        if literal.kind() != "string_literal" {
            return None;
        }
        let path = node_text(literal, source).trim_matches('"').to_string();
        (!path.is_empty()).then_some(path)
    }

    /// Statement-level dispatch, shared by the file top level, `module`
    /// bodies, and unwrapped top-level macro calls. `at_file_top_level` gates
    /// submodule declarations to depth 0 so the HAS_SUBMODULE parent
    /// (`FileInfo.module_path`) stays truthful.
    #[allow(clippy::too_many_arguments)]
    fn parse_items(
        container: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        file_info: &mut FileInfo,
        result: &mut ParseResult,
        at_file_top_level: bool,
    ) {
        let mut cursor = container.walk();
        for child in container.named_children(&mut cursor) {
            match child.kind() {
                "module_definition" => {
                    let Some(name_node) = Self::find_child(child, "identifier") else {
                        continue;
                    };
                    let name = node_text(name_node, source).to_string();
                    if at_file_top_level {
                        file_info.submodule_declarations.push(name.clone());
                    }
                    let inner_path = make_qualified(module_path, "", &name, '.');
                    Self::parse_items(
                        child,
                        source,
                        &inner_path,
                        rel_path,
                        file_info,
                        result,
                        false,
                    );
                }
                "function_definition" => {
                    if let Some(f) = Self::parse_function(child, source, module_path, rel_path) {
                        result.functions.push(f);
                    }
                }
                "assignment" => {
                    if Self::is_short_form_function(child) {
                        if let Some(f) = Self::parse_function(child, source, module_path, rel_path)
                        {
                            result.functions.push(f);
                        }
                    }
                    // Non-const global assignments are deliberately skipped.
                }
                "struct_definition" => {
                    Self::parse_struct(child, source, module_path, rel_path, result);
                }
                "abstract_definition" => {
                    Self::parse_abstract_type(child, source, module_path, rel_path, result);
                }
                "const_statement" => {
                    Self::parse_const(child, source, module_path, rel_path, result);
                }
                "using_statement" | "import_statement" => {
                    Self::collect_import_refs(child, source, &mut file_info.imports);
                }
                "export_statement" => {
                    let mut ec = child.walk();
                    for item in child.named_children(&mut ec) {
                        if item.kind() == "identifier" {
                            file_info.exports.push(node_text(item, source).to_string());
                        }
                    }
                }
                "call_expression" => {
                    if let Some(target) = Self::include_target(child, source) {
                        file_info.imports.push(target);
                    }
                }
                // One level of macro unwrapping: `@inline f(x) = …`,
                // `Base.@kwdef struct …`, `@doc`-less annotated definitions.
                "macrocall_expression" => {
                    if let Some(args) = Self::find_child(child, "macro_argument_list") {
                        Self::parse_items(
                            args,
                            source,
                            module_path,
                            rel_path,
                            file_info,
                            result,
                            at_file_top_level,
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

/// First `limit` chars of an expression, for value previews.
fn truncate_chars(text: &str, limit: usize) -> String {
    let take = text
        .char_indices()
        .nth(limit)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    text[..take].to_string()
}

impl LanguageParser for JuliaParser {
    fn language_name(&self) -> &'static str {
        "julia"
    }
    fn file_extensions(&self) -> &'static [&'static str] {
        &["jl"]
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
        let loc = count_lines(&source);
        let filename = filepath
            .file_name()
            .and_then(|o| o.to_str())
            .unwrap_or("")
            .to_string();
        let module_path = file_to_module_path(filepath, src_root, '.');
        let is_test =
            is_test_path(&rel_path, &filename, &["_test.jl"]) || filename == "runtests.jl";

        let mut file_info = FileInfo {
            path: rel_path.clone(),
            filename,
            loc,
            module_path: module_path.clone(),
            language: "julia".to_string(),
            submodule_declarations: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            annotations: None,
            is_test,
            skip_reason: None,
        };

        let mut result = ParseResult::new();
        if let Some(reason) = is_generated_or_minified(&source) {
            file_info.skip_reason = Some(reason.to_string());
            result.files.push(file_info);
            return result;
        }

        let Some(tree) = self.parse_tree(&source) else {
            result.files.push(file_info);
            return result;
        };
        let root = tree.root_node();
        Self::parse_items(
            root,
            &source,
            &module_path,
            &rel_path,
            &mut file_info,
            &mut result,
            true,
        );

        file_info.annotations = extract_comment_annotations(root, &source, DEFAULT_COMMENT_TYPES);
        result.files.push(file_info);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Parse one source string as `<root>/src/app.jl` under a root named
    /// `pkg`, so module paths land at `pkg.src.app.*`.
    fn parse(source: &str) -> ParseResult {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("pkg");
        fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/app.jl");
        fs::write(&file, source).unwrap();
        JuliaParser::new().parse_files(&[file], &root)
    }

    #[test]
    fn long_and_short_form_functions_are_extracted() {
        let parsed = parse(concat!(
            "function area(c)\n    return c\nend\n",
            "scaled(c, k) = k * area(c)\n",
            "x = 1\n",          // plain assignment: not a function
            "f = y -> y + 1\n", // arrow function binding: not extracted
        ));
        let names: Vec<&str> = parsed.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["area", "scaled"]);
        assert_eq!(parsed.functions[0].qualified_name, "pkg.src.app.area");
        assert_eq!(parsed.functions[0].signature, "function area(c)");
        assert_eq!(parsed.functions[1].qualified_name, "pkg.src.app.scaled");
        assert_eq!(parsed.functions[1].signature, "scaled(c, k)");
        // The short form's RHS is its body: the call to `area` is attributed.
        assert_eq!(parsed.functions[1].calls.len(), 1);
        assert_eq!(parsed.functions[1].calls[0].0, "area");
    }

    /// Multiple dispatch: the parser emits BOTH methods under one qualified
    /// name with distinct signatures — the builder's overload pass relies on
    /// exactly that to decorate the colliding IDs.
    #[test]
    fn multi_dispatch_pair_keeps_one_qname_and_two_signatures() {
        let parsed = parse(concat!(
            "function area(c::Circle)\n    return 1\nend\n",
            "function area(c::Circle, scale::Float64)\n    return scale\nend\n",
        ));
        assert_eq!(parsed.functions.len(), 2);
        assert_eq!(
            parsed.functions[0].qualified_name,
            parsed.functions[1].qualified_name
        );
        assert_ne!(parsed.functions[0].signature, parsed.functions[1].signature);
        assert_eq!(parsed.functions[0].param_count, Some(1));
        assert_eq!(parsed.functions[1].param_count, Some(2));
        assert_eq!(
            parsed.functions[1].parameters[1].type_annotation.as_deref(),
            Some("Float64")
        );
    }

    #[test]
    fn module_members_are_qualified_and_only_top_level_modules_declared() {
        let parsed = parse(concat!(
            "module Outer\n",
            "helper(x) = x\n",
            "module Inner\n",
            "deep(y) = y\n",
            "end\n",
            "end\n",
        ));
        let qnames: Vec<&str> = parsed
            .functions
            .iter()
            .map(|f| f.qualified_name.as_str())
            .collect();
        assert_eq!(
            qnames,
            ["pkg.src.app.Outer.helper", "pkg.src.app.Outer.Inner.deep"]
        );
        // Only the FILE-top-level module is declared: HAS_SUBMODULE's parent
        // is the file's module path, so a deeper chain would mis-parent.
        assert_eq!(parsed.files[0].submodule_declarations, ["Outer"]);
    }

    #[test]
    fn structs_carry_fields_mutability_supertype_and_type_params() {
        let parsed = parse(concat!(
            "abstract type Shape end\n",
            "struct Pair{T} <: Shape\n    a::T\n    b\nend\n",
            "mutable struct Counter\n    count::Int\nend\n",
        ));
        assert_eq!(parsed.classes.len(), 3);
        let shape = &parsed.classes[0];
        assert_eq!(shape.kind, "abstract_type");
        assert_eq!(shape.qualified_name, "pkg.src.app.Shape");
        let pair = &parsed.classes[1];
        assert_eq!(pair.kind, "struct");
        assert_eq!(pair.type_parameters.as_deref(), Some("{T}"));
        assert_eq!(pair.bases, ["Shape"]);
        assert!(!pair.metadata.contains_key("is_mutable"));
        let counter = &parsed.classes[2];
        assert_eq!(counter.metadata["is_mutable"], serde_json::json!(true));

        let fields: Vec<(&str, Option<&str>)> = parsed
            .attributes
            .iter()
            .map(|a| (a.name.as_str(), a.type_annotation.as_deref()))
            .collect();
        assert_eq!(
            fields,
            [("a", Some("T")), ("b", None), ("count", Some("Int"))]
        );
        let rels: Vec<(&str, Option<&str>)> = parsed
            .type_relationships
            .iter()
            .map(|r| (r.source_type.as_str(), r.target_type.as_deref()))
            .collect();
        assert_eq!(rels, [("Pair", Some("Shape"))]);
    }

    /// The import split that the whole Julia model hangs on: `include` paths
    /// are recorded verbatim as FILE PATHS; `using`/`import` are recorded as
    /// dotted MODULE references (relative forms keep their leading dots), and
    /// selective/aliased imports record the module only.
    #[test]
    fn includes_and_module_references_are_recorded_distinctly() {
        let parsed = parse(concat!(
            "include(\"shapes/circle.jl\")\n",
            "using Downloads\n",
            "using .Geometry\n",
            "using JSON: parse\n",
            "import Base.show\n",
            "import TOML as T\n",
            "export area, Circle\n",
        ));
        assert_eq!(
            parsed.files[0].imports,
            [
                "shapes/circle.jl",
                "Downloads",
                ".Geometry",
                "JSON",
                "Base.show",
                "TOML",
            ]
        );
        assert_eq!(parsed.files[0].exports, ["area", "Circle"]);
    }

    #[test]
    fn dispatch_stub_and_qualified_extension_take_the_leaf_name() {
        let parsed = parse(concat!(
            "function foo end\n",
            "Base.show(io, c) = 1\n",
            "function Base.print(io, c)\n    return 2\nend\n",
        ));
        let names: Vec<&str> = parsed.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["foo", "show", "print"]);
        assert_eq!(parsed.functions[0].param_count, Some(0));
        assert_eq!(parsed.functions[1].qualified_name, "pkg.src.app.show");
    }

    /// Calls inside a nested named function (long or short form) belong to
    /// the nested definition, which gets no node — never to the parent.
    /// Calls inside an anonymous arrow function DO belong to the parent
    /// (the Rust-closure/Go-func-literal rule).
    #[test]
    fn nested_named_scopes_are_excluded_from_parent_calls() {
        let parsed = parse(concat!(
            "function outer(xs)\n",
            "    function inner(a)\n        hidden(a)\n    end\n",
            "    local_short(b) = also_hidden(b)\n",
            "    mapped = map(x -> visible(x), xs)\n",
            "    return inner(mapped)\n",
            "end\n",
        ));
        assert_eq!(parsed.functions.len(), 1);
        let calls: Vec<&str> = parsed.functions[0]
            .calls
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(calls, ["map", "visible", "inner"]);
    }

    #[test]
    fn const_recursion_docstring_and_macro_unwrap() {
        let parsed = parse(concat!(
            "const MAX = 100\n",
            "\"\"\"\nCount down to zero.\n\"\"\"\n",
            "function countdown(n)\n",
            "    n <= 0 && return 0\n",
            "    return countdown(n - 1)\n",
            "end\n",
            "@inline shortcut(x) = 2 * x\n",
        ));
        assert_eq!(parsed.constants.len(), 1);
        assert_eq!(parsed.constants[0].qualified_name, "pkg.src.app.MAX");
        assert_eq!(parsed.constants[0].value_preview.as_deref(), Some("100"));
        let countdown = &parsed.functions[0];
        assert_eq!(countdown.name, "countdown");
        assert_eq!(countdown.docstring.as_deref(), Some("Count down to zero."));
        assert_eq!(countdown.is_recursive, Some(true));
        // One level of macro unwrapping extracts the wrapped definition.
        assert_eq!(parsed.functions[1].name, "shortcut");
    }
}
