# CLI

The `codingest` command builds a `.kgl` code graph from a checkout or from git
revision(s), and checks whether an existing graph is stale. It's a port of
KGLite's former `kglite code-tree` subcommand.

Two ways to install the exact same command:

```bash
pip install codingest           # the Python wheel bundles the `codingest` command
# or
cargo install codingest-cli     # pure-Rust binary, no Python
```

The wheel links the `codingest-cli` Rust library into its extension and exposes
it through a console-script shim, so `pip install codingest` gives you the same
`codingest build`/`status` command as `cargo install codingest-cli` — no
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

## Querying the result

The output is an ordinary kglite `.kgl` — query or inspect it with any kglite
surface:

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
