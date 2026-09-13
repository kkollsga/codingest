use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const RPC_TIMEOUT: Duration = Duration::from_secs(120);
const STDERR_TAIL_LINES: usize = 12;

struct Rpc {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    next_id: u64,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
}

impl Rpc {
    fn spawn(watch_root: &Path, manifest: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codingest-mcp"))
            .arg("--watch")
            .arg(watch_root)
            .arg("--writable")
            .arg("--mcp-config")
            .arg(manifest)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn codingest-mcp");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();
        let stdout_reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(frame) = serde_json::from_str(&line) {
                    if tx.send(frame).is_err() {
                        break;
                    }
                }
            }
        });
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        let tail = stderr_tail.clone();
        let stderr_reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let mut tail = tail.lock().expect("stderr tail lock");
                if tail.len() == STDERR_TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
        let mut rpc = Self {
            child,
            stdin,
            rx,
            next_id: 0,
            stderr_tail,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
        };
        let initialized = rpc.request_ok(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "codingest-response-contract", "version": "1"}
            }),
        );
        assert_eq!(initialized["serverInfo"]["name"], "Codingest");
        rpc.notify("notifications/initialized");
        rpc
    }

    fn diagnostics(&mut self) -> String {
        let status = self
            .child
            .try_wait()
            .ok()
            .flatten()
            .map_or_else(|| "running".to_string(), |status| status.to_string());
        let tail = self.stderr_tail.lock().expect("stderr tail lock");
        format!(
            "child={status}; stderr={}",
            tail.iter().cloned().collect::<Vec<_>>().join(" | ")
        )
    }

    fn send(&mut self, frame: &Value) {
        writeln!(self.stdin, "{frame}").expect("write JSON-RPC frame");
        self.stdin.flush().expect("flush JSON-RPC frame");
    }

    fn request_frame(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
        loop {
            match self.rx.recv_timeout(RPC_TIMEOUT) {
                Ok(frame) if frame["id"] == id => return frame,
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("timed out waiting for {method}: {}", self.diagnostics())
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!(
                        "server disconnected during {method}: {}",
                        self.diagnostics()
                    )
                }
            }
        }
    }

    fn request_ok(&mut self, method: &str, params: Value) -> Value {
        let frame = self.request_frame(method, params);
        assert!(frame.get("error").is_none(), "{method} failed: {frame}");
        frame
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} returned no result: {frame}"))
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc":"2.0", "method":method}));
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request_ok("tools/call", json!({"name":name, "arguments":arguments}))
    }

    fn call_frame(&mut self, name: &str, arguments: Value) -> Value {
        self.request_frame("tools/call", json!({"name":name, "arguments":arguments}))
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    root_a: PathBuf,
    root_b: PathBuf,
    manifest: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let sandbox = temp.path().canonicalize().expect("canonical fixture root");
        let root_a = sandbox.join("root-a");
        let root_b = sandbox.join("root-b");
        std::fs::create_dir_all(root_a.join("src")).expect("create root A");
        std::fs::create_dir_all(&root_b).expect("create root B");

        let mut rust = String::new();
        for index in 0..48 {
            rust.push_str(&format!(
                "/// contract evidence {index}: {}\npub fn fixture_{index:03}() -> usize {{ {index} }}\n\n",
                "e".repeat(480)
            ));
        }
        std::fs::write(root_a.join("src/lib.rs"), rust).expect("write root A source");
        std::fs::write(
            root_b.join("other.py"),
            "class BetaOnly:\n    def answer(self):\n        return 2\n",
        )
        .expect("write root B source");

        // Local-workspace manifest settings take precedence over the `--watch`
        // launch argument. These response tests mutate the in-memory graph and
        // exercise root switches explicitly, so disable filesystem refresh;
        // Linux read-open events otherwise rebuild and erase test-owned nodes.
        let manifest = sandbox.join("response_contract_mcp.yaml");
        std::fs::write(
            &manifest,
            format!(
                "name: Codingest\n\
             workspace:\n\
             \x20 kind: local\n\
             \x20 root: {}\n\
             \x20 sandbox_root: {}\n\
             \x20 watch: false\n\
             tools:\n\
             \x20 - name: expand_response\n\
             \x20   description: Public manifest collision fixture.\n\
             \x20   parameters:\n\
             \x20     type: object\n\
             \x20     properties:\n\
             \x20       _response: {{type: string}}\n\
             \x20     required: [_response]\n\
             \x20     additionalProperties: false\n\
             \x20   cypher: RETURN $_response AS domain_value\n",
                sandbox.display(),
                sandbox.display()
            ),
        )
        .expect("write MCP manifest");
        Self {
            _temp: temp,
            root_a,
            root_b,
            manifest,
        }
    }

    fn rpc(&self) -> Rpc {
        Rpc::spawn(&self.root_a, &self.manifest)
    }
}

fn activate(rpc: &mut Rpc, root: &Path) {
    let result = rpc.call("set_root_dir", json!({"path":root}));
    assert_success(&result);
    let first_line = result_text(&result)
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim();
    assert!(
        (first_line.starts_with("Cloned '")
            || first_line.starts_with("Updated '")
            || first_line.starts_with("Activated (already up to date) '"))
            && first_line.contains("' at "),
        "root activation did not report success: {result}"
    );
}

fn assert_success(result: &Value) {
    assert_ne!(result["isError"], true, "tool failed: {result}");
}

fn result_text(result: &Value) -> &str {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result has no text: {result}"))
}

fn response_budget(result: &Value) -> Value {
    let text = result_text(result);
    serde_json::from_str::<Value>(text)
        .unwrap_or_else(|error| panic!("expected response preview ({error}): {text}"))
        ["response_budget"]
        .clone()
}

fn assert_rpc_error_contains(frame: &Value, expected: &str) {
    let error = frame
        .get("error")
        .unwrap_or_else(|| panic!("expected JSON-RPC error containing {expected:?}: {frame}"));
    assert!(
        error.to_string().contains(expected),
        "JSON-RPC error did not contain {expected:?}: {error}"
    );
}

fn outlined_field<'a>(outline: &'a Value, location: &str) -> &'a Value {
    if let Some(value) = outline
        .get("value")
        .and_then(|value| value.pointer(location))
    {
        return value;
    }
    outline["fields"]
        .as_array()
        .and_then(|fields| fields.iter().find(|field| field["location"] == location))
        .and_then(|field| field.get("value"))
        .unwrap_or_else(|| panic!("missing outlined field {location}: {outline}"))
}

fn compose_expansion(base: &Value, target: &Value) -> (String, Value) {
    let name = base["name"]
        .as_str()
        .expect("advertised expansion name")
        .to_string();
    let mut arguments = base["arguments"].clone();
    arguments["path"] = target["json_pointer"].clone();
    arguments["offset"] = target["offset"].clone();
    arguments["response"] = target["response"].clone();
    (name, arguments)
}

fn listed_tool<'a>(listing: &'a Value, name: &str) -> &'a Value {
    listing["tools"]
        .as_array()
        .expect("tools/list array")
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("tool {name:?} missing from tools/list: {listing}"))
}

fn response_control_name(tool: &Value) -> String {
    let controls = tool["inputSchema"]["properties"]
        .as_object()
        .expect("tool input-schema properties")
        .iter()
        .filter(|(_, schema)| {
            schema["properties"]["mode"]["default"] == "bounded"
                && schema["properties"]["max_bytes"]["minimum"] == 4096
        })
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        controls.len(),
        1,
        "tool must advertise exactly one response control: {tool}"
    );
    controls.into_iter().next().unwrap()
}

fn with_response_control(mut arguments: Value, control: &str, options: Value) -> Value {
    arguments[control] = options;
    arguments
}

fn assert_expansion_schema(tool: &Value) {
    let required = tool["inputSchema"]["required"]
        .as_array()
        .expect("expansion required fields");
    assert!(required.iter().any(|field| field == "result_id"));
    for property in ["result_id", "path", "offset", "response"] {
        assert!(
            tool["inputSchema"]["properties"].get(property).is_some(),
            "expansion tool omits {property}: {tool}"
        );
    }
}

fn find_outline_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if value.get("path").and_then(Value::as_str) == Some(path) {
        return Some(value);
    }
    value
        .get("items")
        .or_else(|| value.get("fields"))
        .and_then(Value::as_array)
        .and_then(|children| {
            children
                .iter()
                .find_map(|child| find_outline_path(child, path))
        })
}

fn structured_count(result: &Value) -> i64 {
    result["structuredContent"]["rows"][0][0]
        .as_i64()
        .unwrap_or_else(|| panic!("expected count result: {result}"))
}

#[test]
fn discovery_and_advertised_expansion_survive_tool_name_collisions() {
    let fixture = Fixture::new();
    let mut rpc = fixture.rpc();
    let listing = rpc.request_ok("tools/list", json!({}));
    let domain = listed_tool(&listing, "expand_response");
    assert!(domain["inputSchema"]["properties"]
        .get("_response")
        .is_some());
    let domain_control = response_control_name(domain);
    assert_ne!(domain_control, "_response");
    let cypher_control = response_control_name(listed_tool(&listing, "cypher_query"));
    assert_ne!(domain_control, cypher_control);

    activate(&mut rpc, &fixture.root_a);
    let domain_result = rpc.call(
        "expand_response",
        with_response_control(
            json!({"_response":"domain value"}),
            &domain_control,
            json!({"mode":"full"}),
        ),
    );
    assert_success(&domain_result);
    assert!(result_text(&domain_result).contains("domain value"));

    let result = rpc.call(
        "cypher_query",
        json!({
            "query":"MATCH (f:Function) WHERE f.file_path = 'src/lib.rs' RETURN f.qualified_name AS id, f.docstring AS evidence ORDER BY id"
        }),
    );
    assert_success(&result);
    assert!(serde_json::to_vec(&result).unwrap().len() <= 16_384);
    let budget = response_budget(&result);
    assert_eq!(budget["complete"], false);
    assert_eq!(budget["tool"], "cypher_query");
    let advertised_name = budget["next"]["selected_value"]["name"]
        .as_str()
        .expect("advertised expansion name");
    assert_ne!(advertised_name, "expand_response");
    assert_expansion_schema(listed_tool(&listing, advertised_name));
    assert_eq!(
        budget["next"]["full_result"]["name"],
        budget["next"]["selected_value"]["name"]
    );
    let guidance = &budget["domain_guidance"];
    let coverage = outlined_field(guidance, "/coverage");
    assert_eq!(coverage["query_and_executor"]["executed_rows"], 48);
    assert_eq!(
        coverage["query_and_executor"]["database_population"],
        "unknown"
    );
    let directory_pointer = coverage["navigation_json_pointer"]
        .as_str()
        .expect("navigation pointer");
    let action = &budget["next"]["selected_value"];
    let directory = rpc.call(action["name"].as_str().unwrap(), {
        let mut arguments = action["arguments"].clone();
        arguments["path"] = json!(directory_pointer);
        arguments
    });
    let navigation: Value = serde_json::from_str(result_text(&directory)).expect("navigation JSON");
    assert_eq!(navigation["available"]["row_count"], 48);
    let row_target = navigation["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["json_pointer"] == "/rows")
        .expect("advertised row target");
    assert_eq!(row_target["offset"], 15);
    let (name, arguments) = compose_expansion(action, row_target);
    let page = rpc.call(&name, arguments);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 4096);
    let page_budget = response_budget(&page);
    assert_eq!(page_budget["preview"]["offset"], 15);
    let id = find_outline_path(&page_budget["preview"], "/rows/15")
        .and_then(|outline| outline.pointer("/value/0"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("first selected row identity missing: {page_budget}"));
    assert!(id.contains("fixture_015"), "unexpected row identity: {id}");
    assert!(
        page_budget["next"]["page"]["arguments"]["offset"]
            .as_u64()
            .unwrap()
            > 15
    );
}

#[test]
fn nested_late_value_expands_after_root_swap_without_replay() {
    let fixture = Fixture::new();
    let mut rpc = fixture.rpc();
    let listing = rpc.request_ok("tools/list", json!({}));
    let cypher_control = response_control_name(listed_tool(&listing, "cypher_query"));
    activate(&mut rpc, &fixture.root_a);
    let nested = json!({
        "deep": [null, true, 7, 1.5, {"a/b~c":"z".repeat(8_000)}]
    });
    let result = rpc.call(
        "cypher_query",
        with_response_control(json!({
            "query":"UNWIND range(0,47) AS i CREATE (:Snapshot {id:i}) RETURN i AS id, $nested AS nested ORDER BY id",
            "params":{"nested":nested}
        }), &cypher_control, json!({"max_bytes":4096})),
    );
    assert_success(&result);
    let budget = response_budget(&result);
    let action = &budget["next"]["selected_value"];
    let directory = rpc.call(action["name"].as_str().unwrap(), {
        let mut arguments = action["arguments"].clone();
        arguments["path"] = json!("/navigation");
        arguments
    });
    let navigation: Value = serde_json::from_str(result_text(&directory)).expect("navigation JSON");
    let target = navigation["observed_value_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| {
            target["json_pointer"]
                .as_str()
                .is_some_and(|path| path.ends_with("/deep/4/a~1b~0c"))
        })
        .expect("escaped nested target");
    assert_eq!(target["json_pointer"], "/rows/0/1/deep/4/a~1b~0c");
    let (name, arguments) = compose_expansion(action, target);
    let selected = rpc.call(&name, arguments);
    let selected_budget = response_budget(&selected);
    assert!(
        selected_budget["next"]["page"]["arguments"]["offset"]
            .as_u64()
            .unwrap()
            > 0
    );
    let nested_full = rpc.call(action["name"].as_str().unwrap(), {
        let mut arguments = action["arguments"].clone();
        arguments["path"] = target["json_pointer"].clone();
        arguments["response"] = json!({"mode":"full"});
        arguments
    });
    assert_eq!(
        serde_json::from_str::<String>(result_text(&nested_full)).unwrap(),
        "z".repeat(8_000)
    );

    let late_arguments = {
        let mut arguments = action["arguments"].clone();
        arguments["path"] = json!("/rows/47/0");
        arguments
    };
    let late = rpc.call(action["name"].as_str().unwrap(), late_arguments.clone());
    assert_eq!(result_text(&late), "47");
    let changed = rpc.call(
        "cypher_query",
        json!({"query":"MATCH (n:Snapshot {id:47}) DELETE n CREATE (:MutationMarker {id:1})"}),
    );
    assert_success(&changed);
    let retained = rpc.call(action["name"].as_str().unwrap(), late_arguments.clone());
    assert_eq!(result_text(&retained), "47");
    assert_eq!(
        structured_count(&rpc.call(
            "cypher_query",
            json!({"query":"MATCH (n:Snapshot) RETURN count(n) AS count"}),
        )),
        47
    );
    assert_eq!(
        structured_count(&rpc.call(
            "cypher_query",
            json!({"query":"MATCH (n:MutationMarker) RETURN count(n) AS count"}),
        )),
        1
    );
    let missing_live = rpc.call(
        "cypher_query",
        json!({"query":"MATCH (n:Snapshot {id:47}) RETURN n.id AS id"}),
    );
    assert!(missing_live["structuredContent"]["rows"]
        .as_array()
        .unwrap()
        .is_empty());

    activate(&mut rpc, &fixture.root_b);
    let retained_after_swap = rpc.call(action["name"].as_str().unwrap(), late_arguments);
    assert_eq!(result_text(&retained_after_swap), "47");
    let retained_full = rpc.call(action["name"].as_str().unwrap(), {
        let mut arguments = action["arguments"].clone();
        arguments["response"] = json!({"mode":"full"});
        arguments
    });
    let root_a = fixture.root_a.canonicalize().unwrap();
    let root_b = fixture.root_b.canonicalize().unwrap();
    let retained_graph = retained_full["structuredContent"]["identity"]["footer"]
        .as_str()
        .unwrap_or_else(|| panic!("retained result omits graph identity: {retained_full}"));
    assert!(retained_graph.contains(&root_a.to_string_lossy().to_string()));
    assert!(!retained_graph.contains(&root_b.to_string_lossy().to_string()));
    assert_eq!(
        structured_count(&rpc.call(
            "cypher_query",
            json!({"query":"MATCH (n:Snapshot) RETURN count(n) AS count"}),
        )),
        0
    );
}

#[test]
fn per_call_larger_full_and_error_responses_preserve_status() {
    let fixture = Fixture::new();
    let mut rpc = fixture.rpc();
    let listing = rpc.request_ok("tools/list", json!({}));
    let cypher_control = response_control_name(listed_tool(&listing, "cypher_query"));
    activate(&mut rpc, &fixture.root_a);
    let query = "UNWIND range(0,63) AS i RETURN i AS id, $body AS body ORDER BY id";
    let params = json!({"body":"b".repeat(600)});
    let ordinary = rpc.call("cypher_query", json!({"query":query, "params":params}));
    let ordinary_budget = response_budget(&ordinary);
    assert_eq!(ordinary_budget["max_bytes"], 16_384);
    let expansion_name = ordinary_budget["next"]["full_result"]["name"]
        .as_str()
        .unwrap()
        .to_string();

    let larger = rpc.call(
        "cypher_query",
        with_response_control(
            json!({"query":query, "params":{"body":"b".repeat(600)}}),
            &cypher_control,
            json!({"max_bytes":32768}),
        ),
    );
    assert_eq!(response_budget(&larger)["max_bytes"], 32_768);
    assert!(serde_json::to_vec(&larger).unwrap().len() <= 32_768);
    let full = rpc.call(
        "cypher_query",
        with_response_control(
            json!({"query":query, "params":{"body":"b".repeat(600)}}),
            &cypher_control,
            json!({"mode":"full"}),
        ),
    );
    assert_success(&full);
    assert_eq!(
        full["structuredContent"]["rows"].as_array().unwrap().len(),
        64
    );
    let ordinary_again = rpc.call(
        "cypher_query",
        json!({"query":query, "params":{"body":"b".repeat(600)}}),
    );
    assert_eq!(response_budget(&ordinary_again)["max_bytes"], 16_384);

    let rejected = rpc.call_frame(
        "cypher_query",
        with_response_control(
            json!({"query":"CREATE (:ShouldNotExist {id:1})"}),
            &cypher_control,
            json!({"max_bytes":1}),
        ),
    );
    assert_rpc_error_contains(&rejected, "max_bytes must be at least 4096");
    assert_eq!(
        structured_count(&rpc.call(
            "cypher_query",
            json!({"query":"MATCH (n:ShouldNotExist) RETURN count(n) AS count"}),
        )),
        0
    );
    let failed = rpc.call("cypher_query", json!({"query":"RETURN @"}));
    assert_eq!(failed["isError"], true);
    assert!(
        result_text(&failed).starts_with("Cypher syntax error"),
        "unexpected error body: {failed}"
    );

    let unavailable = rpc.call_frame(
        &expansion_name,
        json!({"result_id":"not-a-result", "response":{"mode":"full"}}),
    );
    assert_rpc_error_contains(&unavailable, "Result unavailable in this session");
    let invalid_path = rpc.call_frame(&expansion_name, {
        let mut arguments = ordinary_budget["next"]["selected_value"]["arguments"].clone();
        arguments["path"] = json!("/not-present");
        arguments
    });
    assert_rpc_error_contains(&invalid_path, "No value at that JSON Pointer");
    let invalid_offset = rpc.call_frame(&expansion_name, {
        let mut arguments = ordinary_budget["next"]["selected_value"]["arguments"].clone();
        arguments["path"] = json!("/rows");
        arguments["offset"] = json!(1_000_000);
        arguments
    });
    assert_rpc_error_contains(&invalid_offset, "offset is beyond the selected value");
    let invalid_override = rpc.call_frame(&expansion_name, {
        let mut arguments = ordinary_budget["next"]["selected_value"]["arguments"].clone();
        arguments["response"] = json!({"mode":"full", "max_bytes":4096});
        arguments
    });
    assert_rpc_error_contains(&invalid_override, "full mode cannot also specify max_bytes");

    let created = rpc.call(
        "cypher_query",
        json!({"query":"UNWIND range(0,39) AS i CREATE (:LimitProbe {id:i})"}),
    );
    assert_success(&created);
    let limited = rpc.call(
        "cypher_query",
        json!({
            "query":"MATCH (n:LimitProbe) RETURN n.id AS id, $body AS body ORDER BY id LIMIT 7",
            "params":{"body":"l".repeat(3_000)}
        }),
    );
    let limited_budget = response_budget(&limited);
    let limited_full = rpc.call(
        limited_budget["next"]["full_result"]["name"]
            .as_str()
            .unwrap(),
        limited_budget["next"]["full_result"]["arguments"].clone(),
    );
    assert_eq!(
        limited_full["structuredContent"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(
        limited_full["structuredContent"]["coverage"]["query_literal_limits"],
        json!([7])
    );
    assert_eq!(
        limited_full["structuredContent"]["coverage"]["literal_limit_status"],
        "executed_rows_match_a_literal_limit"
    );
    assert_eq!(
        limited_full["structuredContent"]["coverage"]["database_population"],
        "unknown"
    );
    let omitted = rpc.call_frame(&expansion_name, {
        let mut arguments = limited_budget["next"]["selected_value"]["arguments"].clone();
        arguments["path"] = json!("/rows/7");
        arguments
    });
    assert_rpc_error_contains(&omitted, "No value at that JSON Pointer");
    assert_eq!(
        structured_count(&rpc.call(
            "cypher_query",
            json!({"query":"MATCH (n:LimitProbe) RETURN count(n) AS count"}),
        )),
        40
    );

    let mut first_id = None;
    let mut retained_bytes = 0;
    for call in 0..32 {
        let bounded = rpc.call(
            "cypher_query",
            json!({
                "query":"UNWIND range(0,47) AS i CREATE (:EvictionCall {call:$call, id:i}) RETURN $call AS call, i AS id, $body AS body ORDER BY id",
                "params":{"call":call, "body":"x".repeat(500)}
            }),
        );
        let budget = response_budget(&bounded);
        retained_bytes += budget["original_bytes"].as_u64().unwrap();
        if first_id.is_none() {
            first_id = budget["result_id"].as_str().map(str::to_string);
        }
    }
    assert!(retained_bytes < 32 * 1024 * 1024);
    let first_id = first_id.expect("first retained id");
    let retained_at_capacity = rpc.call(
        &expansion_name,
        json!({"result_id":first_id, "response":{"mode":"full"}}),
    );
    assert_eq!(retained_at_capacity["structuredContent"]["rows"][0][0], 0);
    let bounded_33 = rpc.call(
        "cypher_query",
        json!({
            "query":"UNWIND range(0,47) AS i CREATE (:EvictionCall {call:$call, id:i}) RETURN $call AS call, i AS id, $body AS body ORDER BY id",
            "params":{"call":32, "body":"x".repeat(500)}
        }),
    );
    assert!(response_budget(&bounded_33)["original_bytes"]
        .as_u64()
        .is_some_and(|bytes| retained_bytes + bytes < 32 * 1024 * 1024));
    let evicted = rpc.call_frame(
        &expansion_name,
        json!({"result_id":first_id, "response":{"mode":"full"}}),
    );
    assert_rpc_error_contains(&evicted, "expired or evicted");
    assert_eq!(
        structured_count(&rpc.call(
            "cypher_query",
            json!({"query":"MATCH (n:EvictionCall) RETURN count(n) AS count"}),
        )),
        33 * 48
    );
}

#[test]
fn graph_overview_schema_refreshes_after_root_switch() {
    let fixture = Fixture::new();
    let mut rpc = fixture.rpc();
    let listing = rpc.request_ok("tools/list", json!({}));
    let overview_control = response_control_name(listed_tool(&listing, "graph_overview"));
    activate(&mut rpc, &fixture.root_a);
    let full_overview = |rpc: &mut Rpc| {
        rpc.call(
            "graph_overview",
            with_response_control(json!({}), &overview_control, json!({"mode":"full"})),
        )
    };
    let warm = full_overview(&mut rpc);
    assert_success(&warm);
    let warm_again = full_overview(&mut rpc);
    let root_a = fixture.root_a.canonicalize().unwrap();
    assert!(result_text(&warm_again).contains(&root_a.to_string_lossy().to_string()));
    assert!(result_text(&warm_again).contains("fixture_000"));
    assert!(!result_text(&warm_again).contains("BetaOnly"));

    activate(&mut rpc, &fixture.root_b);
    let refreshed = full_overview(&mut rpc);
    let root_b = fixture.root_b.canonicalize().unwrap();
    assert!(result_text(&refreshed).contains(&root_b.to_string_lossy().to_string()));
    assert!(result_text(&refreshed).contains("BetaOnly"));
    assert!(!result_text(&refreshed).contains("fixture_000"));
    assert!(result_text(&refreshed).contains("<type name=\"Class\""));
}
