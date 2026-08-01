//! TypeScript + JavaScript parsers (ported from parsers/typescript.py).

use serde_json::json;
use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

use super::shared::{
    compute_complexity, count_lines, extract_comment_annotations, extract_procedure_annotations,
    get_type_parameters, is_generated_or_minified, node_text, BRANCH_KINDS_TS,
    DEFAULT_COMMENT_TYPES,
};
use super::LanguageParser;
use crate::models::{
    AttributeInfo, ClassInfo, ConstantInfo, EnumInfo, FileInfo, FunctionInfo, InterfaceInfo,
    ParameterInfo, ParameterKind, ParseResult, TypeRelationship,
};

pub const JSTS_NOISE_NAMES: &[&str] = &[
    // Array methods
    "push",
    "pop",
    "shift",
    "unshift",
    "map",
    "filter",
    "reduce",
    "forEach",
    "find",
    "findIndex",
    "some",
    "every",
    "includes",
    "indexOf",
    "slice",
    "splice",
    "concat",
    "join",
    "flat",
    "flatMap",
    "sort",
    "reverse",
    // Object methods
    "keys",
    "values",
    "entries",
    "assign",
    "freeze",
    "hasOwnProperty",
    "toString",
    "valueOf",
    // String methods
    "trim",
    "split",
    "replace",
    "match",
    "test",
    "search",
    "startsWith",
    "endsWith",
    "substring",
    "toLowerCase",
    "toUpperCase",
    // Promise methods
    "then",
    "catch",
    "finally",
    "resolve",
    "reject",
    // Console methods
    "log",
    "warn",
    "error",
    "info",
    "debug",
    // DOM / common
    "addEventListener",
    "removeEventListener",
    "querySelector",
    "getElementById",
    "createElement",
];

const NESTED_SCOPES: &[&str] = &[
    "function_declaration",
    "function",
    "arrow_function",
    "method_definition",
    "generator_function_declaration",
];

pub enum JstsFlavor {
    TypeScript,
    Tsx,
    JavaScript,
}

pub struct JstsParser {
    lang_name: &'static str,
    extensions: &'static [&'static str],
}

thread_local! {
    static TS_PARSER: std::cell::RefCell<Parser> = {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .expect("loading tree-sitter-typescript grammar");
        std::cell::RefCell::new(p)
    };
    // `.tsx` needs the JSX-aware grammar: the plain TypeScript grammar can't
    // parse JSX, so every component body desyncs into ERROR nodes and the
    // enclosing `export default function App()` loses its name. The two grammars
    // are NOT interchangeable the other way — TSX reads `<T>expr` as JSX, which
    // would misparse `.ts` type assertions/generics — so we keep them separate.
    static TSX_PARSER: std::cell::RefCell<Parser> = {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
            .expect("loading tree-sitter-tsx grammar");
        std::cell::RefCell::new(p)
    };
    static JS_PARSER: std::cell::RefCell<Parser> = {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("loading tree-sitter-javascript grammar");
        std::cell::RefCell::new(p)
    };
}

impl JstsParser {
    pub fn typescript() -> Self {
        JstsParser {
            lang_name: "typescript",
            extensions: &["ts", "tsx"],
        }
    }

    pub fn javascript() -> Self {
        JstsParser {
            lang_name: "javascript",
            extensions: &["js", "jsx", "mjs"],
        }
    }

    fn parse_tree(&self, source: &[u8], is_tsx: bool) -> Option<Tree> {
        if self.lang_name == "typescript" {
            if is_tsx {
                TSX_PARSER.with(|p| p.borrow_mut().parse(source, None))
            } else {
                TS_PARSER.with(|p| p.borrow_mut().parse(source, None))
            }
        } else {
            // tree-sitter-javascript already handles JSX, so `.jsx` is fine here.
            JS_PARSER.with(|p| p.borrow_mut().parse(source, None))
        }
    }

    fn get_name<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(
                child.kind(),
                "identifier" | "type_identifier" | "property_identifier"
            ) {
                return node_text(child, source);
            }
        }
        "unknown"
    }

    fn get_visibility(node: Node, source: &[u8]) -> &'static str {
        if let Some(parent) = node.parent() {
            if parent.kind() == "export_statement" {
                return "export";
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "accessibility_modifier" {
                return match node_text(child, source) {
                    "private" => "private",
                    "protected" => "protected",
                    _ => "public",
                };
            }
        }
        "private"
    }

    fn get_block<'a>(node: Node<'a>) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "statement_block" | "class_body") {
                return Some(child);
            }
        }
        None
    }

    fn get_docstring(node: Node, source: &[u8]) -> Option<String> {
        let sibling = node.prev_named_sibling()?;
        if sibling.kind() != "comment" {
            return None;
        }
        let text = node_text(sibling, source).trim();
        let rest = text.strip_prefix("/**")?;
        let rest = rest.strip_suffix("*/").unwrap_or(rest);
        let mut lines = Vec::new();
        for line in rest.split('\n') {
            let line = line.trim();
            let cleaned = if let Some(r) = line.strip_prefix("* ") {
                r
            } else if let Some(r) = line.strip_prefix('*') {
                r
            } else {
                line
            };
            lines.push(cleaned);
        }
        let joined = lines.join("\n").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    fn get_heritage(node: Node, source: &[u8]) -> (Vec<String>, Vec<String>) {
        let mut extends = Vec::new();
        let mut implements = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let target = match child.kind() {
                "extends_clause" => &mut extends,
                "implements_clause" => &mut implements,
                _ => continue,
            };
            let mut sub_cursor = child.walk();
            for sub in child.children(&mut sub_cursor) {
                match sub.kind() {
                    "identifier" | "type_identifier" | "member_expression" => {
                        target.push(node_text(sub, source).to_string());
                    }
                    "generic_type" => {
                        let mut inner_cursor = sub.walk();
                        for inner in sub.children(&mut inner_cursor) {
                            if matches!(inner.kind(), "identifier" | "type_identifier") {
                                target.push(node_text(inner, source).to_string());
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        (extends, implements)
    }

    fn get_signature(node: Node, source: &[u8]) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "statement_block" | "class_body") {
                break;
            }
            parts.push(node_text(child, source));
        }
        parts.join(" ")
    }

    fn get_return_type(node: Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_annotation" {
                let text = node_text(child, source);
                let stripped = text.strip_prefix(':').unwrap_or(text).trim();
                return Some(stripped.to_string());
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
                if text == "function" || text == "(" {
                    break;
                }
            }
        }
        false
    }

    fn extract_calls(body: Node, source: &[u8]) -> Vec<(String, u32)> {
        let mut calls: Vec<(String, u32)> = Vec::new();
        fn walk(node: Node, source: &[u8], out: &mut Vec<(String, u32)>) {
            match node.kind() {
                "call_expression" => {
                    let line = node.start_position().row as u32 + 1;
                    if let Some(func) = node.child(0) {
                        match func.kind() {
                            "identifier" => {
                                out.push((node_text(func, source).to_string(), line));
                            }
                            "member_expression" => {
                                let prop = func.child_by_field_name("property");
                                let obj = func.child_by_field_name("object");
                                match (prop, obj) {
                                    (Some(prop), Some(obj)) => {
                                        let prop_name = node_text(prop, source);
                                        let obj_text = node_text(obj, source);
                                        let hint = obj_text.rsplit('.').next().unwrap_or(obj_text);
                                        if matches!(
                                            hint,
                                            "this" | "super" | "window" | "document" | "console"
                                        ) {
                                            out.push((prop_name.to_string(), line));
                                        } else {
                                            out.push((format!("{}.{}", hint, prop_name), line));
                                        }
                                    }
                                    (Some(prop), None) => {
                                        out.push((node_text(prop, source).to_string(), line));
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "new_expression" => {
                    let line = node.start_position().row as u32 + 1;
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "identifier" {
                            out.push((node_text(child, source).to_string(), line));
                            break;
                        }
                    }
                }
                _ => {}
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let k = child.kind();
                // Inline anonymous functions — callbacks like `.map(x => foo(x))`
                // and JSX handlers like `onClick={() => foo()}` — get no graph
                // node, so their calls belong to the enclosing function. Arrow /
                // function expressions *assigned to a name* are node-ified
                // elsewhere; we keep skipping those (they're in NESTED_SCOPES) to
                // avoid double-counting, and only descend when the anonymous fn
                // sits directly in a call-argument or JSX-expression position,
                // which is never node-ified.
                let inline_anon = matches!(k, "arrow_function" | "function")
                    && matches!(node.kind(), "arguments" | "jsx_expression");
                if inline_anon || !NESTED_SCOPES.contains(&k) {
                    walk(child, source, out);
                }
            }
        }
        walk(body, source, &mut calls);
        calls
    }

    /// The module specifier of an `import … from '…'` / `export … from '…'`
    /// statement, unquoted. `None` when the statement has no `from` clause
    /// (`export const x = 1`, `export { a }`).
    ///
    /// Scope: static `import`/`export` statements only. `require()` and
    /// dynamic `import()` are call expressions, not statements, and stay out
    /// of scope — resolving them means evaluating an arbitrary expression,
    /// and in the corpora this targets they are a rounding error next to the
    /// static forms.
    fn module_source(node: Node, source: &[u8]) -> Option<String> {
        let string_node = node.child_by_field_name("source").or_else(|| {
            // Older grammar revisions do not set the field on plain
            // side-effect imports (`import './setup'`); there the specifier is
            // the statement's only direct string child. Deliberately NOT
            // applied to `export_statement`, whose direct children can include
            // an unrelated string literal.
            if node.kind() != "import_statement" {
                return None;
            }
            let mut cursor = node.walk();
            let found = node.children(&mut cursor).find(|c| c.kind() == "string");
            found
        })?;
        let text = node_text(string_node, source);
        let path = text.trim_matches(|c| c == '\'' || c == '"' || c == '`');
        (!path.is_empty()).then(|| path.to_string())
    }

    fn file_to_module_path(filepath: &Path, src_root: &Path) -> String {
        let rel = filepath.strip_prefix(src_root).unwrap_or(filepath);
        let mut parts: Vec<String> = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
            .collect();
        if let Some(last) = parts.last_mut() {
            for ext in [".tsx", ".ts", ".jsx", ".mjs", ".js"] {
                if let Some(stem) = last.strip_suffix(ext) {
                    *last = stem.to_string();
                    break;
                }
            }
            if last == "index" {
                parts.pop();
            }
        }
        if parts.is_empty() {
            src_root
                .file_name()
                .and_then(|o| o.to_str())
                .unwrap_or("")
                .to_string()
        } else {
            parts.join("/")
        }
    }

    fn get_decorators(node: Node, source: &[u8]) -> Vec<String> {
        let mut decs = Vec::new();
        let mut sibling = node.prev_named_sibling();
        while let Some(s) = sibling {
            match s.kind() {
                "decorator" => {
                    let text = node_text(s, source).trim();
                    let stripped = text.strip_prefix('@').unwrap_or(text);
                    decs.insert(0, stripped.to_string());
                    sibling = s.prev_named_sibling();
                }
                "comment" => {
                    sibling = s.prev_named_sibling();
                }
                _ => break,
            }
        }
        decs
    }

    fn extract_class_fields(
        body: Node,
        source: &[u8],
        rel_path: &str,
        owner_qname: &str,
        result: &mut ParseResult,
    ) {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if !matches!(
                child.kind(),
                "public_field_definition" | "property_declaration" | "field_definition"
            ) {
                continue;
            }
            let mut name: Option<String> = None;
            let mut type_ann: Option<String> = None;
            let mut default_val: Option<String> = None;
            let mut visibility = "private".to_string();

            let mut fc = child.walk();
            for sub in child.children(&mut fc) {
                match sub.kind() {
                    "property_identifier" | "identifier" if name.is_none() => {
                        name = Some(node_text(sub, source).to_string());
                    }
                    "type_annotation" => {
                        let text = node_text(sub, source);
                        let stripped = text.strip_prefix(':').unwrap_or(text).trim();
                        type_ann = Some(stripped.to_string());
                    }
                    "accessibility_modifier" => {
                        visibility = node_text(sub, source).to_string();
                    }
                    _ => {}
                }
            }
            let Some(name) = name else { continue };
            let mut saw_eq = false;
            let mut fc2 = child.walk();
            for sub in child.children(&mut fc2) {
                if !sub.is_named() && node_text(sub, source) == "=" {
                    saw_eq = true;
                } else if saw_eq {
                    let text = node_text(sub, source);
                    let take = text
                        .char_indices()
                        .nth(100)
                        .map(|(i, _)| i)
                        .unwrap_or(text.len());
                    default_val = Some(text[..take].to_string());
                    break;
                }
            }
            result.attributes.push(AttributeInfo {
                qualified_name: format!("{}.{}", owner_qname, name),
                owner_qualified_name: owner_qname.to_string(),
                type_annotation: type_ann,
                visibility,
                file_path: rel_path.to_string(),
                line_number: child.start_position().row as u32 + 1,
                default_value: default_val,
                name,
            });
        }
    }

    fn get_enum_members(node: Node, source: &[u8]) -> Vec<String> {
        let mut members = Vec::new();
        let mut body = Self::get_block(node);
        if body.is_none() {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "enum_body" {
                    body = Some(child);
                    break;
                }
            }
        }
        if let Some(body) = body {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                match child.kind() {
                    "enum_assignment" | "property_identifier" => {
                        let text = node_text(child, source);
                        let name = text.split('=').next().unwrap_or(text).trim();
                        members.push(name.to_string());
                    }
                    "identifier" => {
                        members.push(node_text(child, source).to_string());
                    }
                    _ => {}
                }
            }
        }
        members
    }

    fn parse_function(
        &self,
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

        let mut metadata = crate::models::MetadataMap::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() && node_text(child, source) == "static" {
                metadata.insert("is_static".into(), json!(true));
                break;
            }
        }

        let calls = block
            .map(|b| Self::extract_calls(b, source))
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
                let (c, n) = compute_complexity(b, BRANCH_KINDS_TS, NESTED_SCOPES);
                (Some(c), Some(n))
            }
            None => (None, None),
        };
        let is_recursive = Some(calls.iter().any(|(n, _)| n == &name));
        let docstring = Self::get_docstring(node, source);
        let procedure_names = extract_procedure_annotations(docstring.as_deref());

        FunctionInfo {
            visibility: Self::get_visibility(node, source).to_string(),
            is_async: Self::is_async(node, source),
            is_method,
            signature: Self::get_signature(node, source),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring,
            return_type: Self::get_return_type(node, source),
            decorators: Self::get_decorators(node, source),
            calls,
            references: Vec::new(),
            function_refs: Vec::new(),
            type_parameters: get_type_parameters(node, source, "type_parameters"),
            parameters,
            branch_count,
            param_count,
            max_nesting,
            is_recursive,
            procedure_names,
            metadata,
            qualified_name,
            name,
        }
    }

    /// Extract structured parameters from a TS/JS function-like node.
    /// Walks the `formal_parameters` child. Distinguishes rest params (`...args`).
    fn extract_parameters(node: Node, source: &[u8]) -> Vec<ParameterInfo> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        let Some(params_node) = node
            .children(&mut cursor)
            .find(|c| c.kind() == "formal_parameters")
        else {
            return out;
        };
        let mut pcursor = params_node.walk();
        for child in params_node.children(&mut pcursor) {
            let kind = child.kind();
            let (name, type_ann, default, pkind) = match kind {
                "required_parameter" | "optional_parameter" => {
                    let mut name: Option<String> = None;
                    let mut type_ann: Option<String> = None;
                    let mut default: Option<String> = None;
                    let mut tcursor = child.walk();
                    for sub in child.children(&mut tcursor) {
                        match sub.kind() {
                            "identifier" if name.is_none() => {
                                name = Some(node_text(sub, source).to_string())
                            }
                            "type_annotation" => {
                                let t = node_text(sub, source);
                                let cleaned = t.trim_start_matches(':').trim().to_string();
                                if !cleaned.is_empty() {
                                    type_ann = Some(cleaned);
                                }
                            }
                            // default value: anything past `=` (we approximate)
                            _ => {}
                        }
                    }
                    // crude default extraction: search for `=` then next named child
                    let text = node_text(child, source);
                    if let Some(idx) = text.find('=') {
                        let default_text = text[idx + 1..].trim();
                        if !default_text.is_empty() {
                            default = Some(default_text.to_string());
                        }
                    }
                    let Some(n) = name else { continue };
                    (n, type_ann, default, ParameterKind::Positional)
                }
                "rest_pattern" => {
                    let raw = node_text(child, source);
                    let n = raw.trim_start_matches("...").trim().to_string();
                    if n.is_empty() {
                        continue;
                    }
                    (n, None, None, ParameterKind::Variadic)
                }
                "identifier" => (
                    node_text(child, source).to_string(),
                    None,
                    None,
                    ParameterKind::Positional,
                ),
                _ => continue,
            };
            out.push(ParameterInfo {
                name,
                type_annotation: type_ann,
                default,
                kind: pkind,
            });
        }
        out
    }

    fn parse_class(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let name = Self::get_name(node, source).to_string();
        let qualified_name = format!("{}.{}", module_path, name);
        let (extends, implements) = Self::get_heritage(node, source);
        let docstring = Self::get_docstring(node, source);
        let decorators = Self::get_decorators(node, source);

        let mut metadata = crate::models::MetadataMap::new();
        if !decorators.is_empty() {
            metadata.insert("decorators".into(), json!(decorators));
        }

        result.classes.push(ClassInfo {
            qualified_name: qualified_name.clone(),
            kind: "class".into(),
            visibility: Self::get_visibility(node, source).to_string(),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring,
            bases: extends.clone(),
            type_parameters: get_type_parameters(node, source, "type_parameters"),
            metadata,
            name: name.clone(),
        });

        for base in &extends {
            result.type_relationships.push(TypeRelationship {
                source_type: name.clone(),
                target_type: Some(base.clone()),
                relationship: "extends".into(),
                methods: Vec::new(),
            });
        }
        for iface in &implements {
            result.type_relationships.push(TypeRelationship {
                source_type: name.clone(),
                target_type: Some(iface.clone()),
                relationship: "implements".into(),
                methods: Vec::new(),
            });
        }

        let mut method_rel = TypeRelationship {
            source_type: qualified_name.clone(),
            target_type: None,
            relationship: "inherent".into(),
            methods: Vec::new(),
        };

        if let Some(body) = Self::get_block(node) {
            Self::extract_class_fields(body, source, rel_path, &qualified_name, result);
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if matches!(child.kind(), "method_definition" | "function_declaration") {
                    let fn_info = self.parse_function(
                        child,
                        source,
                        module_path,
                        rel_path,
                        true,
                        Some(&name),
                    );
                    method_rel.methods.push(fn_info.clone());
                    result.functions.push(fn_info);
                }
            }
        }

        if !method_rel.methods.is_empty() {
            result.type_relationships.push(method_rel);
        }
    }

    fn parse_interface(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let name = Self::get_name(node, source).to_string();
        let qualified_name = format!("{}.{}", module_path, name);
        let (extends, _) = Self::get_heritage(node, source);
        let docstring = Self::get_docstring(node, source);

        result.interfaces.push(InterfaceInfo {
            qualified_name: qualified_name.clone(),
            kind: "interface".into(),
            visibility: Self::get_visibility(node, source).to_string(),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring,
            type_parameters: get_type_parameters(node, source, "type_parameters"),
            name: name.clone(),
        });

        for base in &extends {
            result.type_relationships.push(TypeRelationship {
                source_type: name.clone(),
                target_type: Some(base.clone()),
                relationship: "extends".into(),
                methods: Vec::new(),
            });
        }

        let mut iface_rel = TypeRelationship {
            source_type: qualified_name,
            target_type: None,
            relationship: "inherent".into(),
            methods: Vec::new(),
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "interface_body" | "object_type") {
                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    if matches!(item.kind(), "method_signature" | "method_definition") {
                        let fn_info = self.parse_function(
                            item,
                            source,
                            module_path,
                            rel_path,
                            true,
                            Some(&name),
                        );
                        iface_rel.methods.push(fn_info.clone());
                        result.functions.push(fn_info);
                    }
                }
            }
        }
        if !iface_rel.methods.is_empty() {
            result.type_relationships.push(iface_rel);
        }
    }

    fn parse_enum(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
    ) {
        let name = Self::get_name(node, source).to_string();
        let qualified_name = format!("{}.{}", module_path, name);
        result.enums.push(EnumInfo {
            qualified_name,
            visibility: Self::get_visibility(node, source).to_string(),
            file_path: rel_path.to_string(),
            line_number: node.start_position().row as u32 + 1,
            end_line: Some(node.end_position().row as u32 + 1),
            docstring: Self::get_docstring(node, source),
            variants: Self::get_enum_members(node, source),
            variant_details: None,
            name,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_top_level(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        result: &mut ParseResult,
        file_info: &mut FileInfo,
    ) {
        match node.kind() {
            "function_declaration" | "function" => {
                let fn_info = self.parse_function(node, source, module_path, rel_path, false, None);
                result.functions.push(fn_info);
            }
            "class_declaration" => self.parse_class(node, source, module_path, rel_path, result),
            "interface_declaration" => {
                self.parse_interface(node, source, module_path, rel_path, result)
            }
            "enum_declaration" => self.parse_enum(node, source, module_path, rel_path, result),
            "export_statement" => {
                // `export … from '…'` / `export * from '…'` is an import in
                // every sense that matters to a dependency graph: the file
                // reads the module. Barrel files (`index.ts` re-exports) are
                // the main dependency conduit in a TS monorepo and consist of
                // nothing else, so without this they contribute no edges at
                // all. Recorded into `imports` rather than a parallel list so
                // one resolution path covers both spellings.
                if let Some(path) = Self::module_source(node, source) {
                    file_info.imports.push(path);
                }
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "function_declaration"
                        | "class_declaration"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "type_alias_declaration" => {
                            self.parse_top_level(
                                child,
                                source,
                                module_path,
                                rel_path,
                                result,
                                file_info,
                            );
                            let name = Self::get_name(child, source);
                            if name != "unknown" {
                                file_info.exports.push(name.to_string());
                            }
                        }
                        "lexical_declaration" => {
                            self.parse_top_level(
                                child,
                                source,
                                module_path,
                                rel_path,
                                result,
                                file_info,
                            );
                            let mut ic = child.walk();
                            for sub in child.children(&mut ic) {
                                if sub.kind() == "variable_declarator" {
                                    if let Some(name_node) = sub.child_by_field_name("name") {
                                        file_info
                                            .exports
                                            .push(node_text(name_node, source).to_string());
                                    }
                                }
                            }
                        }
                        "export_clause" => {
                            let mut ec = child.walk();
                            for sub in child.children(&mut ec) {
                                if sub.kind() == "export_specifier" {
                                    let mut sc = sub.walk();
                                    for inner in sub.children(&mut sc) {
                                        if inner.kind() == "identifier" {
                                            file_info
                                                .exports
                                                .push(node_text(inner, source).to_string());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "import_statement" => {
                // Record the specifier verbatim — relative ones included.
                // `./x` and `../x` used to be dropped here, which made TS
                // import resolution structurally impossible: the builder never
                // saw the specifiers that carry ~all intra-project dependency
                // in a TS codebase. Normalization against the importing file's
                // directory happens in `builder::other_edges`, which is the
                // only place that knows the project's module set.
                if let Some(path) = Self::module_source(node, source) {
                    file_info.imports.push(path);
                }
            }
            "type_alias_declaration" => {
                let name = Self::get_name(node, source).to_string();
                let mut value_node: Option<Node> = None;
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "type_annotation" {
                        value_node = Some(child);
                        break;
                    }
                }
                if value_node.is_none() {
                    let mut saw_eq = false;
                    let mut cursor2 = node.walk();
                    for child in node.children(&mut cursor2) {
                        if !child.is_named() && node_text(child, source) == "=" {
                            saw_eq = true;
                        } else if saw_eq {
                            value_node = Some(child);
                            break;
                        }
                    }
                }
                let preview = value_node.map(|v| {
                    let text = node_text(v, source);
                    let take = text
                        .char_indices()
                        .nth(100)
                        .map(|(i, _)| i)
                        .unwrap_or(text.len());
                    text[..take].to_string()
                });
                result.constants.push(ConstantInfo {
                    qualified_name: format!("{}.{}", module_path, name),
                    kind: "type_alias".into(),
                    type_annotation: None,
                    value_preview: preview,
                    visibility: Self::get_visibility(node, source).to_string(),
                    file_path: rel_path.to_string(),
                    line_number: node.start_position().row as u32 + 1,
                    name,
                });
            }
            "lexical_declaration" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() != "variable_declarator" {
                        continue;
                    }
                    let name_node = child.child_by_field_name("name");
                    let value = child.child_by_field_name("value");
                    if let (Some(name_node), Some(value)) = (name_node, value) {
                        if matches!(value.kind(), "arrow_function" | "function") {
                            let mut fn_info = self.parse_function(
                                value,
                                source,
                                module_path,
                                rel_path,
                                false,
                                None,
                            );
                            fn_info.name = node_text(name_node, source).to_string();
                            fn_info.qualified_name = format!("{}.{}", module_path, fn_info.name);
                            result.functions.push(fn_info);
                            continue;
                        }
                    }
                    if let Some(name_node) = name_node {
                        let name = node_text(name_node, source).to_string();
                        let mut type_ann: Option<String> = None;
                        let mut sc = child.walk();
                        for sub in child.children(&mut sc) {
                            if sub.kind() == "type_annotation" {
                                let text = node_text(sub, source);
                                let stripped = text.strip_prefix(':').unwrap_or(text).trim();
                                type_ann = Some(stripped.to_string());
                                break;
                            }
                        }
                        let preview = value.map(|v| {
                            let text = node_text(v, source);
                            let take = text
                                .char_indices()
                                .nth(100)
                                .map(|(i, _)| i)
                                .unwrap_or(text.len());
                            text[..take].to_string()
                        });
                        result.constants.push(ConstantInfo {
                            qualified_name: format!("{}.{}", module_path, name),
                            kind: "constant".into(),
                            type_annotation: type_ann,
                            value_preview: preview,
                            visibility: Self::get_visibility(node, source).to_string(),
                            file_path: rel_path.to_string(),
                            line_number: child.start_position().row as u32 + 1,
                            name,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

impl LanguageParser for JstsParser {
    fn language_name(&self) -> &'static str {
        self.lang_name
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        self.extensions
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
        let test_suffixes = [
            ".test.ts",
            ".spec.ts",
            ".test.tsx",
            ".spec.tsx",
            ".test.js",
            ".spec.js",
            ".test.jsx",
            ".spec.jsx",
            ".test.mjs",
            ".spec.mjs",
        ];
        let is_test = crate::parsers::shared::is_test_path(&rel_path, &filename, &test_suffixes);

        if let Some(reason) = is_generated_or_minified(&source) {
            let mut r = ParseResult::new();
            r.files.push(FileInfo {
                path: rel_path,
                filename,
                loc,
                module_path,
                language: self.lang_name.to_string(),
                submodule_declarations: Vec::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                annotations: None,
                is_test,
                skip_reason: Some(reason.to_string()),
            });
            return r;
        }

        let is_tsx = filepath
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("tsx"));
        let Some(tree) = self.parse_tree(&source, is_tsx) else {
            return ParseResult::new();
        };
        let root = tree.root_node();

        let mut file_info = FileInfo {
            path: rel_path.clone(),
            filename,
            loc,
            module_path: module_path.clone(),
            language: self.lang_name.to_string(),
            submodule_declarations: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            annotations: None,
            is_test,
            skip_reason: None,
        };

        let mut result = ParseResult::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            self.parse_top_level(
                child,
                &source,
                &module_path,
                &rel_path,
                &mut result,
                &mut file_info,
            );
        }
        file_info.annotations = extract_comment_annotations(root, &source, DEFAULT_COMMENT_TYPES);
        result.files.push(file_info);
        result
    }
}

#[cfg(test)]
mod import_capture_tests {
    use super::*;
    use crate::parsers::LanguageParser;

    /// Parse one `.ts` (or `.tsx`) source through the real parser and return
    /// the resulting `FileInfo.imports`, in encounter order.
    fn imports_of(name: &str, src: &str) -> Vec<String> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(name);
        std::fs::write(&path, src).expect("write fixture");
        let result = JstsParser::typescript().parse_file(&path, dir.path());
        assert_eq!(result.files.len(), 1, "expected exactly one parsed file");
        result.files[0].imports.clone()
    }

    /// The regression this phase exists for: relative specifiers used to be
    /// dropped at parse time (`if !path.starts_with('.')`), so the builder
    /// never saw the specifiers that carry essentially all intra-project
    /// dependency in a TS codebase.
    #[test]
    fn relative_import_specifiers_are_captured_verbatim() {
        let imports = imports_of(
            "a.ts",
            r#"
import { helper } from "./util"
import type { Deep } from "../nested/deep"
import defaultThing from './default-thing'
import "./side-effect"
import * as ns from "../../other/mod"
"#,
        );
        assert_eq!(
            imports,
            vec![
                "./util",
                "../nested/deep",
                "./default-thing",
                "./side-effect",
                "../../other/mod",
            ]
        );
    }

    /// Non-relative specifiers kept working — they were the only ones the old
    /// filter let through, and the alias/workspace phase resolves them.
    #[test]
    fn bare_and_scoped_specifiers_are_still_captured() {
        let imports = imports_of(
            "a.ts",
            r#"
import { z } from "zod"
import { Core } from "@scope/core"
import { Sub } from "@scope/core/sub/path"
"#,
        );
        assert_eq!(imports, vec!["zod", "@scope/core", "@scope/core/sub/path"]);
    }

    /// Barrel files consist of nothing but `export … from`, so before this
    /// they contributed zero edges — in a TS monorepo that is the main
    /// dependency conduit. Pins the grammar's node shape too: if a future
    /// tree-sitter-typescript stops exposing the `source` field on
    /// `export_statement`, this test says so immediately.
    #[test]
    fn export_from_specifiers_are_captured() {
        let imports = imports_of(
            "index.ts",
            r#"
export { helper } from "./util"
export * from "./nested/deep"
export * as ns from "../sibling"
export type { Cfg } from "./config"
"#,
        );
        assert_eq!(
            imports,
            vec!["./util", "./nested/deep", "../sibling", "./config"]
        );
    }

    /// An `export` with no `from` clause is not an import, and neither is a
    /// string literal that merely happens to sit inside one.
    #[test]
    fn exports_without_a_from_clause_capture_nothing() {
        let imports = imports_of(
            "a.ts",
            r#"
export const NAME = "not-a-module"
export function go(): string { return "also-not-a-module" }
const local = "plain"
export { local }
export default go
"#,
        );
        assert!(imports.is_empty(), "captured {imports:?}");
    }

    /// Out-of-scope spellings stay out of scope — documented, not accidental.
    #[test]
    fn require_and_dynamic_import_are_not_captured() {
        let imports = imports_of(
            "a.ts",
            r#"
const fs = require("./legacy")
export async function load() {
  return await import("./lazy")
}
"#,
        );
        assert!(imports.is_empty(), "captured {imports:?}");
    }
}
