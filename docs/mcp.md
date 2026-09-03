# MCP server

`codingest-mcp` is the code-graph workbench for MCP clients and agents. It
embeds the `kglite-mcp-server` query and source-reading surface and injects the
codingest builder so those tools operate on a freshly built code graph.

```bash
pip install codingest
codingest-mcp --watch /absolute/path/to/repo
```

The Codingest wheel installs this command. Its thin builder composition reuses
KGLite's graph server and the transitive `mcp-methods` MCP infrastructure; no
separate server or Cargo installation is required.

For one repository, add that command to the common MCP client configuration
shape:

```json
{
  "mcpServers": {
    "code-review": {
      "command": "codingest-mcp",
      "args": ["--watch", "/absolute/path/to/repo"]
    }
  }
}
```

The graph is ready when the server starts and is rebuilt lazily after relevant
source changes.

For an agent that switches among several local repositories, create
`workspace_mcp.yaml`:

```yaml
workspace:
  kind: local
  root: /absolute/path/to/starting-repository
  sandbox_root: /absolute/path/to/repository-parent   # the actual boundary
  watch: true

# Served in the MCP `initialize` result. Some hosts (opencode) inject this
# verbatim into the system prompt every session, so keep it short — you pay
# for it on every request.
instructions: |
  This server answers structural questions from a code graph.
  Route: graph_overview first (schema), then cypher_query for anything
  structural — where something is defined, what calls what, which types
  exist, counts, multi-hop paths. Use grep and read_source only for literal
  text (error strings, comments, config keys), never to rediscover
  structure the graph already encodes.
  Project and LIMIT your queries. Some hosts truncate large tool results.
```

Then configure the client command as `codingest-mcp --mcp-config
/absolute/path/to/workspace_mcp.yaml`. This local-workspace mode registers
`set_root_dir`, which lets an agent swap the active repository at runtime.

**`root` is the starting root, not a boundary.** It sets where the server
points initially; on its own it does **not** constrain where `set_root_dir` can
subsequently point. Containment is `sandbox_root`, it is **opt-in**, and it
is supported by every engine codingest admits: with it, a swap outside the boundary is
refused and the active root does not move; without it, a root swap is
unbounded.

> **Correction (2026-07-31).** Earlier revisions of this page stated that "every
> activated repository must stay within the declared sandbox." That was **false
> for every version of codingest that has shipped**. No containment existed: the
> read window was *derived from* the active root, so the source tools bounded
> reads relative to wherever the server already pointed, and never constrained
> where it could be pointed. The boundary those revisions described is real only
> on every engine codingest admits, and only when you set `sandbox_root`. If you relied
> on that sentence to confine an agent to a directory tree, **set `sandbox_root`
> explicitly** — the guarantee you were promised was not being enforced.

## codingest-mcp is the builder; `kglite-mcp-server` alone is not

When KGLite removed its in-tree `code_tree` builder (2026-07-16), the MCP server
lost its ability to build a workspace on its own. The generic server accepts a
`WorkspaceGraphHooks` producer through `ServerExtensions` and **refuses to
build a workspace when no producer is injected** — there is no in-tree
fallback left.

`codingest-mcp` is the process shell that injects those hooks, backed by this
project's builder. So:

- **`codingest-mcp`** builds and serves a code graph from a directory of source.
- **`kglite-mcp-server`** (standalone) still serves an already-built `.kgl`, but
  it will refuse to build a workspace from source.

The injected extension owns one unified plain/revision-set build closure plus
the watch predicate (`is a change to this path graph-relevant?`). Revision
requests return canonical labels together with the graph.
See [MCP parity](mcp-parity.md) for the coupling detail.

## Key tools

- **`set_root_dir(path)`** — in manifest-declared local-workspace mode, switch
  the active root and build that directory's graph.
- **`repo_management(...)`** — clone/activate/update a GitHub repo and build its
  graph (the open-source variant of `set_root_dir`).
- **`graph_overview()` / `cypher_query(...)`** — inventory node types, then
  query structure (calls, types, paths, counts).
- **`read_code_source(qualified_name=…)` / `read_source(...)`** — read the
  underlying source for a graph entity or a file slice.

`CALLS` edges carry `resolution`, `candidates` and `import_backed`, which say
how confidently each edge was resolved — including when *not* to trust
`import_backed`. Same graph, same properties, whichever interface you query
from: see [Interpreting CALLS edges](cli.md#interpreting-calls-edges).

## Watch semantics

With `--watch`, a change to a code file under the fixed root tags the graph
dirty and the rebuild fires lazily on the next graph tool call. In local-
workspace mode, the watcher monitors the wider configured tree but rebuilds only
for changes under the active `set_root_dir` target. (Watch scope is a
performance concern, not a security one — it decides what can *trigger a
rebuild*, never what can be read or activated. Confinement is `sandbox_root`,
above.) The graph is built in memory
and discarded on shutdown — nothing is written to disk unless you ask for it.

## Migrating an MCP config from kglite-mcp-server

Point your MCP client at the `codingest-mcp` binary instead of
`kglite-mcp-server`. Every graph tool behaves identically — the difference is
that `codingest-mcp` can build a workspace from source, which is what the code
graph needs. See [Migrating from kglite.code_tree](migrating-from-kglite-code-tree.md).

## Using codingest-mcp with opencode

Everything below was verified against the shipping `opencode` binary —
`opencode-ai@1.18.11` from npm, which is the git revision tagged
`github-v1.2.25-1505`. Those two numbering schemes both appear in the wild:
`opencode --version` prints the npm one (`1.18.11`), while the repository's own
tags use the `github-v1.2.x` family, so a version that looks far newer than
this page claims is usually the same build. Re-verified 2026-08-02 against
`1.18.11`: handshake, config shape, and the diagnostics below all still hold.

The repository also contains a v2 rewrite that ships as a *different* binary
(`lildax`) and does not wire MCP tools up at all yet. Config key names and
behaviour differ between the two, so if you find guidance elsewhere that does
not match this page, check which binary it describes.

### The zero-absolute-path config

opencode spawns a local MCP server with its working directory set to the
instance directory, and `--watch` canonicalises a relative path. So a single
block in your **global** config (`~/.config/opencode/opencode.json`) gives every
project its own code graph, with no per-repo setup and no absolute paths:

```json
{
  "mcp": {
    "codingest": {
      "type": "local",
      "command": ["codingest-mcp", "--watch", "."]
    }
  }
}
```

The `.` resolves against wherever opencode was started, not against the git
worktree. If you launch opencode from a subdirectory you get a graph of that
subdirectory; if you launch it somewhere like `$HOME`, it will try to build a
graph of your home directory, which you do not want. Two ways to handle it: the
model can call `set_root_dir` (it is told both the working directory and the
workspace root in its environment block), or you can scope the config to the
project.

Project-scoped config lives at `<repo>/.opencode/opencode.json` (or
`opencode.jsonc`) — **not** at the repository root. opencode walks up from the
working directory to the worktree root looking for `.opencode` directories. One
side effect to accept before you create one: for every config directory it
finds, opencode writes a `.gitignore` inside it and forks a background
`npm install @opencode-ai/plugin` into it. If that is unwelcome in your repo,
stay on global config and let the model call `set_root_dir`.

For the multi-repo case, use the manifest as documented above:
`["codingest-mcp", "--mcp-config", "/absolute/path/to/workspace_mcp.yaml"]`.

`--watch` with no manifest sends no `initialize` instructions at all, so the
routing doctrine never reaches opencode's system prompt. You do not have to give
up the zero-path config to get it back: `--watch DIR` also picks up a
`workspace_mcp.yaml` sitting in `DIR`. Commit one at the repository root carrying
just an `instructions:` block and the global config block above serves it
per-repo, with no per-repo client setup.

### Config shape: use these keys

opencode's shipping binary reads `type: "local"`, an argv array `command`,
optional `cwd` and `environment`, `enabled` (not `disabled`), a **flat numeric**
`timeout`, and camelCase OAuth keys (`clientId`, `callbackPort`, …).

The v2 rewrite uses different names for the same things — `mcp.servers` nesting,
`disabled`, an object `timeout: {startup, request}`, snake_case OAuth. Do not mix
them in: config is validated against the V1 schema, and a shape mismatch is a
**hard startup failure**, not a skipped server. The same is true of an unknown
**top-level** key, so a typo one level up stops opencode from starting at all.
(The narrow exception is an entry that is only `{"enabled": true}` with no
`type`: that one is logged and skipped.)

Copy the block above by hand. codingest does not generate or edit opencode
config, deliberately: a tool that writes another tool's config file is one typo
away from making it unbootable.

### Budget your query results

opencode truncates every tool result at **2000 lines or 50 KiB**, whichever
comes first, keeping the *head* and spilling the remainder to a file whose path
it appends to the preview.

`cypher_query`'s default rendering will not trip this on row count alone: the
server already prints a **15-row preview** with the true row count in the header
(`38 row(s) (showing first 15):`). What can trip it:

- **`FORMAT CSV`** carries the header plus the first **200 data rows** inline
  (capped since kglite 0.16.6), then a notice naming the true row count, the
  full byte size, and the `csv_http_server` extension that serves the complete
  file as a fetch URL. It is still the widest per-row rendering the server
  produces, so 200 rows can reach the 50 KiB cap on their own — narrow the query
  rather than re-running it hoping for more. The CLI's `codingest query --format
  csv` is uncapped: it writes every row to stdout, not into a tool result.
- Wide projections. `RETURN n` on a node type carrying source text can blow the
  byte budget inside 15 rows. Project the columns you actually need.
- `read_source` / `grep` over large files.

If you routinely need more headroom, raise it in opencode's config:
`"tool_output": {"max_lines": 5000, "max_bytes": 200000}`. When output *is*
truncated, opencode's message points at the spill file — and, if the agent has
the task tool, tells the model to delegate reading it to a sub-agent rather than
pull it back into context.

Two changes in effect since kglite 0.16.6 alter what a runaway or capped
query does. A deep
unbounded traversal is charged against the 10,000,000-row internal ceiling *as
it expands*, so it now stops with a quantified error naming the expansion that
overflowed instead of exhausting the host's memory (`max_rows` still governs on
its own if you set it). And a `LIMIT`-bearing relationship pattern no longer
truncates silently: the executor's candidate seed caps are advisory, and a pass
that hits one and comes back short of the `LIMIT` is re-run without them — a
short result is the graph's answer, not the cap's.

### Timeouts

The real default is **30 seconds**, applied to both the initial connection and
every subsequent request. (opencode's own documentation says 5 s; the shipping
code uses 30 s.) In `--watch` mode the graph is built before the server begins
answering, so a repository that takes longer than 30 s to build will fail to
connect. Raise `timeout` on that server entry if so.

Long-running calls can survive past the timeout if the server reports progress.
`codingest-mcp` does not currently emit progress notifications, so a slow query
is bounded by `timeout` regardless.

### Skills: already installed

If you have run `codingest skill install --host claude` (globally or with
`--project`), **opencode already discovers it** — it reads `.claude/skills`
alongside its own locations. There is nothing extra to install.

Two things worth knowing:

- **Do not also copy the skill into a `skill/` or `skills/` directory under
  `.opencode/` or `~/.config/opencode/`.** opencode scans those too, and two
  discovered copies of the same skill name resolve nondeterministically — you
  will get one of them and cannot predict which.
- If you turn on opencode's tool-output pruning (`"compaction": {"prune": true}`,
  off by default), skill output is exempt from pruning and MCP tool output is
  not. So durable methodology belongs in the skill, and volatile evidence
  belongs in tool calls.

If you have set `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS` (or the broader
`OPENCODE_DISABLE_CLAUDE_CODE`), point opencode at the skill explicitly instead:

```json
{ "skills": { "paths": ["~/.claude/skills/codingest-code-review"] } }
```

### The `skills:` manifest key is a real tradeoff

Leaving it unset (the default) gives lean tool descriptions. Setting

```yaml
skills:
  - true
```

in the manifest attaches the bundled methodology to the tool descriptions, which
are sent verbatim and uncapped on **every** request. Measured against this
project's own graph: total tool-description size goes from **3.3 KB to 40.5 KB**
across nine tools. In exchange, seven skills are served as MCP `prompts`, which
opencode turns into slash commands named `/<server>:<skill>` — for an `mcp` entry
keyed `codingest`, that is `/codingest:cypher_query`, `/codingest:graph_overview`,
`/codingest:grep`, `/codingest:read_source`, `/codingest:list_source`,
`/codingest:github_issues`, `/codingest:repo_management`. Other bundled skills are
gated on what the active graph contains and may not surface as commands at all,
so treat that list as the shape rather than a guarantee.

Worth it if you use those commands; expensive if you do not. Two ways to opt in
more cheaply:

- The manifest's `<basename>.skills/` override directory (auto-detected next to
  the YAML) replaces individual skill bodies by name, so you can swap a long
  methodology file for a short pointer.
- opencode's experimental code mode (`OPENCODE_EXPERIMENTAL_CODE_MODE`) collapses
  all MCP tools into a single `execute` tool with a budgeted catalog instead of
  passing every description through.

### Which root mechanism to use

| You want | Use |
|---|---|
| watch one repository | `--watch /path` (or `--watch .`, above) |
| switch between repositories at runtime | manifest + `set_root_dir` |
| serve a prebuilt graph artifact | `--graph /path/to/graph.kgl` |
| build once in CI, no server | `codingest build` |
| one-shot query in CI, no server | `codingest query` (see [cli.md](cli.md)) |

A `--graph` server re-reads the served `.kgl` automatically:
every tool call stats the file, and when its identity has changed on disk — a
`codingest build` that replaced the artifact, say — the server re-reads it
before answering, with no `reload_graph` round-trip. Write-enabled servers take
the file's writer lease only at their first unsaved change, so several clients
can boot off one manifest; `--lease-label` (or `KGLITE_LEASE_LABEL`) names this
server in the refusal a peer sees while it holds the lease. A write-enabled
server's `save_graph` with nothing unsaved is a no-op — it
answers `Nothing to save: <path>` and leaves the artifact's bytes, mtime and
inode alone, so peer servers bound to the same file do not pay a re-read for a
save that changed nothing; pass `force: true` to rewrite it anyway. `force` is
part of the write opt-in: a server that registers `save_graph` alone (via
`builtins.save_graph: true`) refuses it, because a forced re-encode moves the
file's identity and makes every peer serving it pay a full re-read.

Every answer carries an identity footer — `— active graph: <root> · built …
· file saved <T> · load N · <state>`, matched by the `<active_graph …
file_saved="<T>" load="N" state="…">` header.
That counter is spelled `load`, not `generation`: it counts
the graphs *this server process* has installed since boot, so two servers on one
artifact legitimately report different numbers for the same bytes. The identity
two servers *can* compare is `file saved`, the served path's publish time off
the filesystem; a workspace graph (`--watch`, `set_root_dir`) has no publish
moment and omits it. Anything parsing `· generation ` or `generation="` must
switch — there is no compatibility spelling. codingest itself parses none of it.

Two ways to enable writes: `--writable` on the command line, or
`extensions.writable: true` in the manifest — either alone
opens `cypher_query` to mutations and registers the lifecycle tools.
`builtins.save_graph: true` is **not** a third way: it registers `save_graph`
only, so the server can persist what it loaded while `cypher_query` stays
read-only.

One precedence surprise: a manifest declaring `workspace: {kind: local}` wins
over the mode flags — supply one and the server runs in local-workspace mode
with that manifest's `root`, whatever `--watch` or `--graph` said.

### When it does not work

Start with **`opencode mcp list`**. It connects the configured servers and prints
each one's state, and for a failed server it prints the connection error inline.
That is the fastest signal, and usually enough: the most common cause is a build
that exceeds the 30 s timeout.

If you need more than the connection error, be aware of where the output does
*not* go. opencode spawns the server with its stderr on a pipe that nothing
reads, so anything `codingest-mcp` writes to stderr is discarded — it reaches
neither your terminal nor opencode's log. Run opencode with `--print-logs` to see
opencode's own log on stderr, which is where MCP protocol-level server log
notifications land.

`opencode mcp debug <name>` exists but will not help here: it debugs OAuth for
**remote** servers and refuses a local one.

To rule out the codingest side independently of opencode, run
`codingest-mcp --selftest` with the same flags. It re-spawns the binary, drives a
real handshake, and exits non-zero if anything fails.
