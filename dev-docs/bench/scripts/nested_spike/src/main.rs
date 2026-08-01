//! Phase-1 spike walker for `dev-docs/plans/closure-scoped-definitions.md`.
//!
//! Implements design decision **D1** (named-binding inclusion criterion) and
//! **D2** (scope-chain qualified names with `<anonL{line}>` positional
//! markers) as a *measurement*, not as builder code. Nothing here is shipped:
//! it exists to answer the go/no-go question "does D1 capture behaviour or
//! flood the graph with two-line lambdas?" before Phases 2-4 build on it.
//!
//! It re-implements the walk rather than calling into `codingest` on purpose —
//! the whole point is to model the walk codingest does *not* have yet. What it
//! does NOT re-implement is file discovery: the file list is handed in, taken
//! straight from the File nodes of a real `codingest build`, so the measured
//! population is exactly the population the builder parses.
//!
//! Usage:
//!   nested_spike --repo <abs-root> --files <tsv> --out <json>
//!   nested_spike --probe <file>          # dump the grammar s-expression
//!
//! `--files` is `<relative path>\t<language>` per line, language in
//! {typescript, javascript} (the `.tsx` grammar is selected by extension, the
//! way `JstsParser::parse_file` does it).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tree_sitter::{Node, Parser};

// ── grammar vocabulary ───────────────────────────────────────────────────
//
// Verified against the pinned grammars by `--probe`; see
// `grammar-kinds.txt` in the run's artifact directory.

/// Function *literals* — an expression that evaluates to a function.
const FN_LITERALS: &[&str] = &[
    "arrow_function",
    "function",
    "function_expression",
    "generator_function",
];

/// Function *declarations* — named by grammar, statement position.
const FN_DECLS: &[&str] = &["function_declaration", "generator_function_declaration"];

/// The value kinds `typescript.rs:1015`'s `lexical_declaration` arm accepts
/// today. Anything else at depth 0 becomes a `Constant`. Kept as its own
/// list (rather than reusing `FN_LITERALS`) precisely because the two
/// differing is what this spike measures.
///
/// NOTE the `"function"` entry is *dead vocabulary*: tree-sitter-typescript
/// 0.23.2 emits `function_expression` for `const x = function(){}`, never a
/// bare `function`. It is reproduced verbatim so the spike models the
/// builder's real behaviour, defect included — `const x = function(){}` is a
/// `Constant` today. Verified end-to-end, see
/// `dev-docs/bench/out/nested-spike/grammar-vocabulary-defects.txt`.
const TODAY_BINDING_VALUE_KINDS: &[&str] = &["arrow_function", "function"];

/// Declaration kinds `parse_top_level`'s `match` has an arm for. It has none
/// for `generator_function_declaration`, so a top-level `function* g(){}`
/// produces no node at all today — also a reproduced defect, not an
/// approximation.
const TODAY_DECL_KINDS: &[&str] = &["function_declaration"];

// ── output shape ─────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct Capture {
    file: String,
    name: String,
    qualified_name: String,
    /// D1 bucket: `decl` | `binding` | `factory`.
    bucket: &'static str,
    /// D2 `nesting_depth`: enclosing function-like scopes. 0 == top level.
    depth: u32,
    /// Inside a `namespace X { … }` body (D1 item 4).
    in_namespace: bool,
    /// D2 `parent_scope` — nearest enclosing scope-chain entry, or "".
    parent_scope: String,
    /// The scope chain contains at least one `<anonL…>` marker.
    anon_in_chain: bool,
    start_line: u32,
    end_line: u32,
    /// Line span of the captured function literal/declaration, inclusive.
    loc: u32,
    /// What the current builder emits here: `function` | `constant` | `none`.
    today: &'static str,
    /// Factory wrapper callee text, e.g. `Effect.fn`.
    wrapped_by: Option<String>,
    /// Grammar kind of the captured literal (`arrow_function`,
    /// `function_expression`, `generator_function`) or of the declaration.
    literal_kind: String,
    /// Factory-bucket structural discriminators, so a *narrowed* variant of
    /// the D1-3 rule can be scored from the raw artifact without re-walking.
    /// `None` outside the factory bucket.
    factory: Option<FactoryShape>,
}

/// How the single function literal sits inside the binding's call chain.
/// This is the substrate for narrowing D1-3: `Effect.fn("n")(function*…)`
/// and `arr.map(x => x.y)` both satisfy "exactly one function literal", and
/// only these fields tell them apart.
#[derive(Serialize, Clone)]
struct FactoryShape {
    /// The literal's immediate call has a call-expression callee —
    /// `Effect.fn("n")(fn)`.
    curried: bool,
    /// Callee shape of the literal's immediate call: `identifier`,
    /// `member.Capitalized` (`Effect.fn`, `Layer.effect`), `member.lower`
    /// (`arr.map`, `results.filter`), `member.expr` (`this[k].map`),
    /// `curried`, or `other`.
    callee_shape: &'static str,
    /// Bare callee name of the literal's immediate call (`fn`, `map`, `gen`).
    callee_name: String,
    /// Nesting of call expressions between the binding's value and the
    /// literal. 1 == a direct argument of the outermost call.
    call_depth: u32,
    /// The literal is the last argument of its immediate call.
    last_arg: bool,
}

/// Shapes D1 deliberately excludes, counted so the plan can see what it is
/// leaving on the table (and so a later widening has a baseline).
#[derive(Serialize, Default)]
struct Excluded {
    /// `class` declared inside a function/closure scope.
    nested_class_decls: u64,
    /// `method_definition` inside a nested class.
    nested_methods: u64,
    /// `foo.bar = function(){}` / `module.exports = () => {}`.
    assignment_bound_fns: u64,
    /// `{ run: () => {} }` / `{ run() {} }` object-literal function values.
    object_property_fns: u64,
    /// `const [a, b] = …` / `const {a} = …` whose value is a function-ish.
    destructured_bindings: u64,
    /// Call chains with >= 2 function literals — the ambiguity guard firing.
    factory_multi_literal: u64,
    /// Call chains with 0 function literals — an ordinary constant.
    factory_zero_literal: u64,
    /// `foo(function named(){})` — grammar-named, but an *expression* in
    /// argument position, not a declaration. D1 item 1 says "declarations",
    /// so the spike declines it; counted to size the judgement call.
    named_fn_expressions: u64,
}

#[derive(Serialize)]
struct Report {
    repo: String,
    files_walked: u64,
    files_unreadable: u64,
    captures: Vec<Capture>,
    /// `<anonL{line}>` markers that actually appear in an emitted qname.
    anon_markers_used: Vec<String>,
    /// Every anonymous literal entered, by the syntactic parent that holds it
    /// — the population a marker could be minted for.
    anon_scopes_entered_by_parent: BTreeMap<String, u64>,
    excluded: Excluded,
    /// qnames emitted more than once (D2 determinism / uniqueness check).
    duplicate_qnames: BTreeMap<String, u64>,
}

// ── walk ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Scope {
    Named(String),
    Anon(u32),
}

struct Cx<'a> {
    src: &'a [u8],
    file: &'a str,
    module: &'a str,
    captures: Vec<Capture>,
    anon_used: BTreeSet<String>,
    anon_by_parent: BTreeMap<String, u64>,
    excluded: Excluded,
}

fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[n.byte_range()]).unwrap_or("")
}

fn line(n: Node) -> u32 {
    n.start_position().row as u32 + 1
}

fn is_fn_literal(n: Node) -> bool {
    FN_LITERALS.contains(&n.kind())
}

/// Strip `(expr)`, `expr as T`, `expr!`, `expr satisfies T` down to the
/// expression that actually produces the value.
fn unwrap_expr(mut n: Node) -> Node {
    loop {
        match n.kind() {
            "parenthesized_expression" => match n.named_child(0) {
                Some(c) => n = c,
                None => return n,
            },
            "as_expression" | "satisfies_expression" | "non_null_expression" => {
                match n.named_child(0) {
                    Some(c) => n = c,
                    None => return n,
                }
            }
            _ => return n,
        }
    }
}

/// D1-3 factory unwrap: every function literal reachable through a value's
/// call chain, without descending into any literal's body. Curried callees
/// (`Effect.fn("n")(function*…)`) and call-valued arguments
/// (`Layer.effect(S, Effect.gen(function*…))`) are both in the chain;
/// object/array literals are not (an object argument's methods are not a
/// factory wrap, they are configuration).
fn chain_fn_literals<'t>(value: Node<'t>, out: &mut Vec<Node<'t>>) {
    let v = unwrap_expr(value);
    if is_fn_literal(v) {
        out.push(v);
        return;
    }
    if v.kind() != "call_expression" {
        return;
    }
    if let Some(callee) = v.child_by_field_name("function") {
        let callee = unwrap_expr(callee);
        if callee.kind() == "call_expression" {
            chain_fn_literals(callee, out);
        }
    }
    if let Some(args) = v.child_by_field_name("arguments") {
        let mut c = args.walk();
        for arg in args.named_children(&mut c) {
            chain_fn_literals(arg, out);
        }
    }
}

/// Callee text of the outermost call in a factory chain, e.g. `Effect.fn`.
fn wrapper_name(value: Node, src: &[u8]) -> Option<String> {
    let mut v = unwrap_expr(value);
    // Descend curried callees to the innermost callee — `Effect.fn("x")(fn)`
    // should report `Effect.fn`, not the whole `Effect.fn("x")` text.
    loop {
        if v.kind() != "call_expression" {
            return None;
        }
        let callee = unwrap_expr(v.child_by_field_name("function")?);
        if callee.kind() == "call_expression" {
            v = callee;
            continue;
        }
        return Some(text(callee, src).to_string());
    }
}

/// Classify how `lit` sits inside the binding value `v` — see
/// [`FactoryShape`]. Walks up from the literal to its immediate enclosing
/// call, then reads that call's callee.
fn classify_factory(v: Node, lit: Node, src: &[u8]) -> FactoryShape {
    // Immediate enclosing call, and the call nesting depth from `v`.
    let mut call: Option<Node> = None;
    let mut depth = 0u32;
    let mut n = lit;
    while let Some(p) = n.parent() {
        if p.kind() == "call_expression" {
            if call.is_none() {
                call = Some(p);
            }
            depth += 1;
        }
        if p.id() == v.id() {
            break;
        }
        n = p;
    }
    let Some(call) = call else {
        return FactoryShape {
            curried: false,
            callee_shape: "other",
            callee_name: String::new(),
            call_depth: depth,
            last_arg: false,
        };
    };
    let callee = call.child_by_field_name("function").map(unwrap_expr);
    let (callee_shape, callee_name) = match callee {
        Some(c) => match c.kind() {
            "identifier" => ("identifier", text(c, src).to_string()),
            "call_expression" => ("curried", String::new()),
            "member_expression" => {
                let prop = c
                    .child_by_field_name("property")
                    .map(|p| text(p, src).to_string())
                    .unwrap_or_default();
                let obj = c.child_by_field_name("object").map(unwrap_expr);
                let shape = match obj.map(|o| (o.kind(), text(o, src).to_string())) {
                    Some(("identifier", t)) => {
                        if t.chars().next().is_some_and(|ch| ch.is_uppercase()) {
                            "member.Capitalized"
                        } else {
                            "member.lower"
                        }
                    }
                    Some(_) => "member.expr",
                    None => "other",
                };
                (shape, prop)
            }
            "subscript_expression" => ("subscript", String::new()),
            _ => ("other", String::new()),
        },
        None => ("other", String::new()),
    };
    let last_arg = call
        .child_by_field_name("arguments")
        .and_then(|a| {
            let mut c = a.walk();
            a.named_children(&mut c).last().map(|l| l.id())
        })
        .map(|id| {
            // `lit` may be wrapped (`(fn) as T`), so compare spans too.
            let mut m = lit;
            loop {
                if m.id() == id {
                    return true;
                }
                match m.parent() {
                    Some(p) if p.id() != call.id() => m = p,
                    _ => return false,
                }
            }
        })
        .unwrap_or(false);
    FactoryShape {
        curried: callee_shape == "curried",
        callee_shape,
        callee_name,
        call_depth: depth,
        last_arg,
    }
}

fn qualify(module: &str, chain: &[Scope], name: &str) -> String {
    let mut parts = String::from(module);
    for s in chain {
        parts.push('.');
        match s {
            Scope::Named(n) => parts.push_str(n),
            Scope::Anon(l) => parts.push_str(&format!("<anonL{l}>")),
        }
    }
    parts.push('.');
    parts.push_str(name);
    parts
}

fn parent_scope_of(module: &str, chain: &[Scope]) -> String {
    if chain.is_empty() {
        String::new()
    } else {
        let (last, head) = chain.split_last().unwrap();
        let name = match last {
            Scope::Named(n) => n.clone(),
            Scope::Anon(l) => format!("<anonL{l}>"),
        };
        qualify(module, head, &name)
    }
}

#[allow(clippy::too_many_arguments)]
fn record(
    cx: &mut Cx,
    node: Node,
    name: &str,
    bucket: &'static str,
    chain: &[Scope],
    depth: u32,
    in_namespace: bool,
    today: &'static str,
    wrapped_by: Option<String>,
    factory: Option<FactoryShape>,
) {
    let start = line(node);
    let end = node.end_position().row as u32 + 1;
    let anon_in_chain = chain.iter().any(|s| matches!(s, Scope::Anon(_)));
    for s in chain {
        if let Scope::Anon(l) = s {
            cx.anon_used.insert(format!("{}:{}", cx.file, l));
        }
    }
    cx.captures.push(Capture {
        file: cx.file.to_string(),
        name: name.to_string(),
        qualified_name: qualify(cx.module, chain, name),
        bucket,
        depth,
        in_namespace,
        parent_scope: parent_scope_of(cx.module, chain),
        anon_in_chain,
        start_line: start,
        end_line: end,
        loc: end.saturating_sub(start) + 1,
        today,
        wrapped_by,
        literal_kind: node.kind().to_string(),
        factory,
    });
}

/// `visible` tracks whether the current node sits where today's
/// `parse_file` -> `parse_top_level` walk actually looks: a direct child of
/// the program root, or a child of an `export_statement` in that position.
/// It is NOT the same as `depth == 0` — `if (x) { function f(){} }` at file
/// scope has depth 0 but is invisible to the current parser.
struct Pos {
    depth: u32,
    visible: bool,
    in_namespace: bool,
}

fn visit(cx: &mut Cx, node: Node, chain: &mut Vec<Scope>, pos: &Pos) {
    match node.kind() {
        k if FN_DECLS.contains(&k) => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, cx.src).to_string())
                .unwrap_or_else(|| "<unnamed>".to_string());
            let today = if pos.visible && !pos.in_namespace && TODAY_DECL_KINDS.contains(&k) {
                "function"
            } else {
                "none"
            };
            record(
                cx,
                node,
                &name,
                "decl",
                chain,
                pos.depth,
                pos.in_namespace,
                today,
                None,
                None,
            );
            chain.push(Scope::Named(name));
            descend_body(cx, node, chain, pos.depth + 1, pos.in_namespace);
            chain.pop();
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut c = node.walk();
            let declarators: Vec<Node> = node
                .named_children(&mut c)
                .filter(|n| n.kind() == "variable_declarator")
                .collect();
            for d in declarators {
                visit_declarator(cx, d, chain, pos);
            }
        }
        "export_statement" => {
            // Transparent: `export const x = …` is positionally identical to
            // `const x = …` for both today's walk and D1.
            let mut c = node.walk();
            let kids: Vec<Node> = node.named_children(&mut c).collect();
            for k in kids {
                visit(cx, k, chain, pos);
            }
        }
        "internal_module" | "module" => {
            let name_node = node.child_by_field_name("name");
            let Some(name_node) = name_node else { return };
            // `declare module "pkg"` (ambient, string-named) is not a
            // namespace scope; skip it entirely.
            if name_node.kind() == "string" {
                return;
            }
            let name = text(name_node, cx.src).to_string();
            chain.push(Scope::Named(name));
            let body = node.child_by_field_name("body");
            if let Some(body) = body {
                let mut c = body.walk();
                let kids: Vec<Node> = body.named_children(&mut c).collect();
                let inner = Pos {
                    depth: pos.depth,
                    visible: false,
                    in_namespace: true,
                };
                for k in kids {
                    visit(cx, k, chain, &inner);
                }
            }
            chain.pop();
        }
        "class_declaration" | "class" | "abstract_class_declaration" => {
            if pos.depth > 0 {
                cx.excluded.nested_class_decls += 1;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, cx.src).to_string())
                .unwrap_or_else(|| format!("<anonClassL{}>", line(node)));
            chain.push(Scope::Named(name));
            if let Some(body) = node.child_by_field_name("body") {
                let mut c = body.walk();
                let kids: Vec<Node> = body.named_children(&mut c).collect();
                for k in kids {
                    if k.kind() == "method_definition" {
                        if pos.depth > 0 {
                            cx.excluded.nested_methods += 1;
                        }
                        let mname = k
                            .child_by_field_name("name")
                            .map(|n| text(n, cx.src).to_string())
                            .unwrap_or_else(|| "<method>".to_string());
                        chain.push(Scope::Named(mname));
                        descend_body(cx, k, chain, pos.depth + 1, pos.in_namespace);
                        chain.pop();
                    } else {
                        generic(cx, k, chain, pos.depth, pos.in_namespace);
                    }
                }
            }
            chain.pop();
        }
        k if FN_LITERALS.contains(&k) => {
            // Reached generically: an anonymous literal. D1 refuses it a node;
            // D2 gives it a positional marker so bindings inside it still get
            // a stable qualified name.
            if matches!(k, "function_expression" | "generator_function")
                && node.child_by_field_name("name").is_some()
            {
                cx.excluded.named_fn_expressions += 1;
            }
            enter_anon(cx, node, chain, pos.depth, pos.in_namespace);
        }
        _ => generic(cx, node, chain, pos.depth, pos.in_namespace),
    }
}

fn visit_declarator(cx: &mut Cx, d: Node, chain: &mut Vec<Scope>, pos: &Pos) {
    let name_node = d.child_by_field_name("name");
    let value = d.child_by_field_name("value");
    let Some(value) = value else {
        return;
    };
    let named = name_node.filter(|n| n.kind() == "identifier");
    let Some(name_node) = named else {
        // Destructuring pattern: no single binding name, so D1 declines it.
        let v = unwrap_expr(value);
        let mut lits = Vec::new();
        chain_fn_literals(v, &mut lits);
        if !lits.is_empty() {
            cx.excluded.destructured_bindings += 1;
        }
        generic(cx, value, chain, pos.depth, pos.in_namespace);
        return;
    };
    let name = text(name_node, cx.src).to_string();
    let v = unwrap_expr(value);

    if is_fn_literal(v) {
        let today = if pos.visible
            && !pos.in_namespace
            && TODAY_BINDING_VALUE_KINDS.contains(&v.kind())
        {
            "function"
        } else if pos.visible && !pos.in_namespace {
            // Visible but the value kind is outside the arm's match — today
            // it falls through to the Constant branch.
            "constant"
        } else {
            "none"
        };
        record(
            cx,
            v,
            &name,
            "binding",
            chain,
            pos.depth,
            pos.in_namespace,
            today,
            None,
            None,
        );
        chain.push(Scope::Named(name));
        descend_body(cx, v, chain, pos.depth + 1, pos.in_namespace);
        chain.pop();
        return;
    }

    if v.kind() == "call_expression" {
        let mut lits = Vec::new();
        chain_fn_literals(v, &mut lits);
        match lits.len() {
            1 => {
                let lit = lits[0];
                let today = if pos.visible && !pos.in_namespace {
                    "constant"
                } else {
                    "none"
                };
                record(
                    cx,
                    lit,
                    &name,
                    "factory",
                    chain,
                    pos.depth,
                    pos.in_namespace,
                    today,
                    wrapper_name(v, cx.src),
                    Some(classify_factory(v, lit, cx.src)),
                );
                chain.push(Scope::Named(name));
                descend_body(cx, lit, chain, pos.depth + 1, pos.in_namespace);
                chain.pop();
                // The rest of the chain (other arguments) still holds code.
                visit_siblings_excluding(cx, v, lit, chain, pos.depth, pos.in_namespace);
                return;
            }
            0 => cx.excluded.factory_zero_literal += 1,
            _ => cx.excluded.factory_multi_literal += 1,
        }
    }
    generic(cx, value, chain, pos.depth, pos.in_namespace);
}

/// Walk everything under `root` except the subtree rooted at `skip`.
fn visit_siblings_excluding(
    cx: &mut Cx,
    root: Node,
    skip: Node,
    chain: &mut Vec<Scope>,
    depth: u32,
    in_namespace: bool,
) {
    let mut c = root.walk();
    let kids: Vec<Node> = root.named_children(&mut c).collect();
    for k in kids {
        if k.id() == skip.id() {
            continue;
        }
        if k.byte_range().start <= skip.byte_range().start
            && k.byte_range().end >= skip.byte_range().end
        {
            visit_siblings_excluding(cx, k, skip, chain, depth, in_namespace);
        } else {
            generic(cx, k, chain, depth, in_namespace);
        }
    }
}

/// Enter an anonymous function literal: mint a positional scope marker and
/// keep walking, so named bindings inside it are still captured.
fn enter_anon(cx: &mut Cx, node: Node, chain: &mut Vec<Scope>, depth: u32, in_namespace: bool) {
    let parent_kind = node
        .parent()
        .map(|p| p.kind().to_string())
        .unwrap_or_else(|| "<root>".into());
    *cx.anon_by_parent.entry(parent_kind).or_insert(0) += 1;
    chain.push(Scope::Anon(line(node)));
    descend_body(cx, node, chain, depth + 1, in_namespace);
    chain.pop();
}

/// Walk a function-like's body (and its parameter defaults, which can hold
/// arrow literals) with the given scope already pushed.
fn descend_body(cx: &mut Cx, fnode: Node, chain: &mut Vec<Scope>, depth: u32, in_namespace: bool) {
    let mut c = fnode.walk();
    let kids: Vec<Node> = fnode.named_children(&mut c).collect();
    let pos = Pos {
        depth,
        visible: false,
        in_namespace,
    };
    for k in kids {
        // Skip the identifier/type-parameter chrome; statement_block, an
        // expression body, formal_parameters all get walked.
        match k.kind() {
            "identifier" | "property_identifier" | "type_parameters" | "type_annotation" => {}
            _ => visit(cx, k, chain, &pos),
        }
    }
}

/// Structural pass-through for a node that is neither a scope nor a
/// declaration: recurse into children carrying the same position, except
/// that `visible` is always false below the top level.
fn generic(cx: &mut Cx, node: Node, chain: &mut Vec<Scope>, depth: u32, in_namespace: bool) {
    // Two shapes are counted here rather than captured, because D1 excludes
    // them: assignment-bound functions and object-literal function values.
    match node.kind() {
        "assignment_expression" => {
            if let Some(right) = node.child_by_field_name("right") {
                if is_fn_literal(unwrap_expr(right)) {
                    cx.excluded.assignment_bound_fns += 1;
                }
            }
        }
        "pair" => {
            if let Some(v) = node.child_by_field_name("value") {
                if is_fn_literal(unwrap_expr(v)) {
                    cx.excluded.object_property_fns += 1;
                }
            }
        }
        "method_definition" => {
            if node.parent().map(|p| p.kind()) == Some("object") {
                cx.excluded.object_property_fns += 1;
            }
        }
        _ => {}
    }
    let mut c = node.walk();
    let kids: Vec<Node> = node.named_children(&mut c).collect();
    let pos = Pos {
        depth,
        visible: false,
        in_namespace,
    };
    for k in kids {
        visit(cx, k, chain, &pos);
    }
}

// ── module path (mirrors JstsParser::file_to_module_path) ────────────────

fn file_to_module_path(rel: &str) -> String {
    let mut parts: Vec<String> = rel.split('/').map(str::to_string).collect();
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
        String::new()
    } else {
        parts.join("/")
    }
}

// ── driver ───────────────────────────────────────────────────────────────

fn parser_for(rel: &str, language: &str) -> Parser {
    let mut p = Parser::new();
    let lang = if language == "javascript" {
        tree_sitter_javascript::LANGUAGE.into()
    } else if rel.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    p.set_language(&lang).expect("grammar");
    p
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let Some(path) = arg(&args, "--probe") {
        let src = std::fs::read(&path).expect("read probe file");
        let mut p = parser_for(&path, if path.ends_with(".js") { "javascript" } else { "typescript" });
        let tree = p.parse(&src, None).expect("parse");
        println!("{}", tree.root_node().to_sexp());
        return;
    }

    let repo = PathBuf::from(arg(&args, "--repo").expect("--repo"));
    let files_path = arg(&args, "--files").expect("--files");
    let out_path = arg(&args, "--out").expect("--out");

    let listing = std::fs::read_to_string(&files_path).expect("read --files");
    let mut entries: Vec<(String, String)> = listing
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut it = l.splitn(2, '\t');
            let p = it.next().unwrap_or("").to_string();
            let lang = it.next().unwrap_or("typescript").to_string();
            (p, lang)
        })
        .collect();
    // Path order, not filesystem order — the builder sorts its per-language
    // file list for exactly this reason (`builder/mod.rs`), and the spike's
    // output must be byte-identical run to run.
    entries.sort();
    entries.dedup();

    let mut all_captures: Vec<Capture> = Vec::new();
    let mut anon_used: BTreeSet<String> = BTreeSet::new();
    let mut anon_by_parent: BTreeMap<String, u64> = BTreeMap::new();
    let mut excluded = Excluded::default();
    let mut walked = 0u64;
    let mut unreadable = 0u64;

    for (rel, lang) in &entries {
        let abs: PathBuf = Path::new(&repo).join(rel);
        let Ok(src) = std::fs::read(&abs) else {
            unreadable += 1;
            continue;
        };
        let mut parser = parser_for(rel, lang);
        let Some(tree) = parser.parse(&src, None) else {
            unreadable += 1;
            continue;
        };
        walked += 1;
        let module = file_to_module_path(rel);
        let mut cx = Cx {
            src: &src,
            file: rel,
            module: &module,
            captures: Vec::new(),
            anon_used: BTreeSet::new(),
            anon_by_parent: BTreeMap::new(),
            excluded: Excluded::default(),
        };
        let root = tree.root_node();
        let mut chain: Vec<Scope> = Vec::new();
        let pos = Pos {
            depth: 0,
            visible: true,
            in_namespace: false,
        };
        let mut c = root.walk();
        let kids: Vec<Node> = root.named_children(&mut c).collect();
        for k in kids {
            visit(&mut cx, k, &mut chain, &pos);
        }
        all_captures.extend(cx.captures);
        anon_used.extend(cx.anon_used);
        for (k, v) in cx.anon_by_parent {
            *anon_by_parent.entry(k).or_insert(0) += v;
        }
        excluded.nested_class_decls += cx.excluded.nested_class_decls;
        excluded.nested_methods += cx.excluded.nested_methods;
        excluded.assignment_bound_fns += cx.excluded.assignment_bound_fns;
        excluded.object_property_fns += cx.excluded.object_property_fns;
        excluded.destructured_bindings += cx.excluded.destructured_bindings;
        excluded.factory_multi_literal += cx.excluded.factory_multi_literal;
        excluded.factory_zero_literal += cx.excluded.factory_zero_literal;
        excluded.named_fn_expressions += cx.excluded.named_fn_expressions;
    }

    // Deterministic emission order, independent of walk order.
    all_captures.sort_by(|a, b| {
        (&a.file, a.start_line, &a.qualified_name).cmp(&(&b.file, b.start_line, &b.qualified_name))
    });

    let mut seen: BTreeMap<String, u64> = BTreeMap::new();
    for c in &all_captures {
        *seen.entry(c.qualified_name.clone()).or_insert(0) += 1;
    }
    let duplicate_qnames: BTreeMap<String, u64> =
        seen.into_iter().filter(|(_, n)| *n > 1).collect();

    let report = Report {
        repo: repo.display().to_string(),
        files_walked: walked,
        files_unreadable: unreadable,
        captures: all_captures,
        anon_markers_used: anon_used.into_iter().collect(),
        anon_scopes_entered_by_parent: anon_by_parent,
        excluded,
        duplicate_qnames,
    };
    std::fs::write(&out_path, serde_json::to_string(&report).expect("serialize"))
        .expect("write --out");
    eprintln!(
        "{}: {} files, {} captures -> {}",
        repo.display(),
        report.files_walked,
        report.captures.len(),
        out_path
    );
}
