---
name: codingest-code-review
description: Use when reviewing a code change or answering structural questions about a codebase, including definitions, callers, dependencies, routes, affected tests, and history across git revisions. Builds a local Cypher-queryable code graph, uses it alongside the diff and literal search, and verifies every finding against exact source lines.
---

# Codingest code review

Use Codingest for structural evidence during review. The graph complements the
git diff, source reading, and literal-text search; it does not replace them.

## Review workflow

> **Prerequisites:** the `codingest` and `kglite` CLIs must be on PATH.
> `pip install codingest` provides both plus the builder-aware
> `codingest-mcp` server. Rust-only environments can alternatively use Cargo.

Inspect the diff and repository guidance first. Identify changed symbols and
the base/head revisions, then build or refresh the graph without executing
repository code:

```bash
codingest build . --output .kglite/code-review.kgl --format json
```

For a committed comparison, use one graph spanning both revisions:

```bash
codingest build . --revs '<base>' '<head>' \
  --output .kglite/code-review.kgl --format json
```

Before reusing an artifact, check freshness:

```bash
codingest status --output .kglite/code-review.kgl --format json
```

## Retrieve only what answers the question

Choose the narrowest route for the uncertainty in front of you; these are
alternatives, not a compulsory sequence. Reuse tool signatures already known
in the current session; if a needed tool is not visible, discover that tool
rather than loading a broad catalog.

- **Known symbol or location:** read it directly. With MCP, use
  `read_code_source` for a qualified name or `read_source` for a path, applying
  `start_line`, `end_line`, and `max_chars` when a full body or file is not
  needed.
- **Exact structural question:** use `cypher_query`, or `kglite query` at the
  CLI. Reuse a schema already observed for the same unchanged graph. Otherwise
  inspect only the needed node or connection shape with `graph_overview` or
  `kglite describe`; do not retrieve the whole schema by habit.
- **Broad explanation or unfamiliar subsystem:** use bounded `explore`, starting
  with a short topic, a few `max_entities`, shallow `max_depth`, and
  `include_source: false` unless bodies are needed. Narrow to selected symbols
  before reading source.
- **Literal text:** use grep/ripgrep for error strings, comments, and config
  keys, then open only the relevant matches.

Every additional read should resolve a concrete uncertainty needed for the
answer. Project only useful Cypher properties, return a few relevant candidates,
and exclude unrelated fixtures, generated code, vendors, or examples when the
question does not cover them. If output truncates, narrow the query or source
range instead of repeatedly increasing output. Reuse evidence already seen
unless its source or active graph changed.

Open implicated code at exact lines before claiming behavior; an edge alone is
not runtime proof. Stop when the requested conclusions are supported. Expand
only for a contradiction, missing evidence, or a correctness risk that could
change the answer.

See [queries.md](references/queries.md) for query patterns,
[public-repositories.md](references/public-repositories.md) for safe public-repo
review, and [mcp-upgrade.md](references/mcp-upgrade.md) for the persistent MCP
workflow.

## What counts as a finding

The workflow above verifies a finding against exact source lines. This is the
prior question — what is eligible to be a finding at all.

- **A finding names a concrete failure**: the input, state, or sequence, and the
  wrong outcome it produces. A wrong result, a crash, data loss or corruption, a
  broken contract with a caller or a persisted format, a security hole, a
  *measured* performance regression, a check that cannot fail, or a claim the
  code contradicts. **"No findings" is a valid review**, and a good one.
- **Design, structure, naming, "consider using X", and "this won't scale" are
  not findings** — they are mis-staged. Their venue is planning, where "I would
  have designed this differently" is invited and settled before the code exists.
  After a plan is approved, review measures the implementation against that plan
  and against correctness, never against a design the reviewer would have
  preferred. A design opinion formed while reading a diff is input to the *next*
  plan.
- **A finding that cannot state its failure case is removed, not downgraded.**
  Severity labels are how a preference gets laundered into a report: "Minor:
  consider extracting this" is a preference wearing a label.
- **One narrow exception**: citing a rule the project declared *before* the diff
  existed — a documented ceiling, a boundary rule, a checklist — naming both the
  rule and the violating line. That is enforcement, not taste.
- **A review tool's effort or confidence level is orthogonal.** A higher level
  buys more *speculative bugs*; it never buys permission to report preferences.
- **A graph edge showing coupling is a fact, not a defect.** Structural evidence
  answers "what would this change reach"; it does not by itself establish that
  anything is wrong.

## Honesty rules

- Never invent labels, properties, or connection types. Discover an unfamiliar
  shape before querying it; reuse a known shape while the graph is unchanged.
- Treat unresolved or missing graph edges as absence of evidence, not proof.
- Quote paths and revisions passed through the shell.
- Never build, import, or execute code from a repository merely to review it.
- Use grep/ripgrep for exact tokens and the graph for relationships and impact.
