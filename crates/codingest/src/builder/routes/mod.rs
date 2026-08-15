//! Web-framework route extraction.
//!
//! Recognizes URL-routing patterns across web frameworks and synthesizes
//! `Route` nodes linked to handler `Function`s via `HANDLES` edges. The
//! per-framework files under this directory each implement a single
//! `detect(parse) -> (Vec<RouteNode>, Vec<RouteEdge>)` entry point.
//!
//! Direction: `Route -[HANDLES]-> Function`. Reads naturally as "this
//! route handles via that function", so the typical query
//!
//! ```cypher
//! MATCH (r:Route)-[:HANDLES]->(f:Function) WHERE r.path STARTS WITH '/api'
//! RETURN r.method, r.path, f.qualified_name
//! ```
//!
//! returns one row per endpoint.
//!
//! Frameworks shipped in 0.9.34:
//!   - **Flask** — `@app.route(...)`, `@blueprint.route(...)`, method-shortcuts
//!     `@app.get(...)`, `@app.post(...)`, etc.
//!   - **FastAPI** — `@app.get(...)`, `@router.post(...)`, all HTTP verbs.
//!   - **Django** — `urlpatterns = [path('users/', view)]` in `urls.py`.
//!
//! Express, Axum, Rails, Spring, Laravel and the rest of CodeGraph's
//! framework list need parser-side capture of call-arguments (e.g.
//! `app.get('/x', handler)` in TS) which `FunctionInfo.calls` doesn't
//! preserve today. They land as follow-up PRs once the parser model
//! gains a `function_calls_with_args` channel; adding each new framework
//! after that is one new file in this directory plus a line below.

use crate::models::{ConstantInfo, FileInfo, FunctionInfo};
use std::collections::HashSet;

mod django;
mod fastapi;
mod flask;

/// A discovered route **registration** — not a URL. The graph stores one
/// Route node per `(framework, method, path, declaring file)`, so the same
/// path registered from two files is two nodes, each reporting its own
/// truthful `file_path`/`line_number`. (Keying on the `(framework, method,
/// path)` triple alone made every methodless `@app.route('/')` in a repo one
/// node, whose source location described whichever file the sorted walk
/// reached first and mislocated all the rest.)
///
/// Within one file the triple is still the identity: two handlers registering
/// the same method+path there are one registration site with parallel HANDLES
/// edges, which is also the shape of legitimate stacked decorators
/// (`@app.get @app.post` on the same fn — those already differ by method).
///
/// Consumers match routes by the `path` PROPERTY, never by parsing the id —
/// cross-language `CALLS_SERVICE` linking does exactly that (`cross_lang.rs`),
/// so a client call to a path with N registrations now links to all N.
#[derive(Debug)]
pub struct RouteNode {
    /// Stable id — `"{FRAMEWORK}::{METHOD}::{PATH}::{FILE_PATH}"`
    /// (e.g. `"flask::GET::/users/{id}::app/views.py"`).
    pub id: String,
    /// Display name — the URL path (e.g. `"/users/{id}"`).
    pub name: String,
    /// URL path or pattern, framework-native syntax preserved.
    pub path: String,
    /// HTTP method or `"ANY"` for `@app.route(...)` without `methods=`.
    pub method: String,
    /// `"flask"` | `"fastapi"` | `"django"` (and more later).
    pub framework: String,
    /// File where the route is declared.
    pub file_path: String,
    /// Source line of the declaration.
    pub line_number: u32,
}

/// `Route -[HANDLES]-> Function` — links a route to the function that
/// handles its requests. Multiple routes can point at the same handler
/// (`@app.get @app.post on same fn`) and one handler can be hit by
/// multiple routes (no uniqueness constraint).
#[derive(Debug)]
pub struct RouteEdge {
    pub route_id: String,
    pub function_qname: String,
}

/// Run every registered framework detector over the parse result and
/// concatenate their outputs. One node is retained per route id while distinct
/// handler edges survive, matching the registration identity contract above.
/// Since the id now carries the declaring file, this dedup collapses only true
/// duplicates — two registrations of the same method+path in the same file, and
/// exact duplicate decorators (which collapse to one edge as well). Cross-file
/// registrations of one path each keep their own node and source location.
///
/// **One registration is one Route.** The ambiguous decorator shape
/// `@app.<verb>('/x')` is legal in both Flask (2.0 method shortcuts) and
/// FastAPI; the id carries the framework, so letting both detectors claim it
/// minted two Route nodes for a single registration. Ownership is settled by
/// per-file import evidence (`fastapi_import_evidence`): a file that imports
/// `fastapi` hands the shape to the FastAPI detector, every other file to
/// Flask. `@app.route`/`@bp.route` stays Flask-only and `@router.<verb>` /
/// `@api_router.<verb>` stays FastAPI-only — those shapes were never
/// ambiguous.
pub fn build_routes(
    files: &[FileInfo],
    functions: &[FunctionInfo],
    constants: &[ConstantInfo],
) -> (Vec<RouteNode>, Vec<RouteEdge>) {
    let fastapi_files = fastapi_import_evidence(files);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (det_nodes, det_edges) in [
        flask::detect(functions, &fastapi_files),
        fastapi::detect(functions, &fastapi_files),
        django::detect(constants, functions),
    ] {
        nodes.extend(det_nodes);
        edges.extend(det_edges);
    }
    let mut node_ids = HashSet::new();
    nodes.retain(|node| node_ids.insert(node.id.clone()));
    let mut edge_ids = HashSet::new();
    edges.retain(|edge| edge_ids.insert((edge.route_id.clone(), edge.function_qname.clone())));
    (nodes, edges)
}

/// Files whose imports name `fastapi` — the evidence that settles which
/// framework owns the ambiguous `@app.<verb>(...)` decorator shape.
///
/// Python import strings are dotted origins (`"fastapi"`,
/// `"fastapi.FastAPI"`, `"fastapi.responses.JSONResponse"`), so membership is
/// decided on the first dotted segment. Relative imports (leading `.`) have an
/// empty first segment and can never match.
fn fastapi_import_evidence(files: &[FileInfo]) -> HashSet<&str> {
    files
        .iter()
        .filter(|file| {
            file.imports
                .iter()
                .any(|import| import.split('.').next() == Some("fastapi"))
        })
        .map(|file| file.path.as_str())
        .collect()
}

// ── Per-framework shared helpers ────────────────────────────────────

/// Decorator parser: split `"app.route('/x', methods=['GET'])"` into
/// `("app.route", "'/x', methods=['GET']")`. Returns `None` if there's
/// no call-syntax (`@property` etc.).
pub(super) fn split_decorator(raw: &str) -> Option<(&str, &str)> {
    let open = raw.find('(')?;
    let close = raw.rfind(')')?;
    if close < open {
        return None;
    }
    Some((raw[..open].trim(), &raw[open + 1..close]))
}

/// Extract the first positional string-literal argument from a decorator
/// arg-list. Walks `"'/users', methods=['GET']"` and returns `"/users"`.
/// Handles single and double quotes; ignores f-string prefixes since path
/// patterns are almost always plain literals.
pub(super) fn first_string_literal(args: &str) -> Option<String> {
    let bytes = args.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            // Newlines are whitespace here: the dominant real-world FastAPI
            // style breaks the line after `(` (long arg lists,
            // `response_model=`), and treating `\n` as "not a string literal"
            // silently dropped every such registration — 20 of 147 on the
            // first real-repo acceptance run (2026-08-15). The comment below
            // always said "whitespace"; the code now agrees.
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            let quote = b;
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != quote {
                // Skip simple `\<x>` escapes — path literals rarely have them
                // but this keeps the scan correct for `'\'`.
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                    continue;
                }
                j += 1;
            }
            if j < bytes.len() {
                return Some(std::str::from_utf8(&bytes[start..j]).ok()?.to_string());
            }
            return None;
        }
        // Anything that isn't whitespace or a quote means the first arg is
        // not a string literal (e.g. a variable, an f-string, a list).
        return None;
    }
    None
}

/// Find the value of a keyword argument like `methods=['GET', 'POST']`
/// in a decorator arg-list. Returns the raw string between the brackets
/// or after `=`, unstripped. Used to extract Flask's `methods=` and
/// `methods=` arguments.
pub(super) fn keyword_arg<'a>(args: &'a str, key: &str) -> Option<&'a str> {
    // Naive but sufficient for the shapes we care about: split at the
    // key, ensure preceded by start or `,`, look for `=`, then scan
    // until top-level comma respecting `[]`/`()`/quotes.
    let pat = format!("{key}=");
    let mut start = 0;
    while let Some(rel) = args[start..].find(&pat) {
        let abs = start + rel;
        let preceding_ok = abs == 0 || {
            let b = args.as_bytes()[abs - 1];
            b == b' ' || b == b','
        };
        if !preceding_ok {
            start = abs + 1;
            continue;
        }
        let after = abs + pat.len();
        // Scan until top-level comma.
        let bytes = args.as_bytes();
        let mut depth = 0i32;
        let mut in_quote: Option<u8> = None;
        let mut j = after;
        while j < bytes.len() {
            let c = bytes[j];
            if let Some(q) = in_quote {
                if c == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                    continue;
                }
                if c == q {
                    in_quote = None;
                }
            } else {
                match c {
                    b'\'' | b'"' => in_quote = Some(c),
                    b'[' | b'(' | b'{' => depth += 1,
                    b']' | b')' | b'}' => depth -= 1,
                    b',' if depth == 0 => break,
                    _ => {}
                }
            }
            j += 1;
        }
        return Some(args[after..j].trim());
    }
    None
}

/// Parse a Python collection-of-strings literal into owned uppercased
/// strings. Accepts list `['GET', 'POST']`, tuple `('GET', 'POST')`, and
/// bare-string `'GET'` forms — Flask's own tutorial uses the tuple
/// shape, so without it the parens leak into the parsed methods.
pub(super) fn parse_methods_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .or_else(|| trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')))
        .unwrap_or(trimmed);
    let mut out = Vec::new();
    for piece in inner.split(',') {
        let p = piece.trim().trim_matches(['\'', '"']);
        if !p.is_empty() {
            out.push(p.to_ascii_uppercase());
        }
    }
    out
}

/// Stable Route id used as both the node id and the source side of the
/// HANDLES edge. Keeps a parsable shape if anyone wants to split it.
///
/// The declaring file is part of the identity — a Route node is a
/// *registration*, not a URL (see `RouteNode`). Without it, every methodless
/// `@app.route('/')` in a repo collapsed into one node.
///
/// The file, and deliberately not the line: two registrations of the same
/// method+path within one file are one registration-site family, while a
/// line-bearing id would churn whenever an unrelated line is inserted above
/// the decorator. (Line would not even disambiguate Django, whose entries all
/// share the `urlpatterns` constant's line.)
pub(super) fn make_route_id(framework: &str, method: &str, path: &str, file_path: &str) -> String {
    format!("{framework}::{method}::{path}::{file_path}")
}

#[cfg(test)]
mod tests {
    use super::{build_routes, parse_methods_list};
    use crate::models::{FileInfo, FunctionInfo};

    /// A Python function carrying one route decorator.
    fn handler(qname: &str, file: &str, line: u32, decorator: &str) -> FunctionInfo {
        FunctionInfo {
            name: qname.rsplit('.').next().unwrap_or(qname).to_string(),
            qualified_name: qname.to_string(),
            file_path: file.to_string(),
            line_number: line,
            decorators: vec![decorator.to_string()],
            ..FunctionInfo::default()
        }
    }

    /// A parsed file carrying only the import evidence the detectors read.
    fn file_with_imports(path: &str, imports: &[&str]) -> FileInfo {
        FileInfo {
            path: path.to_string(),
            imports: imports.iter().map(|s| s.to_string()).collect(),
            ..FileInfo::default()
        }
    }

    /// The registration model. Two files each registering a methodless
    /// `@app.route('/')` are two registrations, so they are two Route nodes —
    /// under the old `(framework, method, path)` identity they collapsed into
    /// one node whose `file_path`/`line_number` described whichever file the
    /// walk reached first, silently mislocating every other registration.
    #[test]
    fn same_path_in_two_files_is_two_registrations() {
        let functions = vec![
            handler("a.index", "a.py", 3, "app.route('/')"),
            handler("b.index", "b.py", 7, "app.route('/')"),
        ];
        let (nodes, edges) = build_routes(&[], &functions, &[]);

        assert_eq!(
            nodes.len(),
            2,
            "one Route node per registration, got {:?}",
            nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
        assert_ne!(nodes[0].id, nodes[1].id, "registrations need distinct ids");

        // Every node reports its OWN declaration site.
        let mut located: Vec<(String, u32)> = nodes
            .iter()
            .map(|n| (n.file_path.clone(), n.line_number))
            .collect();
        located.sort();
        assert_eq!(
            located,
            vec![("a.py".to_string(), 3), ("b.py".to_string(), 7)],
            "each Route must carry its own truthful source location"
        );

        // Both keep the path/method they share — only identity is per-file.
        for n in &nodes {
            assert_eq!(n.path, "/");
            assert_eq!(n.method, "ANY");
            assert_eq!(n.framework, "flask");
        }

        // HANDLES lands on the handler declared in that node's own file.
        let mut handled: Vec<(String, String)> = edges
            .iter()
            .map(|e| {
                let node = nodes
                    .iter()
                    .find(|n| n.id == e.route_id)
                    .expect("edge references a retained node");
                (node.file_path.clone(), e.function_qname.clone())
            })
            .collect();
        handled.sort();
        assert_eq!(
            handled,
            vec![
                ("a.py".to_string(), "a.index".to_string()),
                ("b.py".to_string(), "b.index".to_string()),
            ]
        );
    }

    /// The id carries the declaring FILE, not the line: two registrations of
    /// the same method+path inside one file are one registration-site family,
    /// so they stay one node with parallel HANDLES edges. Line-level identity
    /// was rejected deliberately — it would make every Route id churn whenever
    /// an unrelated line is inserted above it.
    #[test]
    fn duplicate_registrations_within_one_file_stay_one_node() {
        let functions = vec![
            handler("a.index", "a.py", 3, "app.route('/')"),
            handler("a.alias", "a.py", 9, "app.route('/')"),
        ];
        let (nodes, edges) = build_routes(&[], &functions, &[]);
        assert_eq!(nodes.len(), 1, "same file + method + path = one node");
        assert_eq!(edges.len(), 2, "both handlers keep a HANDLES edge");
    }

    /// Stacked decorators still split by method, and distinct paths in one
    /// file stay distinct — the file suffix must not merge anything.
    #[test]
    fn method_and_path_still_separate_registrations_in_one_file() {
        let functions = vec![
            handler(
                "a.get_it",
                "a.py",
                3,
                "app.route('/x', methods=['GET','POST'])",
            ),
            handler("a.other", "a.py", 9, "app.route('/y')"),
        ];
        let (nodes, _) = build_routes(&[], &functions, &[]);
        let mut ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "flask::ANY::/y::a.py",
                "flask::GET::/x::a.py",
                "flask::POST::/x::a.py",
            ]
        );
    }

    /// The ambiguous `@app.<verb>(...)` decorator shape is legal in BOTH
    /// Flask and FastAPI, and the framework label is part of the Route id —
    /// so before import-evidence arbitration one registration minted two
    /// Route nodes (`flask::POST::...` + `fastapi::POST::...`). One
    /// registration is one Route, with the framework decided by the file's
    /// imports.
    #[test]
    fn ambiguous_app_verb_decorator_yields_one_route_per_registration() {
        // fastapi import evidence → the FastAPI detector owns the shape.
        let files = vec![file_with_imports("srv/app.py", &["fastapi.FastAPI"])];
        let functions = vec![handler(
            "srv.app.create",
            "srv/app.py",
            5,
            "app.post('/api/session')",
        )];
        let (nodes, edges) = build_routes(&files, &functions, &[]);
        assert_eq!(
            nodes.len(),
            1,
            "one registration must be one Route, got {:?}",
            nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
        assert_eq!(nodes[0].framework, "fastapi");
        assert_eq!(nodes[0].id, "fastapi::POST::/api/session::srv/app.py");
        assert_eq!(edges.len(), 1);

        // flask import evidence → the Flask detector owns it.
        let files = vec![file_with_imports("web/app.py", &["flask.Flask"])];
        let functions = vec![handler(
            "web.app.login",
            "web/app.py",
            5,
            "app.post('/login')",
        )];
        let (nodes, edges) = build_routes(&files, &functions, &[]);
        assert_eq!(
            nodes.len(),
            1,
            "one registration must be one Route, got {:?}",
            nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
        assert_eq!(nodes[0].framework, "flask");
        assert_eq!(nodes[0].id, "flask::POST::/login::web/app.py");
        assert_eq!(edges.len(), 1);

        // No framework import at all → still exactly one Route (Flask, the
        // default owner of the `app` holder without fastapi evidence).
        let functions = vec![handler("x.app.ping", "x/app.py", 3, "app.get('/ping')")];
        let (nodes, _) = build_routes(&[], &functions, &[]);
        assert_eq!(
            nodes.len(),
            1,
            "one registration must be one Route, got {:?}",
            nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
        assert_eq!(nodes[0].framework, "flask");
    }

    /// The never-ambiguous shapes keep their unconditional owners: `.route`
    /// is Flask even in a fastapi-importing file, and `@router.<verb>` is
    /// FastAPI even with no import evidence at all.
    #[test]
    fn unambiguous_shapes_keep_their_framework() {
        let files = vec![file_with_imports("srv/app.py", &["fastapi.FastAPI"])];
        let functions = vec![handler(
            "srv.app.legacy",
            "srv/app.py",
            9,
            "app.route('/legacy')",
        )];
        let (nodes, _) = build_routes(&files, &functions, &[]);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].framework, "flask", ".route is Flask-only syntax");

        let functions = vec![handler(
            "api.users",
            "api/users.py",
            4,
            "router.get('/users')",
        )];
        let (nodes, _) = build_routes(&[], &functions, &[]);
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes[0].framework, "fastapi",
            "router holder is FastAPI-conventional"
        );
    }

    #[test]
    fn list_form() {
        assert_eq!(parse_methods_list("['GET', 'POST']"), vec!["GET", "POST"]);
        assert_eq!(
            parse_methods_list(r#"["GET", "POST"]"#),
            vec!["GET", "POST"]
        );
    }

    #[test]
    fn tuple_form() {
        // Regression: Flask's own tutorial uses `methods=("GET", "POST")`.
        // Without paren-stripping the methods came out as `("GET` / `POST")`.
        assert_eq!(
            parse_methods_list(r#"("GET", "POST")"#),
            vec!["GET", "POST"]
        );
        assert_eq!(parse_methods_list("('get', 'post')"), vec!["GET", "POST"]);
    }

    #[test]
    fn bare_string() {
        assert_eq!(parse_methods_list("'GET'"), vec!["GET"]);
    }

    #[test]
    fn lowercase_uppercased() {
        assert_eq!(parse_methods_list("['get', 'post']"), vec!["GET", "POST"]);
    }
}
