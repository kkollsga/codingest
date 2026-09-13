//! Executable contract for the query recipes shipped inside the CLI crate.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use kglite::api::io::save_graph;
use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::storage::{new_dir_graph_in_mode, StorageMode};
use serde_json::{json, Value};

const SHIPPED_QUERIES: &str = include_str!("../skills/codingest-code-review/references/queries.md");

const WRITE_TARGET: &str = "fixture::graph::execute_mut";
const READ_TARGET: &str = "fixture::graph::execute_read";
const UNUSED_TARGET: &str = "fixture::graph::unused_target";
const MISSING_TARGET: &str = "fixture::graph::missing_target";
const WRITE_ENTRY: &str = "fixture::service::prod_write_000";
const PATH_START: &str = "fixture::service::path_start";
const PATH_FINISH: &str = "fixture::service::path_finish";
const USED_TYPE: &str = "fixture::model::Request";
const USED_TRAIT: &str = "fixture::service::Handler";
const IMPLEMENTOR: &str = "fixture::service::ConcreteHandler";
const TYPE_CONSUMER: &str = "fixture::service::takes_request";
const BOTH_TYPE_CONSUMER: &str = "fixture::service::round_trips_request";
const UNRESOLVED_CALLER: &str = "fixture::service::unresolved_entry";

struct Fixture {
    _dir: tempfile::TempDir,
    source: PathBuf,
    graph: PathBuf,
}

fn cypher_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn mutate(graph: &mut kglite::api::DirGraph, query: &str) {
    let params = HashMap::new();
    let options = ExecuteOptions::eager(&params);
    execute_mut(graph, query, &options)
        .unwrap_or_else(|error| panic!("fixture mutation failed: {query}\n{error}"));
}

fn add_function(
    graph: &mut kglite::api::DirGraph,
    qualified_name: &str,
    file_path: &str,
    line_number: usize,
    is_test: bool,
) {
    let name = qualified_name.rsplit("::").next().unwrap();
    mutate(
        graph,
        &format!(
            "CREATE (:Function {{qualified_name: {}, name: {}, file_path: {}, \
             line_number: {line_number}, is_test: {is_test}}})",
            cypher_literal(qualified_name),
            cypher_literal(name),
            cypher_literal(file_path),
        ),
    );
}

struct CallMetadata<'a> {
    resolution: &'a str,
    candidates: usize,
    import_backed: bool,
    call_lines: &'a str,
}

fn add_call(
    graph: &mut kglite::api::DirGraph,
    caller: &str,
    callee: &str,
    metadata: CallMetadata<'_>,
) {
    mutate(
        graph,
        &format!(
            "MATCH (caller:Function {{qualified_name: {caller}}}), \
                   (callee:Function {{qualified_name: {callee}}}) \
             CREATE (caller)-[:CALLS {{resolution: {resolution}, \
                     candidates: {candidates}, import_backed: {import_backed}, \
                     call_lines: {call_lines}, call_count: 1}}]->(callee)",
            caller = cypher_literal(caller),
            callee = cypher_literal(callee),
            resolution = cypher_literal(metadata.resolution),
            candidates = metadata.candidates,
            import_backed = metadata.import_backed,
            call_lines = cypher_literal(metadata.call_lines),
        ),
    );
}

fn source_lines(count: usize, replacements: &[(usize, &str)]) -> String {
    let mut lines = (1..=count)
        .map(|line| format!("// fixture line {line}"))
        .collect::<Vec<_>>();
    for &(line, text) in replacements {
        lines[line - 1] = text.to_string();
    }
    lines.join("\n") + "\n"
}

fn caller_source(prefix: &str, target: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("pub fn {prefix}_{index:03}() {{\n    {target}();\n}}\n\n"))
        .collect()
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("project");
    for path in ["src", "tests"] {
        std::fs::create_dir_all(source.join(path)).unwrap();
    }
    std::fs::write(
        source.join("src/session.rs"),
        "pub fn execute_mut() {}\npub fn execute_read() {}\npub fn unused_target() {}\n",
    )
    .unwrap();
    std::fs::write(
        source.join("src/alternative.rs"),
        "pub fn execute_mut() {}\n",
    )
    .unwrap();
    std::fs::write(
        source.join("src/service.rs"),
        source_lines(
            130,
            &[
                (
                    10,
                    "pub fn path_start() { path_mid_00(); path_mid_01(); path_mid_02(); path_mid_03(); path_mid_04(); path_mid_05(); path_mid_06(); path_mid_07(); path_mid_08(); path_mid_09(); path_mid_10(); path_mid_11(); }",
                ),
                (30, "pub fn path_finish() {}"),
                (40, "pub fn takes_request(_: Request) {}"),
                (
                    41,
                    "pub fn round_trips_request(value: Request) -> Request { value }",
                ),
                (42, "pub fn returns_request() -> Request { Request }"),
                (50, "pub fn unresolved_entry() { external_missing(); }"),
                (55, "pub trait Handler {}"),
                (60, "pub struct ConcreteHandler;"),
                (61, "impl Handler for ConcreteHandler {}"),
                (100, "pub fn path_mid_00() { path_finish(); }"),
                (101, "pub fn path_mid_01() { path_finish(); }"),
                (102, "pub fn path_mid_02() { path_finish(); }"),
                (103, "pub fn path_mid_03() { path_finish(); }"),
                (104, "pub fn path_mid_04() { path_finish(); }"),
                (105, "pub fn path_mid_05() { path_finish(); }"),
                (106, "pub fn path_mid_06() { path_finish(); }"),
                (107, "pub fn path_mid_07() { path_finish(); }"),
                (108, "pub fn path_mid_08() { path_finish(); }"),
                (109, "pub fn path_mid_09() { path_finish(); }"),
                (110, "pub fn path_mid_10() { path_finish(); }"),
                (111, "pub fn path_mid_11() { path_finish(); }"),
            ],
        ),
    )
    .unwrap();
    std::fs::write(
        source.join("src/prod_write.rs"),
        caller_source("prod_write", "execute_mut", 9),
    )
    .unwrap();
    std::fs::write(
        source.join("src/prod_read.rs"),
        caller_source("prod_read", "execute_read", 13),
    )
    .unwrap();
    std::fs::write(source.join("src/model.rs"), "pub struct Request;\n").unwrap();
    std::fs::write(
        source.join("tests/write_callers.rs"),
        caller_source("test_write", "execute_mut", 111),
    )
    .unwrap();

    let mut graph = new_dir_graph_in_mode(StorageMode::Memory, None).unwrap();
    for (qualified_name, file_path, line) in [
        (WRITE_TARGET, "src/session.rs", 1),
        (READ_TARGET, "src/session.rs", 2),
        (UNUSED_TARGET, "src/session.rs", 3),
        (PATH_START, "src/service.rs", 10),
        (PATH_FINISH, "src/service.rs", 30),
        (TYPE_CONSUMER, "src/service.rs", 40),
        (BOTH_TYPE_CONSUMER, "src/service.rs", 41),
        (UNRESOLVED_CALLER, "src/service.rs", 50),
    ] {
        add_function(&mut graph, qualified_name, file_path, line, false);
    }

    let ambiguous_alternative = "fixture::alternative::execute_mut";
    add_function(
        &mut graph,
        ambiguous_alternative,
        "src/alternative.rs",
        1,
        false,
    );
    for index in 0..9 {
        let caller = format!("fixture::service::prod_write_{index:03}");
        let function_line = index * 4 + 1;
        let call_line = (function_line + 1).to_string();
        add_function(
            &mut graph,
            &caller,
            "src/prod_write.rs",
            function_line,
            false,
        );
        let metadata = if index == 0 {
            CallMetadata {
                resolution: "lang_group",
                candidates: 2,
                import_backed: false,
                call_lines: &call_line,
            }
        } else {
            CallMetadata {
                resolution: "exact_qualified",
                candidates: 1,
                import_backed: true,
                call_lines: &call_line,
            }
        };
        add_call(&mut graph, &caller, WRITE_TARGET, metadata);
        if index == 0 {
            add_call(
                &mut graph,
                &caller,
                ambiguous_alternative,
                CallMetadata {
                    resolution: "lang_group",
                    candidates: 2,
                    import_backed: false,
                    call_lines: &call_line,
                },
            );
        }
    }

    for index in 0..111 {
        let caller = format!("fixture::tests::test_write_{index:03}");
        let function_line = index * 4 + 1;
        let call_line = (function_line + 1).to_string();
        add_function(
            &mut graph,
            &caller,
            "tests/write_callers.rs",
            function_line,
            true,
        );
        add_call(
            &mut graph,
            &caller,
            WRITE_TARGET,
            CallMetadata {
                resolution: "exact_qualified",
                candidates: 1,
                import_backed: true,
                call_lines: &call_line,
            },
        );
    }

    for index in 0..13 {
        let caller = format!("fixture::service::prod_read_{index:03}");
        let function_line = index * 4 + 1;
        let call_line = (function_line + 1).to_string();
        add_function(
            &mut graph,
            &caller,
            "src/prod_read.rs",
            function_line,
            false,
        );
        add_call(
            &mut graph,
            &caller,
            READ_TARGET,
            CallMetadata {
                resolution: "same_file",
                candidates: 1,
                import_backed: true,
                call_lines: &call_line,
            },
        );
    }

    for index in 0..12 {
        let middle = format!("fixture::service::path_mid_{index:02}");
        add_function(&mut graph, &middle, "src/service.rs", 100 + index, false);
        add_call(
            &mut graph,
            PATH_START,
            &middle,
            CallMetadata {
                resolution: "exact_qualified",
                candidates: 1,
                import_backed: true,
                call_lines: "10",
            },
        );
        add_call(
            &mut graph,
            &middle,
            PATH_FINISH,
            CallMetadata {
                resolution: "same_file",
                candidates: 1,
                import_backed: true,
                call_lines: &(100 + index).to_string(),
            },
        );
    }

    mutate(
        &mut graph,
        &format!(
            "CREATE (:Struct {{qualified_name: {}, name: 'Request', \
             file_path: 'src/model.rs', line_number: 1}})",
            cypher_literal(USED_TYPE)
        ),
    );
    let return_consumer = "fixture::service::returns_request";
    add_function(&mut graph, return_consumer, "src/service.rs", 42, false);
    for (consumer, position) in [
        (TYPE_CONSUMER, "parameter"),
        (BOTH_TYPE_CONSUMER, "both"),
        (return_consumer, "return"),
    ] {
        mutate(
            &mut graph,
            &format!(
                "MATCH (consumer:Function {{qualified_name: {consumer}}}), \
                       (used:Struct {{qualified_name: {used}}}) \
                 CREATE (consumer)-[:USES_TYPE {{position: {position}}}]->(used)",
                consumer = cypher_literal(consumer),
                used = cypher_literal(USED_TYPE),
                position = cypher_literal(position),
            ),
        );
    }

    mutate(
        &mut graph,
        &format!(
            "CREATE (:Struct {{qualified_name: {}, name: 'ConcreteHandler', \
             file_path: 'src/service.rs', line_number: 60}})",
            cypher_literal(IMPLEMENTOR)
        ),
    );
    mutate(
        &mut graph,
        &format!(
            "CREATE (:Trait {{qualified_name: {}, name: 'Handler', \
             file_path: 'src/service.rs', line_number: 55}})",
            cypher_literal(USED_TRAIT)
        ),
    );
    mutate(
        &mut graph,
        &format!(
            "MATCH (implementor:Struct {{qualified_name: {implementor}}}), \
                   (implemented:Trait {{qualified_name: {implemented}}}) \
             CREATE (implementor)-[:IMPLEMENTS]->(implemented)",
            implementor = cypher_literal(IMPLEMENTOR),
            implemented = cypher_literal(USED_TRAIT),
        ),
    );

    let graph_path = dir.path().join("recipes.kgl");
    let mut graph = Arc::new(graph);
    save_graph(&mut graph, graph_path.to_str().unwrap()).unwrap();
    Fixture {
        _dir: dir,
        source,
        graph: graph_path,
    }
}

fn extract_recipes() -> BTreeMap<String, String> {
    let lines = SHIPPED_QUERIES.lines().collect::<Vec<_>>();
    let mut recipes = BTreeMap::new();
    let mut index = 0;
    while index < lines.len() {
        let Some(name) = lines[index]
            .strip_prefix("<!-- codingest-query: ")
            .and_then(|line| line.strip_suffix(" -->"))
        else {
            index += 1;
            continue;
        };
        assert_eq!(lines.get(index + 1), Some(&"```sh"), "recipe {name}");
        assert_eq!(
            lines.get(index + 2),
            Some(&"codingest query --graph \"$GRAPH\" --format json - <<'CYPHER'"),
            "recipe {name} must use the documented quoted-heredoc stdin form"
        );
        let body_start = index + 3;
        let body_end = (body_start..lines.len())
            .find(|&line| lines[line] == "CYPHER")
            .unwrap_or_else(|| panic!("recipe {name} has no CYPHER terminator"));
        assert_eq!(lines.get(body_end + 1), Some(&"```"), "recipe {name}");
        let query = lines[body_start..body_end].join("\n");
        assert!(recipes.insert(name.to_string(), query).is_none());
        index = body_end + 2;
    }
    recipes
}

fn instantiate(query: &str) -> String {
    let replacements = [
        ("crate::src::graph::session::execute_mut", WRITE_TARGET),
        ("crate::src::graph::session::execute_read", READ_TARGET),
        ("crate::src::graph::session::unused_target", UNUSED_TARGET),
        ("crate::src::graph::session::missing_target", MISSING_TARGET),
        ("crate::src::service::write_entry", WRITE_ENTRY),
        ("crate::src::service::path_start", PATH_START),
        ("crate::src::service::path_finish", PATH_FINISH),
        ("crate::src::model::Request", USED_TYPE),
        ("crate::src::service::Handler", USED_TRAIT),
    ];
    replacements
        .iter()
        .fold(query.to_string(), |query, (from, to)| {
            query.replace(&cypher_literal(from), &cypher_literal(to))
        })
}

fn run_stdin_query(graph: &Path, query: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codingest"))
        .args([
            "query",
            "--graph",
            graph.to_str().unwrap(),
            "--format",
            "json",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(query.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "query failed ({:?}):\n{query}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "query did not return JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn run_recipe(fixture: &Fixture, recipes: &BTreeMap<String, String>, name: &str) -> Value {
    let query = instantiate(
        recipes
            .get(name)
            .unwrap_or_else(|| panic!("missing recipe {name}")),
    );
    run_stdin_query(&fixture.graph, &query)
}

fn assert_source_line(fixture: &Fixture, file: &str, line: usize, expected: &str) {
    let source = std::fs::read_to_string(fixture.source.join(file)).unwrap();
    let actual = source
        .lines()
        .nth(line - 1)
        .unwrap_or_else(|| panic!("missing {file}:{line}"));
    assert!(
        actual.contains(expected),
        "{file}:{line} is {actual:?}, expected {expected:?}"
    );
}

fn assert_caller_row_source(fixture: &Fixture, row: &Value, target_name: &str) {
    let caller = row[1].as_str().unwrap();
    let file = row[2].as_str().unwrap();
    let function_line = row[3].as_u64().unwrap() as usize;
    assert_source_line(
        fixture,
        file,
        function_line,
        caller.rsplit("::").next().unwrap(),
    );
    for line in row[7].as_str().unwrap().split(',') {
        assert_source_line(fixture, file, line.parse().unwrap(), target_name);
    }
}

#[test]
fn shipped_recipe_fences_execute_via_stdin_json() {
    let fixture = fixture();
    let recipes = extract_recipes();
    assert_eq!(
        recipes.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "bounded-call-path",
            "direct-callees",
            "production-callers",
            "read-caller-coverage",
            "target-coverage",
            "test-callers",
            "trait-implementations",
            "type-consumers",
            "write-caller-coverage",
        ]
    );
    assert!(!SHIPPED_QUERIES.contains("--params"));

    for name in recipes.keys() {
        let output = run_recipe(&fixture, &recipes, name);
        assert!(output["columns"].is_array(), "{name}: {output}");
        assert!(output["rows"].is_array(), "{name}: {output}");
    }
}

#[test]
fn caller_recipes_keep_targets_and_prod_tests_separate() {
    let fixture = fixture();
    let recipes = extract_recipes();

    let targets = run_recipe(&fixture, &recipes, "target-coverage");
    assert_eq!(
        targets["rows"],
        json!([
            [WRITE_TARGET, WRITE_TARGET, 120],
            [MISSING_TARGET, null, 0],
            [UNUSED_TARGET, UNUSED_TARGET, 0],
        ])
    );

    let write = run_recipe(&fixture, &recipes, "write-caller-coverage");
    assert_eq!(
        write["rows"],
        json!([[WRITE_TARGET, false, 9], [WRITE_TARGET, true, 111]])
    );
    let read = run_recipe(&fixture, &recipes, "read-caller-coverage");
    assert_eq!(read["rows"], json!([[READ_TARGET, false, 13]]));

    let production = run_recipe(&fixture, &recipes, "production-callers");
    assert_eq!(production["rows"].as_array().unwrap().len(), 9);
    assert!(production["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| { row[0] == WRITE_TARGET && row[2].as_str().unwrap().starts_with("src/") }));
    assert_eq!(
        production["rows"][0],
        json!([
            WRITE_TARGET,
            WRITE_ENTRY,
            "src/prod_write.rs",
            1,
            "lang_group",
            2,
            false,
            "2",
        ])
    );
    for row in production["rows"].as_array().unwrap() {
        assert_caller_row_source(&fixture, row, "execute_mut();");
    }

    let read_production_recipe = instantiate(recipes.get("production-callers").unwrap())
        .replace(&cypher_literal(WRITE_TARGET), &cypher_literal(READ_TARGET));
    assert_eq!(read_production_recipe.matches("SKIP 0 LIMIT 25").count(), 1);
    let mut all_read_callers = Vec::new();
    for offset in [0, 25] {
        let page_query =
            read_production_recipe.replace("SKIP 0 LIMIT 25", &format!("SKIP {offset} LIMIT 25"));
        let page = run_stdin_query(&fixture.graph, &page_query);
        let rows = page["rows"].as_array().unwrap();
        assert_eq!(rows.len(), if offset == 0 { 13 } else { 0 });
        for row in rows {
            assert_eq!(row[0], READ_TARGET);
            assert!(row[2].as_str().unwrap().starts_with("src/"));
            assert_caller_row_source(&fixture, row, "execute_read();");
            all_read_callers.push(row[1].as_str().unwrap().to_string());
        }
    }
    let expected_read_callers = (0..13)
        .map(|index| format!("fixture::service::prod_read_{index:03}"))
        .collect::<Vec<_>>();
    assert_eq!(all_read_callers, expected_read_callers);

    let test_recipe = instantiate(recipes.get("test-callers").unwrap());
    assert_eq!(test_recipe.matches("SKIP 0 LIMIT 25").count(), 1);
    let mut all_test_callers = Vec::new();
    for offset in (0..=125).step_by(25) {
        let page_query = test_recipe.replace("SKIP 0 LIMIT 25", &format!("SKIP {offset} LIMIT 25"));
        let page = run_stdin_query(&fixture.graph, &page_query);
        let rows = page["rows"].as_array().unwrap();
        let expected_page_len = match offset {
            0 | 25 | 50 | 75 => 25,
            100 => 11,
            125 => 0,
            _ => unreachable!(),
        };
        assert_eq!(rows.len(), expected_page_len, "SKIP {offset}");
        for row in rows {
            assert_eq!(row[0], WRITE_TARGET);
            assert!(row[2].as_str().unwrap().starts_with("tests/"));
            assert_caller_row_source(&fixture, row, "execute_mut();");
            all_test_callers.push(row[1].as_str().unwrap().to_string());
        }
    }
    let expected_test_callers = (0..111)
        .map(|index| format!("fixture::tests::test_write_{index:03}"))
        .collect::<Vec<_>>();
    assert_eq!(all_test_callers, expected_test_callers);

    // Regression witness for the reported starvation shape: changing these
    // per-target recipes back to one ordered global LIMIT returns all 120 write
    // callers and silently excludes all 13 read callers.
    let legacy_global_limit = format!(
        "MATCH (caller:Function)-[:CALLS]->(target:Function) \
         WHERE target.qualified_name IN [{write}, {read}] \
         RETURN target.qualified_name AS target, caller.qualified_name AS caller \
         ORDER BY target, caller LIMIT 120",
        write = cypher_literal(WRITE_TARGET),
        read = cypher_literal(READ_TARGET),
    );
    let starved = run_stdin_query(&fixture.graph, &legacy_global_limit);
    assert_eq!(starved["rows"].as_array().unwrap().len(), 120);
    assert!(
        starved["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row[0] == WRITE_TARGET),
        "the acceptance fixture no longer reproduces target starvation"
    );
}

#[test]
fn bounded_path_type_and_impl_recipes_project_verified_schema() {
    let fixture = fixture();
    let recipes = extract_recipes();

    let callees = run_recipe(&fixture, &recipes, "direct-callees");
    assert_eq!(callees["rows"].as_array().unwrap().len(), 2);
    assert!(callees["rows"].as_array().unwrap().iter().all(|row| {
        row[0] == WRITE_ENTRY
            && row[2] == "src/prod_write.rs"
            && row[3] == 1
            && row[6] == "lang_group"
            && row[7] == 2
            && row[8] == false
            && row[9] == "2"
    }));
    for row in callees["rows"].as_array().unwrap() {
        assert_source_line(&fixture, row[2].as_str().unwrap(), 1, "prod_write_000");
        assert_source_line(&fixture, row[2].as_str().unwrap(), 2, "execute_mut();");
        assert_source_line(
            &fixture,
            row[4].as_str().unwrap(),
            row[5].as_u64().unwrap() as usize,
            row[1].as_str().unwrap().rsplit("::").next().unwrap(),
        );
    }

    let paths = run_recipe(&fixture, &recipes, "bounded-call-path");
    assert_eq!(paths["rows"].as_array().unwrap().len(), 10);
    assert!(paths["rows"].as_array().unwrap().iter().all(|row| {
        row[0].as_array().unwrap().first() == Some(&json!(PATH_START))
            && row[0].as_array().unwrap().last() == Some(&json!(PATH_FINISH))
            && row[1] == 2
    }));
    let service_source = std::fs::read_to_string(fixture.source.join("src/service.rs")).unwrap();
    for row in paths["rows"].as_array().unwrap() {
        let middle = row[0][1].as_str().unwrap().rsplit("::").next().unwrap();
        assert!(service_source.contains(&format!("pub fn {middle}()")));
    }

    let consumers = run_recipe(&fixture, &recipes, "type-consumers");
    assert_eq!(
        consumers["rows"],
        json!([
            [USED_TYPE, BOTH_TYPE_CONSUMER, "src/service.rs", 41, "both"],
            [USED_TYPE, TYPE_CONSUMER, "src/service.rs", 40, "parameter"]
        ])
    );
    let implementations = run_recipe(&fixture, &recipes, "trait-implementations");
    assert_eq!(
        implementations["rows"],
        json!([[USED_TRAIT, IMPLEMENTOR, "src/service.rs", 60]])
    );

    assert_source_line(&fixture, "src/service.rs", 40, "takes_request(_: Request)");
    assert_source_line(
        &fixture,
        "src/service.rs",
        41,
        "round_trips_request(value: Request) -> Request",
    );
    assert_source_line(&fixture, "src/service.rs", 55, "trait Handler");
    assert_source_line(&fixture, "src/service.rs", 60, "struct ConcreteHandler");
    assert_source_line(
        &fixture,
        "src/service.rs",
        61,
        "impl Handler for ConcreteHandler",
    );
    for file in [
        "src/session.rs",
        "src/alternative.rs",
        "src/service.rs",
        "src/prod_write.rs",
        "src/prod_read.rs",
        "src/model.rs",
        "tests/write_callers.rs",
    ] {
        assert!(
            fixture.source.join(file).is_file(),
            "missing fixture path {file}"
        );
    }

    let unresolved = run_stdin_query(
        &fixture.graph,
        &format!(
            "MATCH (caller:Function {{qualified_name: {caller}}}) \
             OPTIONAL MATCH (caller)-[:CALLS]->(callee:Function) \
             RETURN caller.file_path AS file, callee.qualified_name AS callee",
            caller = cypher_literal(UNRESOLVED_CALLER),
        ),
    );
    assert_eq!(unresolved["rows"], json!([["src/service.rs", null]]));
    assert!(
        std::fs::read_to_string(fixture.source.join("src/service.rs"))
            .unwrap()
            .contains("external_missing();"),
        "an absent CALLS edge must not become proof that source has no call"
    );
}
