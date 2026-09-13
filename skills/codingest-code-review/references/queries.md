# Code-review query patterns

Use these recipes only when the relationship answers the current question.
Resolve exact qualified identities first, establish per-target coverage, then
read the selected source. Do not fetch callers, callees, paths, type consumers,
and implementations as a routine inventory.

The examples use `codingest query` because it is installed with this skill and
returns complete JSON/CSV. Set `GRAPH` to the saved `.kgl` artifact. Each query
is sent on stdin through a quoted heredoc, so the shell cannot expand `$`,
backticks, or command substitutions inside the Cypher. The CLI has no parameter
binding flag: replace example literals only with trusted graph/source
identifiers, and escape a Cypher string quote as `''`.

For MCP, send the Cypher body to `cypher_query`. Replace a CLI literal such as
`'crate::src::graph::session::execute_mut'` with `$target` and pass
`params={"target":"crate::src::graph::session::execute_mut"}`; MCP parameters bind
values as data. Inspect `graph_overview` first when a label, property, or edge
shape is not already known.

Resolve requested targets and distinguish a missing target (`target` is null)
from an existing target with no callers (`target` is present and `callers` is
zero):

<!-- codingest-query: target-coverage -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
UNWIND ['crate::src::graph::session::execute_mut',
        'crate::src::graph::session::unused_target',
        'crate::src::graph::session::missing_target'] AS requested
OPTIONAL MATCH (target:Function {qualified_name: requested})
OPTIONAL MATCH (caller:Function)-[:CALLS]->(target)
RETURN requested, target.qualified_name AS target,
       count(caller) AS callers
ORDER BY requested
CYPHER
```

Count production and test callers before requesting pages. Keep different
targets in different queries: a global `LIMIT` shared by read and write targets
can fill with one target and hide the other.

<!-- codingest-query: write-caller-coverage -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH (caller:Function)-[:CALLS]->
      (target:Function {qualified_name: 'crate::src::graph::session::execute_mut'})
RETURN target.qualified_name AS target, caller.is_test AS is_test,
       count(caller) AS callers
ORDER BY is_test
CYPHER
```

<!-- codingest-query: read-caller-coverage -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH (caller:Function)-[:CALLS]->
      (target:Function {qualified_name: 'crate::src::graph::session::execute_read'})
RETURN target.qualified_name AS target, caller.is_test AS is_test,
       count(caller) AS callers
ORDER BY is_test
CYPHER
```

Fetch production and test pages separately. Project exact identity and source
location plus the evidence that qualified each CALLS edge. `candidates > 1`
means a call site fanned out; `import_backed = false` is unconfirmed, not
refuted. `call_lines` locates call sites in the caller.

<!-- codingest-query: production-callers -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH (caller:Function)-[rel:CALLS]->
      (target:Function {qualified_name: 'crate::src::graph::session::execute_mut'})
WHERE caller.is_test = false
RETURN target.qualified_name AS target, caller.qualified_name AS caller,
       caller.file_path AS file, caller.line_number AS line,
       rel.resolution AS resolution, rel.candidates AS candidates,
       rel.import_backed AS import_backed, rel.call_lines AS call_lines
ORDER BY caller, file, line
SKIP 0 LIMIT 25
CYPHER
```

<!-- codingest-query: test-callers -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH (caller:Function)-[rel:CALLS]->
      (target:Function {qualified_name: 'crate::src::graph::session::execute_mut'})
WHERE caller.is_test = true
RETURN target.qualified_name AS target, caller.qualified_name AS caller,
       caller.file_path AS file, caller.line_number AS line,
       rel.resolution AS resolution, rel.candidates AS candidates,
       rel.import_backed AS import_backed, rel.call_lines AS call_lines
ORDER BY caller, file, line
SKIP 0 LIMIT 25
CYPHER
```

Raise `LIMIT` or advance `SKIP` when the preceding count says more rows exist.
A complete page does not prove the query covered another target or an edge the
builder did not emit. In particular, an unresolved source call can have no
CALLS edge; inspect the selected source before concluding that a call is absent.

Inspect direct callees when the uncertainty runs in the other direction:

<!-- codingest-query: direct-callees -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH (caller:Function {qualified_name: 'crate::src::service::write_entry'})
      -[rel:CALLS]->(callee:Function)
RETURN caller.qualified_name AS caller, callee.qualified_name AS callee,
       caller.file_path AS caller_file, caller.line_number AS caller_line,
       callee.file_path AS callee_file, callee.line_number AS callee_line,
       rel.resolution AS resolution, rel.candidates AS candidates,
       rel.import_backed AS import_backed, rel.call_lines AS call_lines
ORDER BY callee, callee_file, callee_line
LIMIT 25
CYPHER
```

Ask for a small number of bounded path witnesses between two resolved symbols,
rather than an unbounded neighborhood:

<!-- codingest-query: bounded-call-path -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH path = (start:Function {qualified_name: 'crate::src::service::path_start'})
             -[:CALLS*1..4]->
             (finish:Function {qualified_name: 'crate::src::service::path_finish'})
RETURN [step IN nodes(path) | step.qualified_name] AS functions,
       length(path) AS hops
ORDER BY hops, functions
LIMIT 10
CYPHER
```

For type impact through explicit parameters, include `both` because one edge
aggregates a type used as both a parameter and a return value:

<!-- codingest-query: type-consumers -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH (consumer:Function)-[use:USES_TYPE]->
      (used:Struct {qualified_name: 'crate::src::model::Request'})
WHERE use.position IN ['parameter', 'both']
RETURN used.qualified_name AS used_type,
       consumer.qualified_name AS consumer,
       consumer.file_path AS file, consumer.line_number AS line,
       use.position AS position
ORDER BY consumer, file, line
LIMIT 25
CYPHER
```

Ask for trait implementations only when implementation coverage is the
question. Macro-generated or heuristic structure may be absent from the graph,
so a missing edge is a reason to inspect source, not proof of no implementation.

<!-- codingest-query: trait-implementations -->
```sh
codingest query --graph "$GRAPH" --format json - <<'CYPHER'
MATCH (implementor:Struct)-[:IMPLEMENTS]->
      (implemented:Trait {qualified_name: 'crate::src::service::Handler'})
RETURN implemented.qualified_name AS trait,
       implementor.qualified_name AS implementor,
       implementor.file_path AS file, implementor.line_number AS line
ORDER BY implementor, file, line
LIMIT 25
CYPHER
```

For a multi-revision graph, use the built-in delta procedure shown by
`graph_overview`/`describe`:

```cypher
CALL rev_diff({from: '<base>', to: '<head>'})
YIELD bucket, type, qualified_name, name, file, line
RETURN bucket, type, qualified_name, name, file, line
ORDER BY bucket, type, qualified_name
```

`=~` matches the whole value. Use `CONTAINS` for substring search or include
`.*` explicitly. Treat query warnings about unknown properties or reversed
relationships as evidence that the query shape must be corrected before its
empty result can support a conclusion.
