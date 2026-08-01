# CLI

The `codingest` command builds a `.kgl` code graph from a checkout or from git
revision(s), checks whether an existing graph is stale, and runs one-shot Cypher
queries against it. It's a port of KGLite's former `kglite code-tree`
subcommand.

Two ways to install the exact same command:

```bash
pip install codingest           # the Python wheel bundles the `codingest` command
# or
cargo install codingest-cli     # pure-Rust binary, no Python
```

The wheel links the `codingest-cli` Rust library into its extension and exposes
it through a console-script shim, so `pip install codingest` gives you the same
`codingest build`/`status`/`query` command as `cargo install codingest-cli` — no
separate install. Its KGLite dependency also installs the `kglite` command, so
the pip-only flow `pip install codingest && codingest skill install` is
self-sufficient.

## `codingest skill`

Codingest owns and distributes the code-review Agent Skill because the skill
drives Codingest's builder lifecycle. Install it for Codex and Claude Code at
user scope with:

```bash
codingest skill install
```

Use repeated `--host codex` / `--host claude` flags to select hosts, `--project`
for repository-local installation, and `--dry-run` to preview changes. Use
`codingest skill uninstall` to remove an installation managed by Codingest.

Installation migrates an old `kglite-code-review` directory only when it bears
KGLite's managed marker. Codingest refuses to replace its own unmanaged target
and leaves unmanaged legacy directories untouched.

## `codingest build`

Parse a directory into a graph and write it to a `.kgl`:

```bash
codingest build /path/to/repo
# → /path/to/repo/.kglite/code-review.kgl
```

Build committed content at specific git revisions — a **multi-rev merged
graph** with one node per entity across revs (each node carries `revs: [str]`
membership; scope a query with `WHERE 'v2.0' IN n.revs`):

```bash
codingest build /path/to/repo --revs v1.0 v2.0
```

Common options:

- `--output <path>` — where to write the `.kgl` (default `<repo>/.kglite/code-review.kgl`).
- `--rev <revspec>` — build a single git revision instead of the working tree.
- `--revs <r1> <r2> …` — merge several revisions into one graph.
- `--include-docs` — also ingest markdown as `:Doc` nodes linked to the code
  they mention.
- `--no-tests` — exclude test files/dirs from the graph.

Run `codingest build --help` for the full flag list.

## `codingest status`

Report whether a previously-built graph is stale relative to the current tree
(so a wrapper can decide whether to rebuild):

```bash
codingest status --output /path/to/repo/.kglite/code-review.kgl
```

## `codingest query`

Run a read-only Cypher query against a saved graph and print the rows:

```bash
codingest query "MATCH (f:Function)-[:CALLS]->(g:Function) RETURN f.name, g.name LIMIT 20"
```

`codingest cypher` is a visible alias for the same subcommand.

**It queries an artifact; it never builds one.** Build-then-query is shell
composition, which keeps each step's flags and freshness semantics its own:

```bash
codingest build . && codingest query "MATCH (f:File) RETURN count(f)"
```

That is also why loading is the right default: a `.kgl` load is far cheaper
than a rebuild, and CI typically queries the same artifact many times. Reads
take no writer lease and writers replace the `.kgl` atomically, so `query` is
safe to run while a rebuild is in flight or while `codingest-mcp --graph`
serves the same file.

Options:

- `-g/--graph <path>` — the artifact to query (default
  `.kglite/code-review.kgl`, relative to the current directory). Note the
  asymmetry with `build`/`status`, which spell the same file `--output`: for
  `query` the artifact is an *input*, and `--graph` matches the vocabulary
  `codingest-mcp --graph` already uses.
- `--format human|csv|json` — see below (default `human`).
- `--timeout <secs>` — abort the query after this long; a query that actually
  hits the deadline exits `1`. The value must be **positive and finite**
  (at most `1e9` seconds). Zero, a negative, `nan`/`inf` and anything past that
  bound are rejected as usage errors (exit `2`) — "no timeout" is spelled by
  omitting the flag, so a `--timeout=0` can only be a mistake.
- `--require-fresh` — refuse to query a graph that is not provably fresh.
- `-` in place of the query reads the query text from stdin:

  ```bash
  codingest query - <<'CYPHER'
  MATCH (f:Function) WHERE f.branch_count > 10
  RETURN f.qualified_name, f.branch_count ORDER BY f.branch_count DESC
  CYPHER
  ```

### Output

**Nothing is ever truncated.** The MCP server's 15-row inline preview is a
host-context budget; a pipe has no such constraint, so row budgeting here is
Cypher's job — use `LIMIT`.

- `human` (default) — a header line of column names, then one tab-separated row
  per result row, on stdout. String cells print raw with `\t`, `\n` and `\r`
  escaped; other values render as compact JSON. The `N row(s)` summary goes to
  **stderr**, so stdout stays pure data and pipes cleanly.
- `csv` — `CypherResult::to_csv()` verbatim, byte-identical to what the MCP
  server exports for a `FORMAT CSV` query.
- `json` — one compact `{"columns": [...], "rows": [[...]]}` object, the same
  single-line convention `status --format json` uses.

A query that ends in `FORMAT CSV` is a parser-level output switch and
**overrides `--format`**, matching MCP behaviour so one query renders the same
on both interfaces. `EXPLAIN` works too — the plan rows print like data rows.

Mutation Cypher (`CREATE`/`SET`/`DELETE`/`MERGE`/…) is rejected by the engine's
read path. The CLI adds no policy layer of its own.

### Freshness

Before executing, `query` runs the same sidecar check as `codingest status` and
**warns on stderr** — it does not refuse:

```
warning: graph is stale: source changed since the graph was built
```

Three non-fresh outcomes all warn: the source changed, the `.meta.json` sidecar
is missing (a foreign or hand-copied `.kgl`), or freshness **could not be
verified at all** — the check itself fails when the recorded source directory is
unreadable or a recorded git revision no longer resolves, which is exactly what
a `.kgl` copied to another machine hits. Rows are still returned in every case;
refusing by default would break the copied-artifact and CI-cache workflows the
CLI exists for.

`--require-fresh` inverts that for CI: any non-fresh *or* unverifiable outcome
becomes a hard refusal with **exit code 3**, and no rows are printed.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | success |
| `1` | operational error — missing artifact, bad Cypher, timeout, I/O |
| `2` | usage error (clap) |
| `3` | `--require-fresh` refused a non-fresh or unverifiable graph |

**Wheel caveat:** the `pip install codingest` console script maps every error to
`PyRuntimeError`, so a stale refusal exits `1` there. Use the cargo binary
(`cargo install codingest-cli`) where the distinction matters.

## Querying the result elsewhere

The output is an ordinary kglite `.kgl`, so any kglite surface reads it too:

```bash
kglite query /path/to/repo/.kglite/code-review.kgl \
  "MATCH (f:Function)-[:CALLS]->(g:Function) RETURN f.name, g.name LIMIT 20"
```

or in Python, `kglite.load("…/.kglite/code-review.kgl")`.

## Accuracy harness

The `codingest` library crate also ships `codingest_stats`, a JSON accuracy
harness (CALLS-resolution stats + node/edge counts) used by the determinism
gate:

```bash
cargo run -p codingest --bin codingest_stats --release -- /path/to/repo
```
