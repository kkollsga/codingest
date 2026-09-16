# Reviewing a codingest code graph

Every graph this server serves was built by codingest from source, so its
shapes are fixed: `Function`, `Struct`, `Class`, `Trait`, `Interface`,
`Protocol`, `Enum`, `Module` and `File` nodes keyed by `qualified_name` (plus
`Doc` when built with docs), with `CALLS`, `USES_TYPE`, `IMPLEMENTS`,
`EXTENDS`, `HAS_METHOD`, `IMPORTS`, `DEFINES` and `HAS_FILE` edges. Functions carry `file_path`, `line_number` and `is_test`. Multi-revision
graphs add a `rev` dimension; `graph_overview` shows it when present.

The graph gives structural evidence. It complements the diff, the source and
literal-text search; it does not replace them. Use it only as far as the
question requires — there is no fixed map set or required query count.

## Sequence

1. **Resolve targets first.** Run `run_recipe_query("code_review",
   "target_coverage", {"requested": [...]})` with the exact qualified names in
   scope. A null `target` means the name does not exist in this graph (a typo,
   a macro-generated symbol, or a different revision); `callers` of zero for an
   existing target is a real absence of callers. Never read an empty result as
   an absence until the target has resolved.
2. **Count before paging.** `caller_coverage` splits a target's callers into
   production and test counts. Keep different targets in different calls: a
   shared page can fill with one target and hide another.
3. **Page callers by kind.** `callers_page` with `is_test: false` gives the
   production callers of a change, `is_test: true` the tests that would notice
   it, 25 at a time with each caller's file and line. Each row carries the
   evidence behind its `CALLS` edge: `resolution`, `candidates` (more than one
   means the call site fanned out), `import_backed` (false is unconfirmed, not
   refuted) and `call_lines` (where in the caller the call sits).
4. **Walk the other direction when needed.** `direct_callees` lists what a
   function calls, with the same evidence columns. An unresolved source call
   has no `CALLS` edge at all, so inspect the source before concluding that a
   call is absent.
5. **Witness reachability, do not explore it.** `bounded_call_path` returns up
   to ten paths of at most four hops between two resolved functions, shortest
   first. Deeper answers stop being about the change under review.
6. **Type and trait impact.** `type_consumers` lists functions taking a struct
   as an explicit parameter; `trait_implementations` lists the structs with an
   `IMPLEMENTS` edge to a trait. Macro-generated or heuristic structure may be
   missing from the graph, so a missing edge is a reason to read source.
7. **Read the selected source.** Once the graph has named the lines, use
   `read_code_source` for a qualified name (or `read_source` for a path) with
   `start_line` / `end_line` bounds. Graph edges identify candidates; source
   and executed tests establish behaviour.
8. **Drop to `cypher_query` only for a question no recipe shapes** — imports,
   module structure, docs links, revision deltas (`CALL rev_diff({from: ...,
   to: ...})` on a multi-revision graph). Inspect the needed node or edge shape
   with `graph_overview` first rather than guessing labels or properties.

## Recipes served for this graph

- `code_review/target_coverage` — existence plus caller count, for a list of names.
- `code_review/caller_coverage` — production versus test caller counts for one target.
- `code_review/callers_page` — direct callers of one kind, 25 at a time, with edge evidence.
- `code_review/direct_callees` — what one function calls, 25 at a time, with edge evidence.
- `code_review/bounded_call_path` — up to ten paths of at most four hops between two functions.
- `code_review/type_consumers` — functions taking a struct as an explicit parameter.
- `code_review/trait_implementations` — structs implementing a trait.

For a diff-scoped review, compose the walk yourself: start from the functions
whose `file_path` the diff touched, resolve them with `target_coverage`, then
run steps 2 to 4 on each.

## Reading results

A recipe result is complete up to its literal page size (25 rows, or 10
paths). A full page does not prove the query covered another target or an
edge the builder did not emit. When a response says the completed result was
retained, follow its advertised expansion action; expansion cannot recover
rows a `LIMIT` excluded — narrow the target instead. Treat a warning about an
unknown property or a reversed relationship as evidence that the query shape
is wrong, not as an empty result.

## What counts as a finding

A finding names a concrete failure: the input, state or sequence, and the
wrong outcome it produces — a wrong result, a crash, data loss, a broken
contract with a caller or a persisted format, a security hole, a measured
regression, a check that cannot fail, or a claim the code contradicts. "No
findings" is a valid review. Design preferences, naming and "consider using X"
are not findings; a graph edge showing coupling is a fact, not a defect.
Verify every finding against exact source lines at the reviewed revision
before reporting it, and keep unresolved questions explicit.

## Honesty rules

- Never invent labels, properties, edge types or recipe names; the list above
  and `graph_overview` are the only sources.
- A missing edge is absence of evidence, not proof of absence.
- Never build, import or execute code from a repository merely to review it.
