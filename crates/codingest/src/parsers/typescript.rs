//! TypeScript + JavaScript parsers (ported from parsers/typescript.py).

use serde_json::json;
use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

use super::shared::{
    break_qualified_name_ties, compute_complexity, count_lines, extend_scope_chain,
    extract_comment_annotations, extract_procedure_annotations, get_type_parameters,
    is_generated_or_minified, node_text, scope_qualify, tag_scope, BRANCH_KINDS_TS,
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
    "arrow_function",
    "method_definition",
    "generator_function_declaration",
];

/// The function-literal node kinds tree-sitter-typescript 0.23.2 (and
/// tree-sitter-javascript) actually emit for a value position.
///
/// There is **no** node kind named `function` in either grammar:
/// `const x = function () {}` is a `function_expression` and
/// `const x = function* () {}` is a `generator_function`. Until 2026-08-01
/// this file matched on a bare `"function"` in three places — dead
/// vocabulary that silently demoted every such binding to a `Constant`, and
/// (via a missing `generator_function_declaration` arm) made a top-level
/// `function* g() {}` produce no node at all. Evidence + reproduction:
/// `dev-docs/bench/out/nested-spike/grammar-vocabulary-defects.txt`.
const FN_LITERALS: &[&str] = &[
    "arrow_function",
    "function_expression",
    "generator_function",
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

    // ── the narrowed factory unwrap (D1 item 3, as amended 2026-08-01) ───
    //
    // `const readFile = Effect.fn("Bom.readFile")(function* … )` binds a
    // function, but the declarator's value kind is `call_expression`, so it
    // used to fall through to the Constant branch and the export vanished
    // from the graph as a callable. Unwrapping *every* such binding is not
    // an option: the Phase 1 spike measured "the call chain contains exactly
    // one function literal" firing 4 345 times on opencode with a median
    // captured size of 1 LOC — `const names = users.map(u => u.name)` binds
    // an array, not a function. The rule below is the measured narrowing
    // (spike rule F5): exactly one literal, that literal is a generator or
    // its call is curried, and the call's callee is not a method on a value
    // receiver. See `dev-docs/plans/closure-scoped-definitions.md` D1.

    /// Strip the wrappers that do not change which value an expression
    /// produces: `(expr)`, `expr as T`, `expr satisfies T`, `expr!`.
    fn unwrap_expr(mut node: Node) -> Node {
        loop {
            match node.kind() {
                "parenthesized_expression"
                | "as_expression"
                | "satisfies_expression"
                | "non_null_expression" => match node.named_child(0) {
                    Some(inner) => node = inner,
                    None => return node,
                },
                _ => return node,
            }
        }
    }

    /// Every function literal reachable through a value's call chain,
    /// without descending into any literal's own body.
    ///
    /// Curried callees (`Effect.fn("n")(function*…)`) and call-valued
    /// arguments (`Layer.effect(S, Effect.gen(function*…))`) are both part
    /// of the chain. Object/array-literal arguments are not: an object
    /// argument's methods are configuration, not a factory wrap.
    fn chain_fn_literals<'t>(value: Node<'t>, out: &mut Vec<Node<'t>>) {
        let v = Self::unwrap_expr(value);
        if FN_LITERALS.contains(&v.kind()) {
            out.push(v);
            return;
        }
        if v.kind() != "call_expression" {
            return;
        }
        if let Some(callee) = v.child_by_field_name("function") {
            let callee = Self::unwrap_expr(callee);
            if callee.kind() == "call_expression" {
                Self::chain_fn_literals(callee, out);
            }
        }
        if let Some(args) = v.child_by_field_name("arguments") {
            let mut cursor = args.walk();
            for arg in args.named_children(&mut cursor) {
                Self::chain_fn_literals(arg, out);
            }
        }
    }

    /// The `wrapped_by` label: callee text of the *outermost* call in the
    /// chain, with curried callees descended so `Effect.fn("x")(fn)` reports
    /// `Effect.fn` rather than `Effect.fn("x")`.
    fn factory_wrapper_name(value: Node, source: &[u8]) -> Option<String> {
        let mut v = Self::unwrap_expr(value);
        loop {
            if v.kind() != "call_expression" {
                return None;
            }
            let callee = Self::unwrap_expr(v.child_by_field_name("function")?);
            if callee.kind() == "call_expression" {
                v = callee;
                continue;
            }
            return Some(node_text(callee, source).to_string());
        }
    }

    /// `Some((literal, wrapped_by))` when a `const|let|var x = <call-chain>`
    /// binding qualifies as a factory-wrapped function under the amended
    /// D1 item 3; `None` when it stays a `Constant`.
    fn factory_wrapped_fn<'t>(value: Node<'t>, source: &[u8]) -> Option<(Node<'t>, String)> {
        let root = Self::unwrap_expr(value);
        if root.kind() != "call_expression" {
            return None;
        }

        // Guard 1 — exactly one function literal in the chain. Zero means an
        // ordinary constant; two or more means we cannot say which one the
        // binding *is*.
        let mut literals: Vec<Node> = Vec::new();
        Self::chain_fn_literals(root, &mut literals);
        if literals.len() != 1 {
            return None;
        }
        let literal = literals[0];

        // The literal's immediate enclosing call, bounded by the binding value.
        let mut node = literal;
        let call = loop {
            let parent = node.parent()?;
            if parent.kind() == "call_expression" {
                break parent;
            }
            if parent.id() == root.id() {
                return None;
            }
            node = parent;
        };

        let callee = call.child_by_field_name("function").map(Self::unwrap_expr);

        // Guard 2 — the callee is not a method on a *value* receiver. A bare
        // identifier (`memoize`), a curried call (`Effect.fn("x")(…)`) and a
        // member on a Capitalized identifier (`Effect.fn`, `Layer.effect`)
        // read as factories; `arr.map`, `results.filter`, `this[k].map` and
        // `tp.split(',').map` read as operations on data.
        let non_value_receiver = match callee.map(|c| (c, c.kind())) {
            Some((_, "identifier")) | Some((_, "call_expression")) => true,
            Some((c, "member_expression")) => c
                .child_by_field_name("object")
                .map(Self::unwrap_expr)
                .is_some_and(|obj| {
                    obj.kind() == "identifier"
                        && node_text(obj, source)
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_uppercase())
                }),
            _ => false,
        };

        // Guard 3 — the literal is function-*like* in the wrap: a generator
        // (the Effect-TS / redux-saga shape) or a curried application. A
        // plain `memoize(x => x + 1)` binds the call's result, not the arrow.
        let curried = matches!(callee.map(|c| c.kind()), Some("call_expression"));
        let fn_like = curried || literal.kind() == "generator_function";

        if !(non_value_receiver && fn_like) {
            return None;
        }
        Some((literal, Self::factory_wrapper_name(root, source)?))
    }

    // ── Nested scope walk (D1 as amended / D2) ───────────────────────────
    //
    // `parse_top_level` used to look only at the direct children of the
    // program root, so everything declared inside a function body, an arrow
    // body, a generator body or a TS `namespace` was invisible: on opencode
    // that is ~37 % of the core package's named callables, including the
    // entire `Layer.effect(Service, Effect.gen(function* () { … }))` service
    // surface. The walk below descends into those scopes.
    //
    // What gets a node is D1 **as amended by the Phase 1 spike**: a named
    // binding (or grammar-named declaration) whose *whole enclosing scope
    // chain is itself named*. `chain: Option<&[String]>` is that rule in one
    // branch — entering an anonymous function literal replaces the chain with
    // `None`, `None` is absorbing, and `None` means "no node here". The walk
    // still descends through anonymous scopes; it simply cannot mint a node
    // beneath one. Dropping bindings under anonymous callbacks is what holds
    // opencode's node growth at 10.90 % instead of 14.15 % (ceiling 12 %),
    // and it is why no qualified name ever needs a positional `<anonL{line}>`
    // segment — see D2 in the plan.

    /// Build the `FunctionInfo` for a nested definition: `parse_function`
    /// plus the D2 identity (chain-qualified name) and properties.
    #[allow(clippy::too_many_arguments)]
    fn nested_function(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: &[String],
        name: &str,
        depth: u32,
        node_ified: &[usize],
    ) -> FunctionInfo {
        let mut info =
            self.parse_function(node, source, module_path, rel_path, false, None, node_ified);
        info.name = name.to_string();
        info.qualified_name = scope_qualify(module_path, chain, name);
        tag_scope(&mut info, module_path, chain, depth);
        info
    }

    /// Walk the members of a scope (`owner`'s named children) for nested
    /// definitions, emitting them into `out` in source order.
    ///
    /// `chain` is the scope chain *including* `owner`'s own name; `depth` is
    /// the nesting depth of whatever is declared inside it. Returns the node
    /// ids of the topmost definitions that became their own `Function` node,
    /// which is exactly what `owner`'s `extract_calls` must skip (D4).
    #[allow(clippy::too_many_arguments)]
    fn descend_scope(
        &self,
        owner: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
    ) -> Vec<usize> {
        let mut node_ified = Vec::new();
        // A `None` chain is *absorbing*: `extend_scope_chain` maps `None` to `None`
        // and no arm below ever rebuilds a `Some`, so nothing inside an
        // anonymous scope can be node-ified and nothing can be skipped
        // either. Not descending is therefore observably identical to
        // descending and declining everything — and on a callback-dense
        // codebase it is the difference between walking the whole AST twice
        // and walking only its named spine. The equivalence is asserted at
        // the anonymous entry point in `walk_scope`.
        if chain.is_none() {
            return node_ified;
        }
        let mut cursor = owner.walk();
        for member in owner.named_children(&mut cursor) {
            // Name / type chrome carries no scope; the body, an expression
            // body and `formal_parameters` (default values can hold arrow
            // literals) all get walked.
            if matches!(
                member.kind(),
                "identifier" | "property_identifier" | "type_parameters" | "type_annotation"
            ) {
                continue;
            }
            self.walk_scope(
                member,
                source,
                module_path,
                rel_path,
                chain,
                depth,
                out,
                &mut node_ified,
            );
        }
        node_ified
    }

    /// The recursive scope walk. `node` is any node inside some scope;
    /// `chain`/`depth` describe that scope.
    #[allow(clippy::too_many_arguments)]
    fn walk_scope(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
        node_ified: &mut Vec<usize>,
    ) {
        match node.kind() {
            // D1-1: grammar-named declarations.
            "function_declaration" | "generator_function_declaration" => {
                let name = Self::get_name(node, source).to_string();
                let inner_chain = extend_scope_chain(chain, &name);
                let mut kids = Vec::new();
                let inner = self.descend_scope(
                    node,
                    source,
                    module_path,
                    rel_path,
                    inner_chain.as_deref(),
                    depth + 1,
                    &mut kids,
                );
                self.emit_scope(
                    node,
                    source,
                    module_path,
                    rel_path,
                    chain,
                    &name,
                    depth,
                    &inner,
                    kids,
                    out,
                    node_ified,
                );
            }
            // D1-2 / D1-3: `const|let|var x = <literal | factory(…)>`.
            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = node.walk();
                for declarator in node
                    .named_children(&mut cursor)
                    .filter(|c| c.kind() == "variable_declarator")
                {
                    self.walk_declarator(
                        declarator,
                        source,
                        module_path,
                        rel_path,
                        chain,
                        depth,
                        out,
                        node_ified,
                    );
                }
            }
            // Transparent: `export const x = …` is positionally identical to
            // `const x = …` (this arm only fires inside a namespace body —
            // at file scope `parse_top_level` owns `export_statement`).
            "export_statement" => {
                let mut cursor = node.walk();
                for kid in node.named_children(&mut cursor) {
                    self.walk_scope(
                        kid,
                        source,
                        module_path,
                        rel_path,
                        chain,
                        depth,
                        out,
                        node_ified,
                    );
                }
            }
            // D1-4: `namespace X { … }`. Members are export-like, so the
            // namespace contributes a name segment but NOT a nesting level —
            // a namespace member is as globally addressable as a top-level
            // one and keeps full CALLS participation under D3.
            "internal_module" | "module" => {
                let Some(name_node) = node.child_by_field_name("name") else {
                    return;
                };
                // `declare module "pkg"` is an ambient declaration, not a
                // namespace scope.
                if name_node.kind() == "string" {
                    return;
                }
                let name = node_text(name_node, source).to_string();
                let inner_chain = extend_scope_chain(chain, &name);
                if let Some(body) = node.child_by_field_name("body") {
                    let inner = self.descend_scope(
                        body,
                        source,
                        module_path,
                        rel_path,
                        inner_chain.as_deref(),
                        depth,
                        out,
                    );
                    node_ified.extend(inner);
                }
            }
            // A class nested inside a scope is excluded from D1 (measured: 3
            // on opencode, 0 elsewhere) and so are its methods — but their
            // bodies still hold named bindings, and those are addressable via
            // `Module.Class.method.binding`.
            "class_declaration" | "class" | "abstract_class_declaration" => {
                let name = Self::get_name(node, source).to_string();
                let inner_chain = extend_scope_chain(chain, &name);
                let Some(body) = Self::get_block(node) else {
                    return;
                };
                let mut cursor = body.walk();
                for member in body.named_children(&mut cursor) {
                    if member.kind() == "method_definition" {
                        let method = Self::get_name(member, source).to_string();
                        let method_chain = extend_scope_chain(inner_chain.as_deref(), &method);
                        let inner = self.descend_scope(
                            member,
                            source,
                            module_path,
                            rel_path,
                            method_chain.as_deref(),
                            depth + 1,
                            out,
                        );
                        node_ified.extend(inner);
                    } else {
                        self.walk_scope(
                            member,
                            source,
                            module_path,
                            rel_path,
                            inner_chain.as_deref(),
                            depth,
                            out,
                            node_ified,
                        );
                    }
                }
            }
            // Reached structurally rather than through a binding: an
            // anonymous function literal. D1 clause 5 — nothing below it can
            // be node-ified, so the chain becomes `None` from here down.
            kind if FN_LITERALS.contains(&kind) => {
                let inner =
                    self.descend_scope(node, source, module_path, rel_path, None, depth + 1, out);
                debug_assert!(inner.is_empty(), "a None chain cannot node-ify anything");
                node_ified.extend(inner);
            }
            // Any other statement/expression: structural pass-through.
            _ => {
                let mut cursor = node.walk();
                for kid in node.named_children(&mut cursor) {
                    self.walk_scope(
                        kid,
                        source,
                        module_path,
                        rel_path,
                        chain,
                        depth,
                        out,
                        node_ified,
                    );
                }
            }
        }
    }

    /// Emit (or decline) a node for a definition whose body has already been
    /// walked. `inner` are the node ids node-ified directly inside it and
    /// `kids` the `FunctionInfo`s they produced.
    ///
    /// Declining is not the same as discarding: when the chain is `None`, or
    /// when a factory binding fails the D1-3 narrowing, the definitions
    /// *below* it may still be nodes, so their ids have to bubble up to
    /// whichever ancestor scope does own the surrounding call sites.
    #[allow(clippy::too_many_arguments)]
    fn emit_scope(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        name: &str,
        depth: u32,
        inner: &[usize],
        kids: Vec<FunctionInfo>,
        out: &mut Vec<FunctionInfo>,
        node_ified: &mut Vec<usize>,
    ) {
        match chain {
            Some(chain) => {
                let info = self.nested_function(
                    node,
                    source,
                    module_path,
                    rel_path,
                    chain,
                    name,
                    depth,
                    inner,
                );
                out.push(info);
                out.extend(kids);
                node_ified.push(node.id());
            }
            None => {
                out.extend(kids);
                node_ified.extend_from_slice(inner);
            }
        }
    }

    /// One `variable_declarator`. Returns `true` when the binding itself
    /// became a `Function` node — the signal `parse_top_level` uses to skip
    /// its `Constant` fallback.
    #[allow(clippy::too_many_arguments)]
    fn walk_declarator(
        &self,
        declarator: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
        node_ified: &mut Vec<usize>,
    ) -> bool {
        let Some(value) = declarator.child_by_field_name("value") else {
            return false;
        };
        // A destructuring pattern binds no single name, so D1 declines it
        // (49 such bindings on opencode) — but the value still holds code.
        let Some(name_node) = declarator
            .child_by_field_name("name")
            .filter(|n| n.kind() == "identifier")
        else {
            self.walk_scope(
                value,
                source,
                module_path,
                rel_path,
                chain,
                depth,
                out,
                node_ified,
            );
            return false;
        };
        let name = node_text(name_node, source).to_string();
        let unwrapped = Self::unwrap_expr(value);

        // D1-2 — a bare function literal.
        if FN_LITERALS.contains(&unwrapped.kind()) {
            let inner_chain = extend_scope_chain(chain, &name);
            let mut kids = Vec::new();
            let inner = self.descend_scope(
                unwrapped,
                source,
                module_path,
                rel_path,
                inner_chain.as_deref(),
                depth + 1,
                &mut kids,
            );
            let emitted = chain.is_some();
            self.emit_scope(
                unwrapped,
                source,
                module_path,
                rel_path,
                chain,
                &name,
                depth,
                &inner,
                kids,
                out,
                node_ified,
            );
            return emitted;
        }

        // D1-3 — a factory-wrapped literal. The scope descent runs for every
        // single-literal chain, narrowing or not: `const rows = xs.map(x => …)`
        // is a Constant, but a helper declared inside that callback is still
        // reachable as `Module.rows.helper`, which is what the Phase 1 spike
        // measured. Only the *node* for the binding itself is gated on the
        // narrowing.
        if unwrapped.kind() == "call_expression" {
            let mut literals: Vec<Node> = Vec::new();
            Self::chain_fn_literals(unwrapped, &mut literals);
            if literals.len() == 1 {
                let literal = literals[0];
                let inner_chain = extend_scope_chain(chain, &name);
                let mut kids = Vec::new();
                let inner = self.descend_scope(
                    literal,
                    source,
                    module_path,
                    rel_path,
                    inner_chain.as_deref(),
                    depth + 1,
                    &mut kids,
                );
                let wrapped = Self::factory_wrapped_fn(value, source);
                let emitted = match (chain, wrapped) {
                    (Some(chain), Some((_, wrapped_by))) => {
                        let mut info = self.nested_function(
                            literal,
                            source,
                            module_path,
                            rel_path,
                            chain,
                            &name,
                            depth,
                            &inner,
                        );
                        info.metadata.insert("wrapped_by".into(), json!(wrapped_by));
                        out.push(info);
                        out.extend(kids);
                        node_ified.push(literal.id());
                        true
                    }
                    _ => {
                        out.extend(kids);
                        node_ified.extend_from_slice(&inner);
                        false
                    }
                };
                // The rest of the call chain (other arguments) is ordinary
                // code belonging to the *enclosing* scope.
                self.walk_excluding(
                    unwrapped,
                    literal,
                    source,
                    module_path,
                    rel_path,
                    chain,
                    depth,
                    out,
                    node_ified,
                );
                return emitted;
            }
        }

        self.walk_scope(
            value,
            source,
            module_path,
            rel_path,
            chain,
            depth,
            out,
            node_ified,
        );
        false
    }

    /// Walk everything under `root` except the subtree rooted at `skip`.
    #[allow(clippy::too_many_arguments)]
    fn walk_excluding(
        &self,
        root: Node,
        skip: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        chain: Option<&[String]>,
        depth: u32,
        out: &mut Vec<FunctionInfo>,
        node_ified: &mut Vec<usize>,
    ) {
        let mut cursor = root.walk();
        for kid in root.named_children(&mut cursor) {
            if kid.id() == skip.id() {
                continue;
            }
            if kid.byte_range().start <= skip.byte_range().start
                && kid.byte_range().end >= skip.byte_range().end
            {
                self.walk_excluding(
                    kid,
                    skip,
                    source,
                    module_path,
                    rel_path,
                    chain,
                    depth,
                    out,
                    node_ified,
                );
            } else {
                self.walk_scope(
                    kid,
                    source,
                    module_path,
                    rel_path,
                    chain,
                    depth,
                    out,
                    node_ified,
                );
            }
        }
    }

    /// Call sites belonging to `body`, minus the subtrees in `node_ified`.
    ///
    /// `node_ified` holds the tree-sitter node ids of the *topmost* nested
    /// definitions the scope walk turned into their own `Function` node (D4).
    /// Every call site is attributed to exactly one node — the nearest
    /// enclosing node-ified scope — instead of being counted twice.
    fn extract_calls(body: Node, source: &[u8], node_ified: &[usize]) -> Vec<(String, u32)> {
        let mut calls: Vec<(String, u32)> = Vec::new();
        fn walk(node: Node, source: &[u8], out: &mut Vec<(String, u32)>, node_ified: &[usize]) {
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
                // D4 — this subtree became its own Function node, so its call
                // sites are that node's, not ours. Checked first because a
                // node-ified literal can sit in a position the rules below
                // would happily descend into: a `function_expression` (not in
                // NESTED_SCOPES at all) or a factory argument such as
                // `Effect.fn("n")(function* () { … })`, which reads as an
                // inline anonymous callback but is a named binding's body.
                if node_ified.contains(&child.id()) {
                    continue;
                }
                let k = child.kind();
                // Inline anonymous functions — callbacks like `.map(x => foo(x))`
                // and JSX handlers like `onClick={() => foo()}` — get no graph
                // node, so their calls belong to the enclosing function. The
                // NESTED_SCOPES skip below is a *conservative* boundary, not a
                // statement that everything behind it is node-ified: before the
                // Phase 3 scope walk it dropped the calls of every nested named
                // binding on the floor, because nothing node-ified those either.
                // The precise "already has its own node" question is answered by
                // `node_ified` above; this stays as the outer bound on how much
                // of an unattributed nested scope we fold into the parent.
                let inline_anon = FN_LITERALS.contains(&k)
                    && matches!(node.kind(), "arguments" | "jsx_expression");
                if inline_anon || !NESTED_SCOPES.contains(&k) {
                    walk(child, source, out, node_ified);
                }
            }
        }
        walk(body, source, &mut calls, node_ified);
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

    #[allow(clippy::too_many_arguments)]
    fn parse_function(
        &self,
        node: Node,
        source: &[u8],
        module_path: &str,
        rel_path: &str,
        is_method: bool,
        owner: Option<&str>,
        node_ified: &[usize],
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
            .map(|b| Self::extract_calls(b, source, node_ified))
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
                    // The method keeps its own identity (`Module.Class.method`,
                    // untagged), but its body is a scope like any other: named
                    // bindings inside it become `Module.Class.method.binding`.
                    let method = Self::get_name(child, source).to_string();
                    let chain = [name.clone(), method];
                    let mut nested = Vec::new();
                    let node_ified = self.descend_scope(
                        child,
                        source,
                        module_path,
                        rel_path,
                        Some(&chain),
                        1,
                        &mut nested,
                    );
                    let fn_info = self.parse_function(
                        child,
                        source,
                        module_path,
                        rel_path,
                        true,
                        Some(&name),
                        &node_ified,
                    );
                    method_rel.methods.push(fn_info.clone());
                    result.functions.push(fn_info);
                    result.functions.extend(nested);
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
                        // An interface member is a signature: no body, so no
                        // scope to descend into.
                        let fn_info = self.parse_function(
                            item,
                            source,
                            module_path,
                            rel_path,
                            true,
                            Some(&name),
                            &[],
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
            // `generator_function_declaration` is the grammar's kind for a
            // top-level `function* g() {}`. Without this arm it matched
            // nothing and produced NO node at all — not even a Constant.
            "function_declaration" | "generator_function_declaration" => {
                let name = Self::get_name(node, source).to_string();
                let chain = [name];
                let mut nested = Vec::new();
                let node_ified = self.descend_scope(
                    node,
                    source,
                    module_path,
                    rel_path,
                    Some(&chain),
                    1,
                    &mut nested,
                );
                let fn_info = self.parse_function(
                    node,
                    source,
                    module_path,
                    rel_path,
                    false,
                    None,
                    &node_ified,
                );
                result.functions.push(fn_info);
                result.functions.extend(nested);
            }
            "class_declaration" => self.parse_class(node, source, module_path, rel_path, result),
            "interface_declaration" => {
                self.parse_interface(node, source, module_path, rel_path, result)
            }
            "enum_declaration" => self.parse_enum(node, source, module_path, rel_path, result),
            // TS `namespace X { … }`. There was no arm for this at all, so
            // every namespace member was invisible (55 namespaces on
            // opencode). Members are export-like: the namespace adds a name
            // segment but not a nesting level.
            "internal_module" | "module" => {
                let mut node_ified = Vec::new();
                self.walk_scope(
                    node,
                    source,
                    module_path,
                    rel_path,
                    Some(&[]),
                    0,
                    &mut result.functions,
                    &mut node_ified,
                );
            }
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
                        | "generator_function_declaration"
                        | "class_declaration"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "internal_module"
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
                    // One shared code path with the nested walk, at depth 0
                    // with an empty scope chain: D1-2's bare function literal
                    // (`() => …`, `function () {}`, `function* () {}`) and
                    // D1-3's narrowed factory unwrap both emit here, the
                    // Constant they used to produce is dropped, and the
                    // binding's body is descended into for nested definitions.
                    // An empty chain qualifies as `module.name`, byte-identical
                    // to what this arm produced before, and `tag_scope` adds
                    // neither property at depth 0.
                    let mut node_ified = Vec::new();
                    if self.walk_declarator(
                        child,
                        source,
                        module_path,
                        rel_path,
                        Some(&[]),
                        0,
                        &mut result.functions,
                        &mut node_ified,
                    ) {
                        continue;
                    }
                    let name_node = child.child_by_field_name("name");
                    let value = child.child_by_field_name("value");
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
        // D2 tie-break, after every definition in the file is known and in
        // source order (the walk emits parent-before-child, so the vector is
        // already ordered by start position).
        break_qualified_name_ties(&mut result.functions);
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

/// Depth-0 higher-order-function bindings and the grammar-vocabulary fixes.
///
/// Two things are pinned here (Phase 2 of
/// `dev-docs/plans/closure-scoped-definitions.md`):
///
///  * the **narrowed factory unwrap** (D1 item 3 as amended by the Phase 1
///    spike) — a `const x = <call-chain>(…)` binding becomes a `Function`
///    only under the three measured guards, and in particular
///    `const names = users.map(u => u.name)` must stay a `Constant`;
///  * the **dead grammar vocabulary** — tree-sitter-typescript 0.23.2 emits
///    `function_expression` / `generator_function` /
///    `generator_function_declaration`, never a bare `function`, so the three
///    reproduced defects D-A / D-B / D-C
///    (`dev-docs/bench/out/nested-spike/grammar-vocabulary-defects.txt`)
///    must all produce `Function` nodes.
#[cfg(test)]
mod hof_binding_tests {
    use super::*;
    use crate::parsers::LanguageParser;

    fn parse_ts(src: &str) -> ParseResult {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.ts");
        std::fs::write(&path, src).expect("write fixture");
        JstsParser::typescript().parse_file(&path, dir.path())
    }

    /// `(name, line_number)` for every Function the parser emitted.
    fn functions(src: &str) -> Vec<(String, u32)> {
        parse_ts(src)
            .functions
            .iter()
            .map(|f| (f.name.clone(), f.line_number))
            .collect()
    }

    fn constant_names(src: &str) -> Vec<String> {
        parse_ts(src)
            .constants
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    /// The `wrapped_by` metadata of the single Function named `name`.
    fn wrapped_by(src: &str, name: &str) -> Option<String> {
        let result = parse_ts(src);
        let f = result
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "no Function named {name}; got {:?}",
                    result.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
                )
            });
        f.metadata
            .get("wrapped_by")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    // ── (a) the narrowed factory unwrap ──────────────────────────────────

    /// The motivating shape: `bom.ts:18`-class Effect-TS bindings. One node,
    /// not two — the `Constant` this used to produce is gone.
    #[test]
    fn curried_generator_wrap_becomes_one_function_node() {
        let src = r#"
export const readFile = Effect.fn("Bom.readFile")(function* (path: string) {
  return yield decode(path)
})
"#;
        assert_eq!(functions(src), vec![("readFile".to_string(), 2)]);
        assert!(
            constant_names(src).is_empty(),
            "the Constant must be dropped, got {:?}",
            constant_names(src)
        );
        assert_eq!(wrapped_by(src, "readFile").as_deref(), Some("Effect.fn"));
    }

    /// A curried application qualifies even when the literal is an arrow —
    /// `f(…)(fn)` is the shape, the generator is only the other half of the
    /// disjunction.
    #[test]
    fn curried_arrow_wrap_becomes_a_function() {
        let src = r#"
export const cached = memoize("cache-key")((n: number) => n + 1)
"#;
        assert_eq!(functions(src), vec![("cached".to_string(), 2)]);
        assert!(constant_names(src).is_empty());
        assert_eq!(wrapped_by(src, "cached").as_deref(), Some("memoize"));
    }

    /// The chain walks call-valued arguments too, and `wrapped_by` reports
    /// the OUTERMOST callee — the layer the binding actually is.
    #[test]
    fn generator_inside_a_call_valued_argument_is_unwrapped() {
        let src = r#"
export const layer = Layer.effect(Service, Effect.gen(function* () {
  return yield make()
}))
"#;
        assert_eq!(functions(src), vec![("layer".to_string(), 2)]);
        assert_eq!(wrapped_by(src, "layer").as_deref(), Some("Layer.effect"));
    }

    /// **The narrowing's regression test.** "Exactly one function literal in
    /// the chain" is true here, and the binding is still an array. Phase 1
    /// measured this shape firing 4 345 times on opencode at a median of
    /// 1 LOC — accepting it is what made D1-3-as-written fail its gate.
    #[test]
    fn array_map_binding_stays_a_constant() {
        let src = r#"
const users = load()
export const names = users.map((u) => u.name)
"#;
        assert!(
            functions(src).is_empty(),
            "arr.map(f) is not a function binding, got {:?}",
            functions(src)
        );
        assert_eq!(constant_names(src), vec!["users", "names"]);
    }

    /// A chained value receiver (`tp.split(',').map`) is rejected for the
    /// same reason, via the `member.expr` shape rather than `member.lower`.
    #[test]
    fn chained_value_receiver_stays_a_constant() {
        let src = r#"
export const parts = raw.split(",").map((s) => s.trim())
"#;
        assert!(functions(src).is_empty(), "got {:?}", functions(src));
        assert_eq!(constant_names(src), vec!["parts"]);
    }

    /// Zero function literals in the chain — an ordinary constant.
    #[test]
    fn zero_function_literal_call_stays_a_constant() {
        let src = r#"
export const config = build(1, "two", { three: 3 })
"#;
        assert!(functions(src).is_empty(), "got {:?}", functions(src));
        assert_eq!(constant_names(src), vec!["config"]);
    }

    /// Two function literals — we cannot say which one the binding *is*, so
    /// the ambiguity guard declines it.
    #[test]
    fn two_function_literal_call_stays_a_constant() {
        let src = r#"
export const pair = combine(function* () { yield 1 }, function* () { yield 2 })
"#;
        assert!(functions(src).is_empty(), "got {:?}", functions(src));
        assert_eq!(constant_names(src), vec!["pair"]);
    }

    /// An uncurried call on an acceptable callee whose literal is a plain
    /// arrow binds the call's *result*, not the arrow. This is the second
    /// half of guard 3 and the reason `createMemo(() => …)` (1 511 hits on
    /// opencode) does not flood the graph.
    #[test]
    fn uncurried_arrow_argument_stays_a_constant() {
        let src = r#"
export const total = createMemo(() => 1 + 2)
"#;
        assert!(functions(src).is_empty(), "got {:?}", functions(src));
        assert_eq!(constant_names(src), vec!["total"]);
    }

    // ── (b) the dead grammar vocabulary ──────────────────────────────────

    /// D-A: `const x = function () {}` — kind `function_expression`.
    #[test]
    fn const_function_expression_is_a_function() {
        let src = r#"
export const asFnExpr = function (n: number): number {
  return n + 1
}
"#;
        assert_eq!(functions(src), vec![("asFnExpr".to_string(), 2)]);
        assert!(constant_names(src).is_empty());
    }

    /// D-B: `const x = function* () {}` — kind `generator_function`.
    #[test]
    fn const_generator_expression_is_a_function() {
        let src = r#"
export const asGenExpr = function* (n: number) {
  yield n
}
"#;
        assert_eq!(functions(src), vec![("asGenExpr".to_string(), 2)]);
        assert!(constant_names(src).is_empty());
    }

    /// D-C: a top-level `function* g() {}` used to produce **no node at
    /// all** — not a Constant, not a Function. Both the exported and the
    /// bare spelling go through different code paths (`export_statement`
    /// child dispatch vs the top-level match), so both are pinned.
    #[test]
    fn top_level_generator_declarations_are_functions() {
        let src = r#"
export function* exportedGen(n: number) {
  yield n
}

function* localGen(n: number) {
  yield n
}
"#;
        assert_eq!(
            functions(src),
            vec![("exportedGen".to_string(), 2), ("localGen".to_string(), 6),]
        );
    }

    /// The exported generator declaration also has to reach `FileInfo.exports`
    /// — the export dispatch is where D-C's second half lived.
    #[test]
    fn exported_generator_declaration_is_recorded_as_an_export() {
        let result = parse_ts("export function* gen() { yield 1 }\n");
        assert_eq!(result.files[0].exports, vec!["gen"]);
    }

    /// Arrow bindings and plain declarations were already correct; this pins
    /// that the vocabulary fix did not disturb them.
    #[test]
    fn arrow_bindings_and_plain_declarations_are_unchanged() {
        let src = r#"
export const asArrow = () => 1
export function asDecl() { return 4 }
export const PLAIN = 42
"#;
        assert_eq!(
            functions(src),
            vec![("asArrow".to_string(), 2), ("asDecl".to_string(), 3)]
        );
        assert_eq!(constant_names(src), vec!["PLAIN"]);
    }
}

/// The nested scope walk: D1 as amended, D2 qualified names + properties,
/// D3's inputs and D4's call attribution (Phase 3 of
/// `dev-docs/plans/closure-scoped-definitions.md`).
#[cfg(test)]
mod closure_scope_tests {
    use super::*;
    use crate::parsers::LanguageParser;

    fn parse_ts(src: &str) -> ParseResult {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.ts");
        std::fs::write(&path, src).expect("write fixture");
        JstsParser::typescript().parse_file(&path, dir.path())
    }

    /// Every emitted Function as `(qualified_name, nesting_depth,
    /// parent_scope)`, in emission order. Both properties are absent (not
    /// zero / not empty) at top level.
    fn scopes(src: &str) -> Vec<(String, Option<u64>, Option<String>)> {
        parse_ts(src)
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

    fn qnames(src: &str) -> Vec<String> {
        parse_ts(src)
            .functions
            .iter()
            .map(|f| f.qualified_name.clone())
            .collect()
    }

    /// The call names attributed to one function, by qualified name.
    fn calls_of(src: &str, qualified_name: &str) -> Vec<String> {
        parse_ts(src)
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
    fn a_nested_binding_is_qualified_by_its_scope_chain() {
        let src = "\
export function outer(n: number) {
  const inner = (x: number) => x + n
  return inner(1)
}
";
        assert_eq!(
            scopes(src),
            vec![
                ("a.outer".to_string(), None, None),
                ("a.outer.inner".to_string(), Some(1), Some("a.outer".into())),
            ]
        );
    }

    #[test]
    fn the_chain_grows_with_every_named_scope() {
        let src = "\
function a() {
  const b = () => {
    function c() {
      return 1
    }
    return c()
  }
  return b()
}
";
        assert_eq!(
            scopes(src),
            vec![
                ("a.a".to_string(), None, None),
                ("a.a.b".to_string(), Some(1), Some("a.a".into())),
                ("a.a.b.c".to_string(), Some(2), Some("a.a.b".into())),
            ]
        );
    }

    /// The motivating Effect-TS shape. The enclosing scope is the *binding*
    /// the closure is assigned to (`layer`), exactly as the Phase 1 spike
    /// emitted `packages/opencode/src/mcp.layer.connectRemote`.
    #[test]
    fn an_effect_service_closure_names_its_members_after_the_binding() {
        let src = "\
export const layer = Layer.effect(
  Service,
  Effect.gen(function* () {
    const connectRemote = Effect.fn(\"connectRemote\")(function* (url: string) {
      return yield fetchIt(url)
    })
    return { connectRemote }
  }),
)
";
        assert_eq!(
            scopes(src),
            vec![
                ("a.layer".to_string(), None, None),
                (
                    "a.layer.connectRemote".to_string(),
                    Some(1),
                    Some("a.layer".into())
                ),
            ]
        );
        let parsed = parse_ts(src);
        let nested = parsed
            .functions
            .iter()
            .find(|f| f.name == "connectRemote")
            .expect("connectRemote");
        assert_eq!(
            nested.metadata.get("wrapped_by").and_then(|v| v.as_str()),
            Some("Effect.fn")
        );
    }

    /// A `namespace` contributes a name segment but NOT a nesting level: its
    /// members are as globally addressable as a top-level export, so they
    /// keep full cross-file CALLS participation under D3.
    #[test]
    fn namespace_members_are_named_but_not_nested() {
        let src = "\
export namespace Text {
  export function widen(v: string): string {
    return v
  }
}
";
        assert_eq!(
            scopes(src),
            vec![("a.Text.widen".to_string(), None, Some("a.Text".into()))]
        );
    }

    #[test]
    fn ambient_module_declarations_are_not_a_namespace_scope() {
        let src = "\
declare module \"pkg\" {
  export function fromAmbient(): void
}
";
        assert_eq!(qnames(src), Vec::<String>::new());
    }

    #[test]
    fn a_class_method_body_is_a_scope() {
        let src = "\
export class Runner {
  run(n: number) {
    const step = (x: number) => x + 1
    return step(n)
  }
}
";
        assert_eq!(
            scopes(src),
            vec![
                // The method itself keeps its plain identity: untagged.
                ("a.Runner.run".to_string(), None, None),
                (
                    "a.Runner.run.step".to_string(),
                    Some(1),
                    Some("a.Runner.run".into())
                ),
            ]
        );
    }

    // ── D1 clause 5: nothing below an anonymous scope ────────────────────

    /// The regression test for the amendment that brought opencode's node
    /// growth from 14.15 % to 10.90 %. The walk must still *descend* into the
    /// callback — that is how it would reach a named binding below one — it
    /// just must not mint a node whose chain passed through it.
    #[test]
    fn a_named_binding_inside_an_anonymous_callback_is_not_a_node() {
        let src = "\
export function outer(items: string[]) {
  items.forEach((item) => {
    const hidden = (label: string) => label.trim()
    hidden(item)
  })
  const visible = (label: string) => label.trim()
  return visible(\"x\")
}
";
        assert_eq!(
            qnames(src),
            vec!["a.outer".to_string(), "a.outer.visible".to_string()]
        );
    }

    #[test]
    fn anonymity_is_absorbing_all_the_way_down() {
        let src = "\
export function outer() {
  run(() => {
    const mid = () => {
      const deep = () => 1
      return deep()
    }
    return mid()
  })
}
";
        assert_eq!(qnames(src), vec!["a.outer".to_string()]);
    }

    // ── D1-3: the narrowing still holds at depth > 0 ─────────────────────

    /// `const x = arr.map(f)` binds an array, not a function — the single
    /// most expensive over-capture the Phase 1 spike found (4 345 hits on
    /// opencode at a median of 1 LOC). Phase 2 pinned it at depth 0; the
    /// scope walk must not reintroduce it at depth 1.
    #[test]
    fn a_value_receiver_call_stays_a_constant_inside_a_closure() {
        let src = "\
export function outer(users: string[]) {
  const names = users.map((u) => u.trim())
  return names
}
";
        assert_eq!(qnames(src), vec!["a.outer".to_string()]);
    }

    /// Declining the *binding* is not declining the *scope*: a helper
    /// declared inside that callback is still addressable through the name
    /// the callback is bound to. This is what the Phase 1 spike counted, and
    /// what the +3 062-node figure includes.
    #[test]
    fn a_declined_factory_binding_still_names_the_scope_below_it() {
        let src = "\
export function outer(users: string[]) {
  const names = users.map((u) => {
    const clean = (s: string) => s.trim()
    return clean(u)
  })
  return names
}
";
        assert_eq!(
            scopes(src),
            vec![
                ("a.outer".to_string(), None, None),
                (
                    "a.outer.names.clean".to_string(),
                    Some(2),
                    Some("a.outer.names".into())
                ),
            ]
        );
    }

    /// A plain IIFE is *not* a factory wrap: its single function literal is
    /// the callee, and D1-3's chain walk covers curried callees and
    /// call-valued arguments only. Pinned so the +3 062-node ceiling — which
    /// counts this shape among the 10 654 opencode "no literal in the chain"
    /// exclusions — is not silently widened.
    /// Follow-up: `dev-docs/plans/iife-module-factory-scope.md`.
    #[test]
    fn a_plain_iife_binding_is_neither_a_function_nor_a_scope() {
        let src = "\
export const registry = (function () {
  function register(name: string) {
    return name
  }
  return { register }
})()
";
        assert_eq!(qnames(src), Vec::<String>::new());
        assert_eq!(
            parse_ts(src)
                .constants
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
            vec!["registry"]
        );
    }

    // ── D2: the duplicate-qualified-name tie-break ───────────────────────

    #[test]
    fn a_duplicate_nested_qualified_name_is_broken_by_line() {
        let src = "\
export function normalize(input: string, upper: boolean) {
  if (upper) {
    const scrub = (raw: string) => raw.toUpperCase()
    return scrub(input)
  } else {
    const scrub = (raw: string) => raw.trimStart()
    return scrub(input)
  }
}
";
        assert_eq!(
            qnames(src),
            vec![
                "a.normalize".to_string(),
                // First occurrence in source order keeps the bare name.
                "a.normalize.scrub".to_string(),
                // Second takes the suffix, at its own line.
                "a.normalize.scrub#6".to_string(),
            ]
        );
    }

    /// Three collisions, so the rule is "second and subsequent", not "swap".
    #[test]
    fn every_repeat_after_the_first_takes_a_suffix() {
        let src = "\
export function f() {
  {
    const g = () => 1
    g()
  }
  {
    const g = () => 2
    g()
  }
  {
    const g = () => 3
    g()
  }
}
";
        assert_eq!(
            qnames(src),
            vec![
                "a.f".to_string(),
                "a.f.g".to_string(),
                "a.f.g#7".to_string(),
                "a.f.g#11".to_string(),
            ]
        );
    }

    /// The suffix is NOT unconditional. Making every qualified name carry a
    /// line number is the fragility D2's amendment removed: an edit anywhere
    /// above a function would move its identity, every CALLS target and every
    /// golden digest with it.
    #[test]
    fn a_unique_nested_qualified_name_carries_no_line_number() {
        let src = "\
export function f() {
  const g = () => 1
  return g()
}
export function h() {
  const g = () => 2
  return g()
}
";
        assert_eq!(
            qnames(src),
            vec![
                "a.f".to_string(),
                "a.f.g".to_string(),
                "a.h".to_string(),
                "a.h.g".to_string(),
            ]
        );
    }

    /// Top-level identities are never rewritten — a collision there predates
    /// the scope walk and is not the walk's to renumber.
    #[test]
    fn top_level_duplicates_are_left_alone() {
        let src = "\
export function dup() {
  return 1
}
export function dup() {
  return 2
}
";
        assert_eq!(qnames(src), vec!["a.dup".to_string(), "a.dup".to_string()]);
    }

    // ── D4: exactly one owner per call site ──────────────────────────────

    /// Before Phase 3 these calls were **dropped**, not mis-attributed:
    /// `extract_calls` skipped the arrow because it is in `NESTED_SCOPES`, on
    /// the theory that named arrows "are node-ified elsewhere" — true only at
    /// top level. Nothing node-ified this one, so `helper`'s calls left no
    /// trace in the graph at all.
    #[test]
    fn calls_inside_a_nested_named_binding_attach_to_it() {
        let src = "\
export function outer(n: number) {
  const helper = (x: number) => {
    return transform(x)
  }
  return helper(n)
}
";
        assert_eq!(calls_of(src, "a.outer.helper"), vec!["transform"]);
        assert_eq!(calls_of(src, "a.outer"), vec!["helper"]);
    }

    /// …and are not counted twice. A `function_expression` is not in
    /// `NESTED_SCOPES`, so before the D4 guard the enclosing scope walked
    /// straight through it.
    #[test]
    fn a_node_ified_function_expression_is_not_double_counted() {
        let src = "\
export function outer(n: number) {
  const helper = function (x: number) {
    return transform(x)
  }
  return helper(n)
}
";
        assert_eq!(calls_of(src, "a.outer.helper"), vec!["transform"]);
        assert_eq!(calls_of(src, "a.outer"), vec!["helper"]);
    }

    /// A factory-wrapped literal sits in argument position, which
    /// `extract_calls` treats as an inline anonymous callback and descends
    /// into. Once the binding is node-ified, the D4 guard has to win.
    #[test]
    fn a_node_ified_factory_argument_is_not_double_counted() {
        let src = "\
export function outer(n: number) {
  const wrapped = Effect.fn(\"w\")(function* (x: number) {
    return yield transform(x)
  })
  return wrapped(n)
}
";
        assert_eq!(calls_of(src, "a.outer.wrapped"), vec!["transform"]);
        assert_eq!(calls_of(src, "a.outer"), vec!["Effect.fn", "wrapped"]);
    }

    /// An anonymous callback is still folded into the enclosing node-ified
    /// scope — D1 keeps it out of the graph, so its calls have to land
    /// somewhere, and the nearest node-ified scope is the honest owner.
    #[test]
    fn anonymous_callback_calls_still_belong_to_the_enclosing_scope() {
        let src = "\
export function outer(items: string[]) {
  items.forEach((item) => {
    transform(item)
  })
}
";
        assert_eq!(calls_of(src, "a.outer"), vec!["items.forEach", "transform"]);
    }
}
