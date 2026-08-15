//! Grammar-drift guard: every node-kind string literal a parser matches on
//! must exist in the grammar that parser compiles against.
//!
//! The extraction layer's failure mode is silence — a `matches!(node.kind(),
//! "some_kind" | …)` arm whose kind a grammar bump renames simply stops
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
//! Extraction is deliberately narrow — two reliably-greppable shapes:
//!   * `matches!(<expr>.kind(), "a" | "b" | …)` alternation literals
//!   * `<expr>.kind() == "lit"` / `"lit" == <expr>.kind()` comparisons
//!
//! plus `child_by_field_name("name")` probed via `field_id_for_name`.
//!
//! `match <expr>.kind() { "lit" => … }` arms are NOT extracted (arm-literal
//! regexes over bodies containing arbitrary strings false-positive); a kind
//! matched only that way is outside this guard, and moving it into a
//! `matches!` is the cheap way to bring it under guard.

use std::collections::BTreeMap;
use std::path::Path;

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

/// Pull the guarded literal shapes out of one parser source.
fn extract_literals(source: &str) -> (Vec<String>, Vec<String>) {
    let mut kinds = Vec::new();
    let mut fields = Vec::new();

    // matches!(<expr>.kind(), "a" | "b" | ...) — capture the alternation.
    // Non-greedy to the closing paren of the matches! bang.
    let matches_re =
        regex::Regex::new(r#"matches!\s*\(\s*[^,]*\.kind\(\)\s*,\s*([^)]+)\)"#).unwrap();
    let lit_re = regex::Regex::new(r#""([A-Za-z_][A-Za-z0-9_]*)""#).unwrap();
    for cap in matches_re.captures_iter(source) {
        for lit in lit_re.captures_iter(&cap[1]) {
            kinds.push(lit[1].to_string());
        }
    }
    // <expr>.kind() == "lit"  /  "lit" == <expr>.kind()
    let eq_re = regex::Regex::new(r#"\.kind\(\)\s*==\s*"([A-Za-z_][A-Za-z0-9_]*)""#).unwrap();
    for cap in eq_re.captures_iter(source) {
        kinds.push(cap[1].to_string());
    }
    let eq_rev_re =
        regex::Regex::new(r#""([A-Za-z_][A-Za-z0-9_]*)"\s*==\s*[^;]*\.kind\(\)"#).unwrap();
    for cap in eq_rev_re.captures_iter(source) {
        kinds.push(cap[1].to_string());
    }
    // child_by_field_name("name")
    let field_re =
        regex::Regex::new(r#"child_by_field_name\(\s*"([A-Za-z_][A-Za-z0-9_]*)"\s*\)"#).unwrap();
    for cap in field_re.captures_iter(source) {
        fields.push(cap[1].to_string());
    }
    kinds.sort();
    kinds.dedup();
    fields.sort();
    fields.dedup();
    (kinds, fields)
}

#[test]
fn every_matched_node_kind_exists_in_its_grammar() {
    let parsers_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/parsers");
    let mut offenders: Vec<String> = Vec::new();
    let mut guarded: BTreeMap<String, (usize, usize)> = BTreeMap::new();

    for entry in std::fs::read_dir(&parsers_dir).expect("read parsers dir") {
        let path = entry.expect("dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(grammars) = grammars_for(name) else {
            continue;
        };
        let source = std::fs::read_to_string(&path).expect("read parser source");
        let (kinds, fields) = extract_literals(&source);
        guarded.insert(name.to_string(), (kinds.len(), fields.len()));

        for kind in &kinds {
            // A kind passes if ANY of the file's grammars defines it, named
            // or anonymous.
            let known = grammars.iter().any(|g| {
                g.id_for_node_kind(kind, true) != 0 || g.id_for_node_kind(kind, false) != 0
            });
            if !known {
                offenders.push(format!(
                    "{name}: node kind {kind:?} unknown to its grammar(s)"
                ));
            }
        }
        for field in &fields {
            let known = grammars
                .iter()
                .any(|g| g.field_id_for_name(field).is_some());
            if !known {
                offenders.push(format!(
                    "{name}: field name {field:?} unknown to its grammar(s)"
                ));
            }
        }
    }

    // The guard must actually be guarding something — an extraction regex
    // that silently matches nothing is this test's own vacuity failure mode.
    let total_kinds: usize = guarded.values().map(|(k, _)| k).sum();
    assert!(
        total_kinds >= 100,
        "extraction found only {total_kinds} kind literals across {} parsers — \
         the regexes have gone stale against the source style, and the guard \
         is no longer guarding. Guarded files: {guarded:?}",
        guarded.len()
    );

    assert!(
        offenders.is_empty(),
        "node-kind/field literals unknown to their grammars — either a grammar \
         bump renamed them (the match arm is now dead and its edge type is \
         silently zero) or a literal is misspelled:\n{}",
        offenders.join("\n")
    );
}
