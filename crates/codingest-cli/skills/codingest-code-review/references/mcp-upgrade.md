# When to use the MCP server

The CLI skill is the low-friction path: no agent configuration, one process per
command, and a review artifact that can be rebuilt explicitly.

Upgrade to `codingest-mcp` when the work benefits from:

- a graph kept warm across many queries;
- watch mode and automatic refresh after file changes;
- typed tool input/output schemas rather than shell quoting;
- switching among several repository roots;
- cached public-repository lifecycle and GitHub/source tools; or
- long collaborative sessions where process startup becomes noticeable.

The local-code-review MCP workspace covers a changing checkout. The
open-source workspace covers cached public repositories and adds constrained
source and GitHub tooling. Codingest builds the code graph; both workflows use
KGLite's graph engine, query language, and read-tool surface.

## Expand retained MCP evidence

A bounded presentation is a view of a completed retained result. It does not
change query semantics and expansion does not rerun the original tool. A
Cypher `LIMIT`, an executor row limit, or a tool-level omission is different:
excluded rows require another reviewed query.

Use the response itself as the protocol contract:

1. Discover the query tool's response-control property and the expansion tool
   through the current session's tool listing. Names may be changed to avoid a
   collision, so do not assume `_response` or `expand_response`.
2. When the result is bounded, copy its complete advertised
   `next.selected_value` action. Keep the action's `name` and
   `arguments.result_id` unchanged.
3. Choose a target actually advertised by the response. Apply its patch by
   copying `json_pointer`, `offset`, and `response` to the action arguments
   named `path`, `offset`, and `response`. Do not infer paths or semantic groups
   that are absent from the returned navigation data.
4. Dispatch the patched action and use the returned value as evidence from the
   original completed result. Follow an advertised page action when more of the
   same selected value is required.

Within an already authorized read-only investigation, selecting an advertised
page, requesting a larger per-call budget, or requesting an advertised full
response needs no additional human approval. Copy the discovered control field
and supported shape into that individual call. Do not turn expansion into a
replay of the original query, especially when that tool may mutate state.

Retained MCP results are short-lived, held in memory, and scoped to the current
MCP session. Honor the retention and eviction facts in the response. If a
handle is unavailable, rerun only a read that remains authorized and whose
preconditions are still current.

## External CLI agent output is optional

The examples below require an external `kglite` CLI satisfying
`>=0.18.0,<0.19`:

```console
kglite --version
```

The following source-composed example is exercised against a generated graph by
the packaged-consumer test:

<!-- example: kglite-agent-query -->
```console
kglite query <graph> "RETURN 'agent-evidence' AS evidence" --format agent
```

Agent output advertises executable expansion commands. Copy the returned
command and its result ID; do not construct either. KGLite's CLI retention uses
a private disk cache, so expansion can run in a later process. This lifecycle
is separate from the MCP server's in-memory, session-scoped retention.

If the compatible external CLI is unavailable, report the agent example as
untested. Codingest's own query command remains the portable complete-output
path and does not require response retention:

```console
codingest query "MATCH (n:Evidence) RETURN n.id ORDER BY n.id" \
  --graph app.kgl --format json
codingest query "MATCH (n:Evidence) RETURN n.id ORDER BY n.id" \
  --graph app.kgl --format csv
```

Codingest JSON and CSV contain every row produced by the query and executor.
They do not make a literal `LIMIT` or another execution boundary broader.
