//! Grammar-drift guard: every node-kind string literal a parser matches on
//! must exist in the grammar that parser compiles against.
//!
//! The extraction layer's failure mode is silence — a `match node.kind() {
//! "some_kind" => … }` arm whose kind a grammar bump renames simply stops
//! firing: no error, no red test, an edge type quietly goes to zero.
//! `Language::id_for_node_kind` returns 0 for a kind the grammar does not
//! define, which makes the class checkable. Provenance: the idea was adapted
//! architecture-only from an external suggestion (mcp-servers, 2026-08-15)
//! that cited an AGPL project; no code from that project was read or used —
//! this implementation is written entirely against tree-sitter's public API.
//!
//! **Stated limit (part of the gate, not a caveat):** this guard checks that
//! matched literals EXIST; it cannot see an ABSENT match arm. The
//! `use_as_clause` omission (import report, finding 2) would NOT have been
//! caught here — `use_as_clause` was always a valid kind; the arm was simply
//! missing. That half of the silence class is closed by corpus coverage
//! (`tests/corpus/*_import`), not by this test.
//!
//! Extraction parses each parser source with `syn` and walks the AST, so a
//! literal's syntactic ROLE is known exactly rather than guessed from
//! surrounding text. The guarded shapes, in all cases keyed on the compared
//! expression mentioning `.kind()` (or being a local bound directly from one):
//!   * `match <expr>.kind() { "a" | "b" => … }` — every string literal in
//!     every arm PATTERN, including `|`-alternations and literals nested in
//!     tuple/enum patterns (`match c.map(|c| (c, c.kind())) { Some((_,
//!     "identifier")) => … }`). Arm BODIES are never read, which is why this
//!     is exact where a regex was not.
//!   * `let kind = node.kind(); match kind { "a" => … }` — a local bound by a
//!     bare `.kind()` call is tracked per lexical scope. The initializer must
//!     be exactly that call: `let kind = if n.kind() == "x" { "a" } else
//!     { "b" }` binds a node TYPE name, not a node kind, and is not tracked.
//!   * `matches!(<expr mentioning .kind()>, "a" | "b")` alternation literals,
//!     including the nested spelling `matches!(c.map(|c| c.kind()),
//!     Some("call_expression"))` that the previous regex could not reach.
//!   * `<expr>.kind() == "lit"` / `!=`, either operand order.
//!
//! plus `child_by_field_name("name")` probed via `field_id_for_name`.
//!
//! A `match` whose scrutinee mentions an identifier containing "kind" but
//! which none of the rules above can attribute to a node kind (say
//! `match constant.kind.as_str()`) is SKIPPED and COUNTED, never guessed at —
//! the per-file skip count is reported alongside the guarded counts so a new
//! unattributable shape is visible rather than silently unguarded.
//!
//! History: extraction was regex-based until 2026-08-22. The regexes covered
//! `matches!` alternations and `==`/`!=` comparisons only; `match` arms were
//! explicitly out of scope, and that is exactly where the C# using-alias and
//! PHP grouped-use defects lived, along with two arms left dead by grammar
//! bumps (java's `comment` split in tree-sitter 0.23, go's `method_spec`
//! rename in 0.25). Moving to `syn` took the guarded literal count from 223 to
//! 423 and closed that gap — and the first run of the new extractor found
//! three more dead literals the regexes could not see (`cpp.rs`
//! `scoped_identifier`, `csharp.rs` `parameter_modifier` /
//! `equals_value_clause`).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use syn::visit::Visit;

/// Parser file → the grammars its literals must exist in. A literal passes if
/// ANY of the file's grammars defines it (typescript.rs serves TS + TSX + JS;
/// cpp.rs handles C and C++ headers).
fn grammars_for(file: &str) -> Option<Vec<tree_sitter::Language>> {
    let langs: Vec<tree_sitter::Language> = match file {
        "rust_lang.rs" => vec![tree_sitter_rust::LANGUAGE.into()],
        "python.rs" => vec![tree_sitter_python::LANGUAGE.into()],
        "typescript.rs" => vec![
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            tree_sitter_javascript::LANGUAGE.into(),
        ],
        "cpp.rs" => vec![
            tree_sitter_cpp::LANGUAGE.into(),
            tree_sitter_c::LANGUAGE.into(),
        ],
        "go.rs" => vec![tree_sitter_go::LANGUAGE.into()],
        "java.rs" => vec![tree_sitter_java::LANGUAGE.into()],
        "csharp.rs" => vec![tree_sitter_c_sharp::LANGUAGE.into()],
        "swift.rs" => vec![tree_sitter_swift::LANGUAGE.into()],
        "php.rs" => vec![tree_sitter_php::LANGUAGE_PHP.into()],
        "html.rs" => vec![tree_sitter_html::LANGUAGE.into()],
        "css.rs" => vec![tree_sitter_css::LANGUAGE.into()],
        "dart.rs" => vec![tree_sitter_dart::LANGUAGE.into()],
        "r.rs" => vec![tree_sitter_r::LANGUAGE.into()],
        "julia.rs" => vec![tree_sitter_julia::LANGUAGE.into()],
        // agc.rs: hand-rolled assembly parser, no tree-sitter grammar.
        // shared.rs/mod.rs/registry.rs: no grammar of their own; any
        // .kind() literals there belong to callers' grammars and are
        // guarded at the calling parser's file.
        _ => return None,
    };
    Some(langs)
}

/// One extracted literal: the text and the source line it sits on, so a
/// failure names a place to go rather than only a string to grep for.
type Hit = (String, usize);

/// `<recv>.kind()` — a bare, argument-less `kind()` call. This exact shape is
/// what licenses treating a compared literal as a node kind.
fn is_kind_call(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::MethodCall(call)
        if call.method == "kind" && call.args.is_empty())
}

/// Does `expr` contain a `.kind()` call anywhere inside it? Covers the direct
/// `child.kind()` scrutinee and the wrapped `callee.map(|c| c.kind())` one.
fn mentions_kind_call(expr: &syn::Expr) -> bool {
    #[derive(Default)]
    struct Probe(bool);
    impl<'ast> Visit<'ast> for Probe {
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if node.method == "kind" && node.args.is_empty() {
                self.0 = true;
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    let mut probe = Probe::default();
    probe.visit_expr(expr);
    probe.0
}

/// Does `expr` name any identifier containing "kind"? Used only to decide
/// whether an UNATTRIBUTED match is worth reporting as skipped — it is a
/// reporting heuristic, never a reason to extract anything.
fn mentions_kind_ident(expr: &syn::Expr) -> bool {
    #[derive(Default)]
    struct Probe(bool);
    impl<'ast> Visit<'ast> for Probe {
        fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
            if ident.to_string().contains("kind") {
                self.0 = true;
            }
        }
    }
    let mut probe = Probe::default();
    probe.visit_expr(expr);
    probe.0
}

/// The string literal an expression IS, if it is one.
fn as_str_lit(expr: &syn::Expr) -> Option<Hit> {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Str(s) => Some((s.value(), s.span().start().line)),
            _ => None,
        },
        _ => None,
    }
}

/// Every string literal inside a pattern, at any nesting depth: `|`
/// alternations, tuples, tuple structs (`Some((_, "identifier"))`), struct
/// patterns, references, slices. Arm bodies are never visited.
fn pattern_literals(pat: &syn::Pat, out: &mut Vec<Hit>) {
    struct Collect<'a>(&'a mut Vec<Hit>);
    impl<'ast> Visit<'ast> for Collect<'_> {
        fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
            self.0.push((lit.value(), lit.span().start().line));
        }
    }
    Collect(out).visit_pat(pat);
}

/// `matches!(<scrutinee>, <pattern> [if <guard>] [,])`.
fn parse_matches_body(mac: &syn::Macro) -> Option<(syn::Expr, syn::Pat)> {
    mac.parse_body_with(|input: syn::parse::ParseStream| {
        let scrutinee: syn::Expr = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        let pat = syn::Pat::parse_multi_with_leading_vert(input)?;
        if input.peek(syn::Token![if]) {
            input.parse::<syn::Token![if]>()?;
            let _guard: syn::Expr = input.parse()?;
        }
        if input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
        }
        Ok((scrutinee, pat))
    })
    .ok()
}

/// AST walker collecting node-kind and field-name literals out of one parser
/// source. `scopes` is a lexical stack of locals bound directly from
/// `.kind()`, so `match kind { … }` is attributable in the block that bound
/// `kind` and nowhere else.
#[derive(Default)]
struct KindExtractor {
    kinds: Vec<Hit>,
    fields: Vec<Hit>,
    /// Match scrutinees that look kind-related but no rule could attribute.
    skipped: Vec<usize>,
    scopes: Vec<HashSet<String>>,
}

impl KindExtractor {
    /// Is `expr` a bare path to a local currently bound from a `.kind()` call?
    fn is_kind_local(&self, expr: &syn::Expr) -> bool {
        let syn::Expr::Path(path) = expr else {
            return false;
        };
        let Some(ident) = path.path.get_ident() else {
            return false;
        };
        let name = ident.to_string();
        self.scopes.iter().any(|scope| scope.contains(&name))
    }

    fn attributable(&self, expr: &syn::Expr) -> bool {
        mentions_kind_call(expr) || self.is_kind_local(expr)
    }

    /// Run a nested extraction over an OWNED expression (a `matches!` body,
    /// which `syn::visit` cannot reach because it never descends into macro
    /// token streams) and fold its hits back in. The nested run inherits the
    /// current scope stack so `matches!(kind, "a")` resolves the same way.
    fn descend_owned(&mut self, expr: &syn::Expr) {
        let mut sub = KindExtractor {
            scopes: self.scopes.clone(),
            ..Default::default()
        };
        sub.visit_expr(expr);
        self.kinds.append(&mut sub.kinds);
        self.fields.append(&mut sub.fields);
        self.skipped.append(&mut sub.skipped);
    }
}

impl<'ast> Visit<'ast> for KindExtractor {
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.scopes.push(HashSet::new());
        for stmt in &block.stmts {
            // Bind before descending, so a `match` later in the same block
            // sees the binding and one in an earlier statement does not.
            if let syn::Stmt::Local(local) = stmt {
                if let (syn::Pat::Ident(ident), Some(init)) = (&local.pat, &local.init) {
                    if init.diverge.is_none() && is_kind_call(&init.expr) {
                        self.scopes
                            .last_mut()
                            .expect("scope pushed above")
                            .insert(ident.ident.to_string());
                    }
                }
            }
            self.visit_stmt(stmt);
        }
        self.scopes.pop();
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        if self.attributable(&node.expr) {
            for arm in &node.arms {
                pattern_literals(&arm.pat, &mut self.kinds);
            }
        } else if mentions_kind_ident(&node.expr) {
            self.skipped.push(node.match_token.span.start().line);
        }
        syn::visit::visit_expr_match(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        if matches!(node.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_)) {
            if let Some(hit) = as_str_lit(&node.right) {
                if self.attributable(&node.left) {
                    self.kinds.push(hit);
                }
            }
            if let Some(hit) = as_str_lit(&node.left) {
                if self.attributable(&node.right) {
                    self.kinds.push(hit);
                }
            }
        }
        syn::visit::visit_expr_binary(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "child_by_field_name" && node.args.len() == 1 {
            if let Some(hit) = as_str_lit(&node.args[0]) {
                self.fields.push(hit);
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("matches") {
            if let Some((scrutinee, pat)) = parse_matches_body(mac) {
                if self.attributable(&scrutinee) {
                    pattern_literals(&pat, &mut self.kinds);
                }
                // The scrutinee itself can hold guarded shapes (a nested
                // `.kind() == "x"`); syn stops at the macro boundary, so walk
                // the parsed expression explicitly.
                self.descend_owned(&scrutinee);
            }
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Extracted from one parser source: node kinds, field names, and the count of
/// match scrutinees deliberately left unattributed.
struct Extracted {
    kinds: Vec<Hit>,
    fields: Vec<Hit>,
    skipped: usize,
}

/// Pull the guarded literal shapes out of one parser source.
fn extract_literals(source: &str) -> Extracted {
    let file = syn::parse_file(source).expect("parser source must parse as Rust");
    let mut extractor = KindExtractor::default();
    extractor.visit_file(&file);

    // Dedup by literal text, keeping the first line each was seen on.
    let dedup = |mut hits: Vec<Hit>| -> Vec<Hit> {
        hits.sort();
        let mut seen: HashSet<String> = HashSet::new();
        hits.retain(|(text, _)| seen.insert(text.clone()));
        hits
    };
    Extracted {
        kinds: dedup(extractor.kinds),
        fields: dedup(extractor.fields),
        skipped: extractor.skipped.len(),
    }
}

#[test]
fn every_matched_node_kind_exists_in_its_grammar() {
    let parsers_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/parsers");
    let mut offenders: Vec<String> = Vec::new();
    let mut guarded: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();

    for entry in std::fs::read_dir(&parsers_dir).expect("read parsers dir") {
        let path = entry.expect("dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(grammars) = grammars_for(name) else {
            continue;
        };
        let source = std::fs::read_to_string(&path).expect("read parser source");
        let extracted = extract_literals(&source);
        guarded.insert(
            name.to_string(),
            (
                extracted.kinds.len(),
                extracted.fields.len(),
                extracted.skipped,
            ),
        );

        for (kind, line) in &extracted.kinds {
            // A kind passes if ANY of the file's grammars defines it, named
            // or anonymous.
            let known = grammars.iter().any(|g| {
                g.id_for_node_kind(kind, true) != 0 || g.id_for_node_kind(kind, false) != 0
            });
            if !known {
                offenders.push(format!(
                    "{name}:{line}: node kind {kind:?} unknown to its grammar(s)"
                ));
            }
        }
        for (field, line) in &extracted.fields {
            let known = grammars
                .iter()
                .any(|g| g.field_id_for_name(field).is_some());
            if !known {
                offenders.push(format!(
                    "{name}:{line}: field name {field:?} unknown to its grammar(s)"
                ));
            }
        }
    }

    // The guard must actually be guarding something — an extractor that
    // silently matches nothing is this test's own vacuity failure mode. The
    // floor is well under the 423 the syn walker finds today, so ordinary
    // parser edits never trip it; a collapse to the pre-syn regex level (223)
    // or below does. Per-file entries are (kinds, fields, skipped scrutinees).
    let total_kinds: usize = guarded.values().map(|(k, _, _)| k).sum();
    assert!(
        total_kinds >= 350,
        "extraction found only {total_kinds} kind literals across {} parsers — \
         the syn walker has gone stale against the source style, and the guard \
         is no longer guarding. Guarded files: {guarded:?}",
        guarded.len()
    );

    assert!(
        offenders.is_empty(),
        "node-kind/field literals unknown to their grammars — either a grammar \
         bump renamed them (the match arm is now dead and its edge type is \
         silently zero) or a literal is misspelled:\n{}\n\
         (guarded per file, as (kinds, fields, skipped scrutinees): {guarded:?})",
        offenders.join("\n")
    );
}
