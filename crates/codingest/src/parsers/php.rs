//! PHP language parser.
//!
//! Coverage in 0.9.36:
//!   - `class` / `interface` / `trait` declarations → ClassInfo /
//!     InterfaceInfo (trait → ClassInfo kind="trait", matching the
//!     Rust-trait encoding).
//!   - Top-level `function` definitions and class `method` declarations
//!     → FunctionInfo.
//!   - `const` declarations (top-level + class-level) → ConstantInfo
//!     per const_element.
//!   - `namespace` declaration → FileInfo.module_path (backslash
//!     separator).
//!   - `use` declarations → FileInfo.imports.
//!   - PHP 8 attributes (`#[Route('/x')]`) → FunctionInfo.decorators
//!     so the existing 0.9.34 DECORATES pass picks them up.
//!   - Visibility modifiers (`public` / `protected` / `private`).
//!   - `static`, `final`, `abstract` modifiers as metadata.
//!
//! Not yet supported (follow-up scope):
//!   - `define('NAME', value)` constants — these are function calls,
//!     not declaration nodes, and need a separate post-pass that
//!     walks the call graph.
//!   - PHP fibers (`Fiber::start`) async detection. v1 marks every
//!     PHP function `is_async=false`.
//!   - Property declarations as AttributeInfo. The grammar exposes
//!     them but we don't currently model PHP class properties; the
//!     same applies to constructor property promotion.

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

use super::shared::{file_to_module_path, make_qualified, node_text};
use super::LanguageParser;
use crate::models::{
    ClassInfo, ConstantInfo, FileInfo, FunctionInfo, InterfaceInfo, ParseResult, TypeRelationship,
};

/// PHP standard-library / language built-in names excluded from CALLS
/// resolution. The list is small on purpose — the call resolver's
/// 5-tier name lookup already disambiguates user-defined identifiers
/// well; we only need to swallow the truly ubiquitous names that would
/// otherwise generate edges to every same-name user function.
pub const PHP_NOISE_NAMES: &[&str] = &[
    "array",
    "count",
    "strlen",
    "isset",
    "empty",
    "unset",
    "print",
    "echo",
    "var_dump",
    "print_r",
    "gettype",
    "is_array",
    "is_string",
    "is_int",
    "is_bool",
    "is_null",
    "is_object",
    "is_callable",
    "trim",
    "explode",
    "implode",
    "str_replace",
    "preg_match",
    "json_encode",
    "json_decode",
    "in_array",
    "array_keys",
    "array_values",
    "array_map",
    "array_filter",
    "array_merge",
    "sprintf",
    "printf",
    "fopen",
    "fclose",
    "fread",
    "fwrite",
    "file_get_contents",
    "file_put_contents",
];

pub struct PhpParser;

thread_local! {
    static TS_PARSER: std::cell::RefCell<Parser> = {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .expect("loading tree-sitter-php grammar");
        std::cell::RefCell::new(p)
    };
}

impl PhpParser {
    // Keep the established constructor-only parser API stable in this hardening pass.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        PhpParser
    }

    fn parse_tree(&self, source: &[u8]) -> Option<Tree> {
        TS_PARSER.with(|p| p.borrow_mut().parse(source, None))
    }

    /// Walk a program/namespace body and dispatch declarations.
    fn parse_block(
        block: Node,
        source: &[u8],
        module_path: &str,
        owner_prefix: &str,
        rel_path: &str,
        result: &mut ParseResult,
        file_info: &mut FileInfo,
    ) {
        let mut cursor = block.walk();
        for child in block.named_children(&mut cursor) {
            match child.kind() {
                "namespace_definition" => {
                    // Nested namespace block — recurse with the
                    // namespace-qualified module path.
                    let ns_name = child
                        .child_by_field_name("name")
                        .map(|n| node_text(n, source).to_string())
                        .unwrap_or_default();
                    let nested_module = if ns_name.is_empty() {
                        module_path.to_string()
                    } else if module_path.is_empty() {
                        ns_name
                    } else {
                        format!("{module_path}\\{ns_name}")
                    };
                    if let Some(body) = child.child_by_field_name("body") {
                        Self::parse_block(
                            body,
                            source,
                            &nested_module,
                            owner_prefix,
                            rel_path,
                            result,
                            file_info,
                        );
                    } else {
                        // `namespace Foo;` form (no body) — the rest of
                        // the file lives under this namespace. The
                        // top-level parse_file sets file_info.module_path
                        // when it sees this; nothing more to do here.
                    }
                }
                "namespace_use_declaration" => {
                    Self::extract_use_imports(child, source, file_info);
                }
                "class_declaration" => {
                    Self::parse_class(
                        child,
                        source,
                        module_path,
                        owner_prefix,
                        rel_path,
                        result,
                        "class",
                    );
                }
                "interface_declaration" => {
                    Self::parse_interface(
                        child,
                        source,
                        module_path,
                        owner_prefix,
                        rel_path,
                        result,
                    );
                }
                "trait_declaration" => {
                    Self::parse_class(
                        child,
                        source,
                        module_path,
                        owner_prefix,
                        rel_path,
                        result,
                        "trait",
                    );
                }
                "function_definition" => {
                    // `is_method` means "is a method": true only when this
                    // definition sits inside an owning type. A top-level
                    // `function` has an empty owner prefix, so the flag is the
                    // *negation* of `is_empty()` — passing `is_empty()` marked
                    // every top-level PHP function as a method.
                    Self::parse_function(
                        child,
                        source,
                        module_path,
                        owner_prefix,
                        rel_path,
                        result,
                        !owner_prefix.is_empty(),
                    );
                }
                "const_declaration" => {
                    Self::parse_const(child, source, module_path, owner_prefix, rel_path, result);
                }
                _ => {}
            }
        }
    }

    /// Extract `use Foo\Bar;` / `use Foo\Bar as Baz;` / `use Foo\{Bar, Baz};`
    /// declarations into `file_info.imports`, one entry per imported symbol.
    ///
    /// tree-sitter-php gives the two shapes different trees:
    ///   * plain — `namespace_use_declaration` holds one or more
    ///     `namespace_use_clause` children, each carrying the full path.
    ///   * grouped — `namespace_use_declaration` holds a `namespace_name`
    ///     *prefix* plus a `namespace_use_group` in field `body`, whose
    ///     `namespace_use_clause` children carry paths **relative** to that
    ///     prefix. Each member is recorded as `prefix\member`; a member may
    ///     itself be qualified (`use App\Domain\{Billing\Invoice};`).
    ///
    /// The group body used to be unmatched, so a grouped `use` collapsed to a
    /// single import naming the bare prefix and every member was lost.
    ///
    /// An `as` alias is ignored in both shapes — the import records the path
    /// that was imported, not the local name it was bound to.
    fn extract_use_imports(node: Node, source: &[u8], file_info: &mut FileInfo) {
        if let Some(group) = node.child_by_field_name("body") {
            let mut pc = node.walk();
            let prefix = node
                .named_children(&mut pc)
                .find(|c| c.kind() == "namespace_name")
                .map(|c| node_text(c, source).trim_end_matches('\\').to_string())
                .unwrap_or_default();
            let mut gc = group.walk();
            for clause in group.named_children(&mut gc) {
                if clause.kind() != "namespace_use_clause" {
                    continue;
                }
                let Some(member) = Self::use_clause_path(clause, source) else {
                    continue;
                };
                if prefix.is_empty() {
                    file_info.imports.push(member.to_string());
                } else {
                    file_info.imports.push(format!("{prefix}\\{member}"));
                }
            }
            return;
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "namespace_name" | "qualified_name" | "name" => {
                    let text = node_text(child, source).to_string();
                    if !text.is_empty() {
                        file_info.imports.push(text);
                    }
                }
                "namespace_use_clause" => {
                    if let Some(text) = Self::use_clause_path(child, source) {
                        file_info.imports.push(text.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    /// The imported path of a `namespace_use_clause`, ignoring any `as` alias.
    ///
    /// The alias is a `name` node in field `alias`, indistinguishable by kind
    /// from a bare unqualified path, so it is excluded by identity rather than
    /// by position.
    fn use_clause_path<'a>(clause: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
        let alias = clause.child_by_field_name("alias");
        let mut cursor = clause.walk();
        let path = clause
            .named_children(&mut cursor)
            .filter(|c| alias.is_none_or(|a| a.id() != c.id()))
            .find(|c| matches!(c.kind(), "namespace_name" | "qualified_name" | "name"))?;
        let text = node_text(path, source);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Extract `#[Attr(args)]` PHP-8 attributes attached to a
    /// declaration. Returns one decorator string per attribute, in
    /// source order. The string includes the parenthesised args when
    /// present so the DECORATES resolver and the (eventual) PHP route
    /// detector can both consume the same shape.
    fn extract_attributes(node: Node, source: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let attrs = match node.child_by_field_name("attributes") {
            Some(n) => n,
            None => return out,
        };
        let mut cursor = attrs.walk();
        for group in attrs.named_children(&mut cursor) {
            if group.kind() != "attribute_group" {
                continue;
            }
            let mut sub = group.walk();
            for attr in group.named_children(&mut sub) {
                if attr.kind() != "attribute" {
                    continue;
                }
                // Attribute name is the first named child (a `name` /
                // `qualified_name` / `relative_name`). Args are in the
                // `parameters` field.
                let mut name_cursor = attr.walk();
                let mut head: Option<String> = None;
                for c in attr.named_children(&mut name_cursor) {
                    if matches!(c.kind(), "name" | "qualified_name" | "relative_name") {
                        head = Some(node_text(c, source).to_string());
                        break;
                    }
                }
                let Some(head) = head else { continue };
                let mut raw = head;
                if let Some(params) = attr.child_by_field_name("parameters") {
                    raw.push_str(node_text(params, source));
                }
                out.push(raw);
            }
        }
        out
    }

    /// Visibility from any of `visibility_modifier` / `abstract_modifier`
    /// / `static_modifier` / etc. children. PHP's grammar exposes these
    /// as direct children of the declaration, not in a `modifiers`
    /// wrapper.
    fn extract_visibility(node: Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                return node_text(child, source).to_string();
            }
        }
        "public".to_string()
    }

    fn has_modifier(node: Node, kind: &str) -> bool {
        let mut cursor = node.walk();
        let mut found = false;
        for child in node.named_children(&mut cursor) {
            if child.kind() == kind {
                found = true;
                break;
            }
        }
        found
    }

    fn extract_name<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
        node.child_by_field_name("name")
            .map(|c| node_text(c, source))
    }

    fn extract_body(node: Node) -> Option<Node> {
        node.child_by_field_name("body")
    }

    fn parse_class(
        node: Node,
        source: &[u8],
        module_path: &str,
        owner_prefix: &str,
        rel_path: &str,
        result: &mut ParseResult,
        kind: &str,
    ) {
        let Some(name) = Self::extract_name(node, source) else {
            return;
        };
        let visibility = Self::extract_visibility(node, source);
        let line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let qname = make_qualified(module_path, owner_prefix, name, '\\');

        result.classes.push(ClassInfo {
            qualified_name: qname.clone(),
            visibility: visibility.clone(),
            name: name.to_string(),
            kind: kind.to_string(),
            file_path: rel_path.to_string(),
            line_number: line,
            docstring: None,
            bases: Vec::new(),
            type_parameters: None,
            end_line: Some(end_line),
            metadata: Default::default(),
        });

        // Extract `extends` (`base_clause`) and `implements`
        // (`class_interface_clause`) and emit TypeRelationships.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "base_clause" => {
                    let mut sub = child.walk();
                    for c in child.named_children(&mut sub) {
                        if matches!(c.kind(), "name" | "qualified_name" | "relative_name") {
                            let parent = node_text(c, source).to_string();
                            result.type_relationships.push(TypeRelationship {
                                source_type: qname.clone(),
                                target_type: Some(parent),
                                relationship: "extends".to_string(),
                                methods: Vec::new(),
                            });
                        }
                    }
                }
                "class_interface_clause" => {
                    let mut sub = child.walk();
                    for c in child.named_children(&mut sub) {
                        if matches!(c.kind(), "name" | "qualified_name" | "relative_name") {
                            let iface = node_text(c, source).to_string();
                            result.type_relationships.push(TypeRelationship {
                                source_type: qname.clone(),
                                target_type: Some(iface),
                                relationship: "implements".to_string(),
                                methods: Vec::new(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(body) = Self::extract_body(node) {
            let nested_prefix = qname.clone();
            let mut method_rel = TypeRelationship {
                source_type: qname.clone(),
                target_type: None,
                relationship: "inherent".to_string(),
                methods: Vec::new(),
            };
            let methods_start = result.functions.len();

            let mut body_cursor = body.walk();
            for child in body.named_children(&mut body_cursor) {
                match child.kind() {
                    "method_declaration" => {
                        Self::parse_function(
                            child,
                            source,
                            module_path,
                            &nested_prefix,
                            rel_path,
                            result,
                            true,
                        );
                    }
                    "const_declaration" => {
                        Self::parse_const(
                            child,
                            source,
                            module_path,
                            &nested_prefix,
                            rel_path,
                            result,
                        );
                    }
                    _ => {}
                }
            }

            // HAS_METHOD edges are computed by builder/type_edges.rs
            // from `inherent` TypeRelationships' `methods` Vec. Collect
            // the methods we just appended that belong directly to this
            // class (one separator past the nested_prefix).
            let direct_prefix = format!("{nested_prefix}\\");
            for fn_info in &result.functions[methods_start..] {
                if let Some(rest) = fn_info.qualified_name.strip_prefix(&direct_prefix) {
                    if !rest.contains('\\') {
                        method_rel.methods.push(fn_info.clone());
                    }
                }
            }
            if !method_rel.methods.is_empty() {
                result.type_relationships.push(method_rel);
            }
        }
    }

    fn parse_interface(
        node: Node,
        source: &[u8],
        module_path: &str,
        owner_prefix: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let Some(name) = Self::extract_name(node, source) else {
            return;
        };
        let visibility = Self::extract_visibility(node, source);
        let line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let qname = make_qualified(module_path, owner_prefix, name, '\\');

        result.interfaces.push(InterfaceInfo {
            qualified_name: qname.clone(),
            visibility,
            name: name.to_string(),
            kind: "interface".to_string(),
            file_path: rel_path.to_string(),
            line_number: line,
            docstring: None,
            type_parameters: None,
            end_line: Some(end_line),
        });

        if let Some(body) = Self::extract_body(node) {
            let nested_prefix = qname;
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                if child.kind() == "method_declaration" {
                    Self::parse_function(
                        child,
                        source,
                        module_path,
                        &nested_prefix,
                        rel_path,
                        result,
                        true,
                    );
                }
            }
        }
    }

    fn parse_function(
        node: Node,
        source: &[u8],
        _module_path: &str,
        owner_prefix: &str,
        rel_path: &str,
        result: &mut ParseResult,
        is_method: bool,
    ) {
        let Some(name) = Self::extract_name(node, source) else {
            return;
        };
        let visibility = Self::extract_visibility(node, source);
        let is_static = Self::has_modifier(node, "static_modifier");
        let line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let qname = if owner_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{owner_prefix}\\{name}")
        };
        let return_type = node
            .child_by_field_name("return_type")
            .map(|c| node_text(c, source).to_string());
        let signature = Self::build_signature(node, source);
        let calls = Self::extract_calls(node, source);
        let decorators = Self::extract_attributes(node, source);
        let mut metadata: crate::models::MetadataMap = Default::default();
        if is_static {
            metadata.insert("is_static".to_string(), serde_json::json!(true));
        }

        result.functions.push(FunctionInfo {
            qualified_name: qname,
            visibility,
            is_async: false,
            is_method,
            signature,
            file_path: rel_path.to_string(),
            line_number: line,
            name: name.to_string(),
            docstring: None,
            return_type,
            decorators,
            calls,
            references: Vec::new(),
            function_refs: Vec::new(),
            type_parameters: None,
            end_line: Some(end_line),
            parameters: Vec::new(),
            branch_count: None,
            param_count: None,
            max_nesting: None,
            is_recursive: None,
            procedure_names: Vec::new(),
            metadata,
        });
    }

    fn parse_const(
        node: Node,
        source: &[u8],
        module_path: &str,
        owner_prefix: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let visibility = Self::extract_visibility(node, source);
        let type_annotation = node
            .child_by_field_name("type")
            .map(|c| node_text(c, source).to_string());
        let line = node.start_position().row as u32 + 1;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "const_element" {
                continue;
            }
            // const_element has no fields — walk children for the
            // `name` node (the identifier) and `expression` (the
            // value). value_preview is the raw const_element source
            // slice capped at 100 chars.
            let mut name_cursor = child.walk();
            let mut const_name: Option<String> = None;
            for c in child.named_children(&mut name_cursor) {
                if c.kind() == "name" {
                    const_name = Some(node_text(c, source).to_string());
                    break;
                }
            }
            let Some(const_name) = const_name else {
                continue;
            };
            let qname = if owner_prefix.is_empty() {
                if module_path.is_empty() {
                    const_name.clone()
                } else {
                    format!("{module_path}\\{const_name}")
                }
            } else {
                format!("{owner_prefix}::{const_name}")
            };
            let value_preview = {
                let raw = node_text(child, source);
                let take = raw
                    .char_indices()
                    .nth(100)
                    .map(|(i, _)| i)
                    .unwrap_or(raw.len());
                Some(raw[..take].to_string())
            };
            result.constants.push(ConstantInfo {
                qualified_name: qname,
                visibility: visibility.clone(),
                name: const_name,
                kind: "constant".to_string(),
                type_annotation: type_annotation.clone(),
                value_preview,
                file_path: rel_path.to_string(),
                line_number: line,
            });
        }
    }

    fn build_signature(node: Node, source: &[u8]) -> String {
        // Take everything before the function body (`compound_statement`
        // for methods, `compound_statement` or `;` for interface
        // methods).
        let mut parts: Vec<&str> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "compound_statement" {
                break;
            }
            parts.push(node_text(child, source));
        }
        parts.join(" ")
    }

    fn extract_calls(node: Node, source: &[u8]) -> Vec<(String, u32)> {
        let mut calls: Vec<(String, u32)> = Vec::new();
        fn walk(n: Node, source: &[u8], out: &mut Vec<(String, u32)>) {
            // PHP function/method call nodes.
            let kind = n.kind();
            if matches!(
                kind,
                "function_call_expression"
                    | "member_call_expression"
                    | "scoped_call_expression"
                    | "nullsafe_member_call_expression"
            ) {
                let line = n.start_position().row as u32 + 1;
                // For free-function calls, the first child is the
                // function name. For member/scoped calls the `name`
                // field gives the method.
                let callee = if let Some(name) = n.child_by_field_name("name") {
                    Some(node_text(name, source).to_string())
                } else if let Some(func) = n.child_by_field_name("function") {
                    Some(node_text(func, source).to_string())
                } else {
                    n.named_child(0).map(|c| node_text(c, source).to_string())
                };
                if let Some(callee) = callee {
                    let bare = callee.rsplit("\\").next().unwrap_or(&callee).trim();
                    let bare = bare.rsplit("::").next().unwrap_or(bare);
                    if !bare.is_empty() && !bare.contains(' ') && !bare.contains('(') {
                        out.push((bare.to_string(), line));
                    }
                }
            }
            let mut cursor = n.walk();
            for child in n.named_children(&mut cursor) {
                walk(child, source, out);
            }
        }
        walk(node, source, &mut calls);
        calls
    }
}

/// Find the first top-level `namespace_definition` that lacks a body
/// (`namespace Foo;` form). Its name becomes the file's
/// module_path. Block-form namespaces (`namespace Foo { ... }`) are
/// handled per-block in `parse_block`.
fn extract_file_namespace(program: Node, source: &[u8]) -> Option<String> {
    let mut cursor = program.walk();
    for child in program.named_children(&mut cursor) {
        if child.kind() == "namespace_definition" && child.child_by_field_name("body").is_none() {
            return child
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string());
        }
    }
    None
}

impl LanguageParser for PhpParser {
    fn language_name(&self) -> &'static str {
        "php"
    }
    fn file_extensions(&self) -> &'static [&'static str] {
        &["php"]
    }
    fn parse_file(&self, filepath: &Path, src_root: &Path) -> ParseResult {
        let mut result = ParseResult::new();
        let Ok(source) = std::fs::read_to_string(filepath) else {
            return result;
        };
        let source_bytes = source.as_bytes();
        let rel_path = filepath
            .strip_prefix(src_root)
            .unwrap_or(filepath)
            .to_string_lossy()
            .to_string();
        let path_default_module = file_to_module_path(filepath, src_root, '\\');

        let Some(tree) = self.parse_tree(source_bytes) else {
            return result;
        };
        let root = tree.root_node();

        // Determine the file-level module_path. PHP files can declare
        // a top-level `namespace Foo;` that applies to everything
        // after; block-form `namespace Foo { ... }` is per-block (see
        // parse_block). If neither exists, fall back to the file path
        // derivation so unnamespaced PHP still gets unique qnames.
        let file_namespace = extract_file_namespace(root, source_bytes);
        let module_path = file_namespace.unwrap_or(path_default_module);

        let filename = filepath
            .file_name()
            .and_then(|o| o.to_str())
            .unwrap_or("")
            .to_string();
        let is_test = crate::parsers::shared::is_test_path(&rel_path, &filename, &["Test.php"]);
        let mut file_info = FileInfo {
            path: rel_path.clone(),
            filename,
            loc: source.lines().count() as u32,
            module_path: module_path.clone(),
            language: "php".to_string(),
            submodule_declarations: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            annotations: None,
            is_test,
            skip_reason: None,
        };

        Self::parse_block(
            root,
            source_bytes,
            &module_path,
            "",
            &rel_path,
            &mut result,
            &mut file_info,
        );

        result.files.push(file_info);
        result
    }
}

/// P8 — `use` declarations, including the grouped form the old extractor
/// collapsed to its bare prefix.
#[cfg(test)]
mod use_import_tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_imports(source: &str) -> Vec<String> {
        let tmp = tempfile::Builder::new()
            .prefix("codingest-php-use-")
            .tempdir()
            .expect("tempdir");
        let root = tmp.path().join("proj");
        let path: PathBuf = root.join("index.php");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(&path, source).expect("write snippet");
        let result = PhpParser::new().parse_file(&path, &root);
        result.files[0].imports.clone()
    }

    /// The reported defect: `use App\Models\{User, Post};` recorded a single
    /// import naming the prefix `App\Models`, and both members were lost.
    #[test]
    fn grouped_use_records_every_member() {
        assert_eq!(
            parse_imports("<?php\nuse App\\Models\\{User, Post};\n"),
            vec![
                "App\\Models\\User".to_string(),
                "App\\Models\\Post".to_string()
            ]
        );
    }

    /// A group member may itself be qualified — the prefix and the member's
    /// own path both belong to the recorded import.
    #[test]
    fn grouped_use_joins_qualified_members() {
        assert_eq!(
            parse_imports("<?php\nuse App\\Domain\\{Billing\\Invoice, Catalog\\Product};\n"),
            vec![
                "App\\Domain\\Billing\\Invoice".to_string(),
                "App\\Domain\\Catalog\\Product".to_string(),
            ]
        );
    }

    /// Inside a group an `as` alias is still ignored: the import is the path.
    #[test]
    fn grouped_use_with_alias_records_the_path() {
        assert_eq!(
            parse_imports("<?php\nuse App\\{Foo as Bar};\n"),
            vec!["App\\Foo".to_string()]
        );
        assert_eq!(
            parse_imports("<?php\nuse App\\Models\\{User, Post as Article};\n"),
            vec![
                "App\\Models\\User".to_string(),
                "App\\Models\\Post".to_string()
            ]
        );
    }

    /// `use function`/`use const` groups carry the same prefix join.
    #[test]
    fn grouped_function_use_records_every_member() {
        assert_eq!(
            parse_imports("<?php\nuse function App\\Support\\{head, tail};\n"),
            vec![
                "App\\Support\\head".to_string(),
                "App\\Support\\tail".to_string(),
            ]
        );
    }

    /// The non-grouped shapes are untouched.
    #[test]
    fn plain_and_aliased_use_are_unchanged() {
        assert_eq!(
            parse_imports("<?php\nuse App\\Models\\Thing;\n"),
            vec!["App\\Models\\Thing".to_string()]
        );
        assert_eq!(
            parse_imports("<?php\nuse App\\Models\\Other as Alias;\n"),
            vec!["App\\Models\\Other".to_string()]
        );
        assert_eq!(
            parse_imports("<?php\nuse function App\\helper;\n"),
            vec!["App\\helper".to_string()]
        );
        // Several clauses in one statement: `use A\B, C\D;`
        assert_eq!(
            parse_imports("<?php\nuse App\\A, App\\B;\n"),
            vec!["App\\A".to_string(), "App\\B".to_string()]
        );
    }
}

/// P8b — `is_method` marks methods, not top-level functions.
#[cfg(test)]
mod is_method_tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_snippet(source: &str) -> ParseResult {
        let tmp = tempfile::Builder::new()
            .prefix("codingest-php-is-method-")
            .tempdir()
            .expect("tempdir");
        let root = tmp.path().join("proj");
        let path: PathBuf = root.join("index.php");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(&path, source).expect("write snippet");
        PhpParser::new().parse_file(&path, &root)
    }

    /// The reported inversion: a top-level `function` was stored with
    /// `is_method = true` and a class method with `is_method = false`.
    #[test]
    fn top_level_function_is_not_a_method_and_a_method_is() {
        let result = parse_snippet(
            "<?php\n\
             function helper($x) { return $x; }\n\
             class Svc {\n\
               public function run($x) { return $x; }\n\
             }\n",
        );
        let seen: Vec<(&str, bool)> = result
            .functions
            .iter()
            .map(|f| (f.name.as_str(), f.is_method))
            .collect();
        assert_eq!(seen, vec![("helper", false), ("run", true)]);
    }

    /// A namespaced top-level function is still not a method — the namespace
    /// lands in `module_path`, not in the owner prefix.
    #[test]
    fn namespaced_top_level_function_is_not_a_method() {
        let result = parse_snippet(
            "<?php\n\
             namespace App\\Support;\n\
             function head(array $xs) { return $xs[0]; }\n",
        );
        assert_eq!(result.functions.len(), 1);
        assert_eq!(result.functions[0].name, "head");
        assert!(!result.functions[0].is_method);
    }

    /// Trait and interface methods are methods too.
    #[test]
    fn trait_and_interface_methods_are_methods() {
        let result = parse_snippet(
            "<?php\n\
             trait T { public function fromTrait() { } }\n\
             interface I { public function fromInterface(); }\n",
        );
        let seen: Vec<(&str, bool)> = result
            .functions
            .iter()
            .map(|f| (f.name.as_str(), f.is_method))
            .collect();
        assert_eq!(seen, vec![("fromTrait", true), ("fromInterface", true)]);
    }
}
