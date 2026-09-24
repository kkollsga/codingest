---
name: codingest-code-review
description: Use when reviewing a code change or answering structural questions about a codebase, including definitions, callers, dependencies, routes, affected tests, and history across git revisions. Builds a local Cypher-queryable code graph, uses it alongside the diff and literal search, and verifies every finding against exact source lines.
---

# Codingest code review

Use Codingest for structural evidence during review. The graph complements the
git diff, source reading, and literal-text search; it does not replace them.

## Review workflow

> **Prerequisites:** the `codingest` CLI must be on PATH. `pip install
> codingest` also provides the builder-aware `codingest-mcp` server and a
> compatible graph engine. Rust-only environments can alternatively use Cargo.
> Examples that invoke the external `kglite` CLI require a compatible
> `kglite>=0.18.0,<0.19`; verify `kglite --version` before using its
> agent-response features.

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

## Work from the question

Use this loop only as far as the question requires. It has no required map set
or fixed query count.

1. **Establish target and coverage.** Record the exact repository, revision,
   symbol, path, or subsystem in scope. For a completeness or absence claim,
   identify the relevant groups before retrieving their rows and record every
   query, executor, or presentation boundary.
2. **Retrieve a relevant relation.** Ask the smallest structural question that
   can change the answer: callers, callees, implementers, imports, dependencies,
   routes, tests, or revision identity.
3. **Inspect selected source.** Open the implicated definitions and call sites
   at exact lines. Graph edges identify candidates; source and executed tests
   establish behavior.
4. **Retain an evidence map.** Keep each working claim with its graph revision,
   exact query or reference, concrete example or input state, what an assertion
   actually guarantees, and any unresolved question. These fields can be a
   compact note rather than separate documents.
5. **Resolve consequential uncertainty.** Expand only when a contradiction,
   missing precondition, or correctness risk could change the answer. Stop when
   the requested conclusions have direct support.

Choose the narrowest route for the uncertainty in front of you; these are
alternatives, not a compulsory sequence. Reuse tool signatures already known
in the current session. If a needed tool is not visible, discover that tool
rather than loading a broad catalog.

- **Known symbol or location:** read it directly. With MCP, use
  `read_code_source` for a qualified name or `read_source` for a path, applying
  `start_line`, `end_line`, and `max_chars` when a full body or file is not
  needed.
- **Exact structural question:** use `cypher_query`, or `kglite query` when the
  compatible external CLI is installed. Reuse a schema already observed for
  the same unchanged graph. Otherwise inspect only the needed node or
  connection shape with `graph_overview` or `kglite describe`; do not retrieve
  the whole schema by habit.
- **Broad explanation or unfamiliar subsystem:** use bounded `explore`, starting
  with a short topic, a few `max_entities`, shallow `max_depth`, and
  `include_source: false` unless bodies are needed. Narrow to selected symbols
  before reading source.
- **Literal text:** use grep/ripgrep for error strings, comments, and config
  keys, then open only the relevant matches.

Every additional read should resolve a concrete uncertainty needed for the
answer. Project only useful Cypher properties, return a few relevant candidates,
and exclude unrelated fixtures, generated code, vendors, or examples when the
question does not cover them. Reuse evidence already seen unless its source or
active graph changed.

Treat response presentation separately from query coverage. When an MCP
response says the completed result was retained, follow its advertised action
to expand the needed value or page within the authorized read-only
investigation; this does not require separate human approval. Expansion cannot
recover rows excluded by a Cypher `LIMIT`, an executor row limit, or tool-level
omission. Broaden or regroup the query when those boundaries matter. Narrow a
source range when only a source read was clipped. See
[mcp-upgrade.md](references/mcp-upgrade.md) for the expansion procedure.

Macro-generated structure may not exist as source-level graph nodes, and
heuristic relationships are candidates rather than runtime proof. Inspect the
source or an executed test before turning either limitation into a behavioral
claim.

## Verify the answer

Before reporting a finding or a consequential conclusion, use focused reads to
check:

- each required precondition holds at the reviewed revision;
- the stated input, state, or sequence reaches the claimed behavior;
- a returned value or error observed below the claimed API boundary is traced
  through catches, fallbacks, retries, and active options to that boundary;
- one observed path is used as evidence for that case, not as a universal
  outcome;
- each cited assertion runs on that input and guarantees what the answer says;
- every file and line reference still points to the supporting code; and
- completeness, absence, ownership, and reachability claims are limited to the
  coverage actually established.

Label source-composed examples that were not executed. Leave unresolved
questions explicit rather than converting them into findings.

See [queries.md](references/queries.md) for query patterns,
[public-repositories.md](references/public-repositories.md) for safe public-repo
review, and [mcp-upgrade.md](references/mcp-upgrade.md) for the persistent MCP
workflow.

## What counts as a finding

The workflow above verifies a finding against exact source lines. This is the
prior question: what is eligible to be a finding at all.

- **A finding names a concrete failure**: the input, state, or sequence, and the
  wrong outcome it produces. A wrong result, a crash, data loss or corruption, a
  broken contract with a caller or a persisted format, a security hole, a
  *measured* performance regression, a check that cannot fail, or a claim the
  code contradicts. **"No findings" is a valid review**, and a good one.
- **Design, structure, naming, "consider using X", and "this won't scale" are
  not findings** — they are mis-staged. Their venue is planning, where "I would
  have designed this differently" is invited and settled before code exists.
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

- Never invent labels, properties, connection types, response-control fields,
  expansion-tool names, or retained-result IDs. Discover unfamiliar shapes and
  controls; reuse them only while the graph and session remain unchanged.
- Treat unresolved or missing graph edges as absence of evidence, not proof.
- Quote paths and revisions passed through the shell.
- Never build, import, or execute code from a repository merely to review it.
- Use grep/ripgrep for exact tokens and the graph for relationships and impact.
