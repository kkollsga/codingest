//! FastAPI route extraction.
//!
//! Looks for the FastAPI-typical decorator shape:
//!   - `@router.get('/x')` / `@router.post(...)` — APIRouter pattern.
//!   - `@app.get('/x')` / `@app.post(...)` with a FastAPI app.
//!
//! The Flask detector also accepts the `@app.<verb>(...)` shape, and the
//! Route id carries the framework label — so both detectors claiming one
//! decorator minted TWO Route nodes for a single registration. Ownership
//! is settled by import evidence: this detector claims the `app` holder
//! only in files whose imports name `fastapi` (`fastapi_files`, computed
//! in `mod.rs`), and the Flask detector stands down for exactly those
//! files. `router`/`api_router` holders are FastAPI-conventional and are
//! claimed unconditionally — the Flask detector always skips them.

use super::{first_string_literal, make_route_id, split_decorator, RouteEdge, RouteNode};
use crate::models::FunctionInfo;
use std::collections::HashSet;

const FRAMEWORK: &str = "fastapi";

const METHODS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "options", "head", "trace",
];

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
            if !METHODS.contains(&suffix.as_str()) {
                continue;
            }
            // Recognise FastAPI-typical holders. `app` is also Flask's app
            // instance, so it is claimed only on fastapi import evidence —
            // see module docs.
            let holder = head.rsplit('.').nth(1).unwrap_or("").to_ascii_lowercase();
            if holder != "router" && holder != "api_router" && holder != "app" {
                continue;
            }
            if holder == "app" && !fastapi_files.contains(fn_info.file_path.as_str()) {
                continue;
            }
            let Some(path) = first_string_literal(args) else {
                continue;
            };
            let method = suffix.to_ascii_uppercase();
            let id = make_route_id(FRAMEWORK, &method, &path, &fn_info.file_path);
            nodes.push(RouteNode {
                id: id.clone(),
                name: path.clone(),
                path: path.clone(),
                method,
                framework: FRAMEWORK.to_string(),
                file_path: fn_info.file_path.clone(),
                line_number: fn_info.line_number,
            });
            edges.push(RouteEdge {
                route_id: id,
                function_qname: fn_info.qualified_name.clone(),
            });
        }
    }
    (nodes, edges)
}
