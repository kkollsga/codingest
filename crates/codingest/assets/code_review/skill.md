# Reviewing a codingest code graph

codingest built every graph this server serves, so the shapes are fixed:
`Function`, `Struct`, `Class`, `Trait`, `Interface`, `Protocol`, `Enum`,
`Module`, `File`, `Doc` nodes keyed by `qualified_name`; `CALLS`,
`USES_TYPE`, `IMPLEMENTS`, `EXTENDS`, `HAS_METHOD`, `IMPORTS`, `DEFINES`,
`HAS_FILE` edges. Functions carry `file_path`, `line_number`, `is_test`.
The graph is structural evidence beside the diff and the source; use it
only as far as the question requires.

## Sequence

1. **Resolve targets first** with `run_recipe_query("code_review",
   "target_coverage", {"requested": [...]})`. A null `target` means the name is
   not in this graph (typo, macro-generated, other revision); `callers` of zero
   for an existing target is a real absence. Never read an empty result as an
   absence until the target has resolved.
2. **Count before paging.** `caller_coverage` splits one target's callers into
   production and test counts. One target per call: a shared page can fill
   with one target and hide another.
3. **Page callers by kind.** `callers_page` with `is_test: false` gives the
   production callers, `is_test: true` the tests that would notice, 25 at a
   time with file and line. Each row carries the `CALLS` edge's evidence:
   `resolution`, `candidates` (>1 = the call site fanned out), `import_backed`
   (false is unconfirmed, not refuted) and `call_lines`.
4. **Walk the other way** with `direct_callees`, same evidence columns. An
   unresolved source call has no `CALLS` edge at all, so read the source before
   concluding a call is absent.
5. **Witness reachability, do not explore it.** `bounded_call_path` returns up
   to ten paths of at most four hops between two resolved functions.
6. **Type and trait impact.** `type_consumers` lists functions taking a struct
   as an explicit parameter; `trait_implementations` lists structs with an
   `IMPLEMENTS` edge to a trait. Macro-generated or heuristic structure may be
   missing, so a missing edge is a reason to read source.
7. **Read the selected source** with `read_code_source` (qualified name) or
   `read_source` (path), bounded by `start_line` / `end_line`.
8. **Use `cypher_query` only for a question no recipe shapes** — imports,
   module structure, docs links, `CALL rev_diff({from, to})` on a multi-rev
   graph — and check the shape with `graph_overview` first.

## Recipes served for this graph

- `code_review/target_coverage` — existence plus caller count, for a list of names.
- `code_review/caller_coverage` — production versus test caller counts for one target.
- `code_review/callers_page` — direct callers of one kind, 25 at a time, with edge evidence.
- `code_review/direct_callees` — what one function calls, 25 at a time, with edge evidence.
- `code_review/bounded_call_path` — up to ten paths of at most four hops between two functions.
- `code_review/type_consumers` — functions taking a struct as an explicit parameter.
- `code_review/trait_implementations` — structs implementing a trait.

For a diff-scoped review, start from the functions whose `file_path` the
diff touched, resolve them, then run steps 2 to 4 on each.

## Reading results

A page is complete only up to its literal size (25 rows, 10 paths); a full
page proves nothing about other targets or edges the builder never emitted.
Follow a retained result's advertised expansion; expansion cannot recover
rows a `LIMIT` excluded — narrow the target instead. A warning about an
unknown property or reversed relationship means the query shape is wrong, not
that the result is empty.

## Findings and honesty

A finding names a concrete failure — the input or state and the wrong outcome
— verified against exact source lines at the reviewed revision. "No findings"
is a valid review; design preferences are not findings, and an edge showing
coupling is a fact, not a defect. Never invent labels, properties, edge types
or recipe names; a missing edge is absence of evidence, not proof of absence;
never build or execute a repository's code merely to review it.
