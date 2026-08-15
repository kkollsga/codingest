//! Flask route extraction.
//!
//! Recognises three decorator shapes on Python functions:
//!
//! 1. `@app.route('/x')` / `@app.route('/x', methods=['POST'])`
//! 2. `@app.get('/x')`, `@app.post('/x')`, ... (method shortcuts)
//! 3. `@blueprint.route(...)` / `@bp.route(...)` — any name ending in
//!    `.route` follows the same arg shape, so we accept the suffix
//!    pattern rather than enumerating blueprint variable names.
//!
//! FastAPI shares decorator shape 2 — see `fastapi.rs`. Exactly one
//! detector claims each registration: `@router.*`/`@api_router.*` holders
//! are FastAPI's (skipped here via `is_fastapi_holder`), and the ambiguous
//! `@app.<verb>(...)` shape is ceded to FastAPI for files whose imports
//! name `fastapi` (skipped here via `fastapi_files`). Everything else,
//! including every `.route(...)` registration, is Flask's.

use super::{
    first_string_literal, keyword_arg, make_route_id, parse_methods_list, split_decorator,
    RouteEdge, RouteNode,
};
use crate::models::FunctionInfo;
use std::collections::HashSet;

const FRAMEWORK: &str = "flask";

/// Method shortcuts that map directly to HTTP verbs. Any decorator
/// suffixed with `.METHOD` where METHOD is in this list registers a
/// route for that verb. We accept the suffix to cover both `app.get`
/// (the Flask app instance) and `blueprint.get`.
const METHOD_SHORTCUTS: &[&str] = &["get", "post", "put", "delete", "patch", "options", "head"];

pub(super) fn detect(
    functions: &[FunctionInfo],
    fastapi_files: &HashSet<&str>,
) -> (Vec<RouteNode>, Vec<RouteEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for fn_info in functions {
        for raw in &fn_info.decorators {
            let Some((head, args)) = split_decorator(raw) else {
                continue;
            };
            let suffix = head.rsplit('.').next().unwrap_or(head).to_ascii_lowercase();

            // Reject decorators that are FastAPI's exclusive markers
            // (e.g. `router.get` on an APIRouter). We use the variable
            // name preceding `.` as a hint: `router` and `api_router`
            // are FastAPI conventions.
            if is_fastapi_holder(head) {
                continue;
            }

            // `@app.route('/x')` — path is positional, method via kwarg.
            // Flask-only syntax: the FastAPI detector never matches `route`,
            // so this shape needs no import-evidence arbitration.
            if suffix == "route" {
                if let Some(path) = first_string_literal(args) {
                    let methods = keyword_arg(args, "methods")
                        .map(parse_methods_list)
                        .unwrap_or_else(|| vec!["ANY".to_string()]);
                    for method in methods {
                        emit(&mut nodes, &mut edges, fn_info, &path, &method);
                    }
                }
                continue;
            }

            // `@app.get('/x')` / `.post` / ... — method baked into the suffix.
            // Both frameworks accept this shape, and one registration must
            // yield ONE Route: a file with fastapi import evidence hands it
            // to the FastAPI detector, so Flask must stand down there.
            if fastapi_files.contains(fn_info.file_path.as_str()) {
                continue;
            }
            if METHOD_SHORTCUTS.contains(&suffix.as_str()) {
                if let Some(path) = first_string_literal(args) {
                    emit(
                        &mut nodes,
                        &mut edges,
                        fn_info,
                        &path,
                        &suffix.to_ascii_uppercase(),
                    );
                }
            }
        }
    }
    (nodes, edges)
}

fn emit(
    nodes: &mut Vec<RouteNode>,
    edges: &mut Vec<RouteEdge>,
    fn_info: &FunctionInfo,
    path: &str,
    method: &str,
) {
    let id = make_route_id(FRAMEWORK, method, path, &fn_info.file_path);
    nodes.push(RouteNode {
        id: id.clone(),
        name: path.to_string(),
        path: path.to_string(),
        method: method.to_string(),
        framework: FRAMEWORK.to_string(),
        file_path: fn_info.file_path.clone(),
        line_number: fn_info.line_number,
    });
    edges.push(RouteEdge {
        route_id: id,
        function_qname: fn_info.qualified_name.clone(),
    });
}

fn is_fastapi_holder(head: &str) -> bool {
    // The variable preceding `.method` is the routing-app instance.
    let holder = head.rsplit('.').nth(1).unwrap_or("").to_ascii_lowercase();
    holder == "router" || holder == "api_router"
}
