# Code-review query patterns

Run `kglite describe <graph> --connections --cypher` first and adapt these
patterns to the labels and properties it reports.

Find a symbol before asking about its relationships:

```cypher
MATCH (n)
WHERE n.name = '<symbol>' OR n.qualified_name = '<qualified_symbol>'
RETURN labels(n) AS labels, n.qualified_name AS symbol,
       n.file_path AS file, n.line_number AS line
LIMIT 20
```

After confirming the connection name, inspect direct callers:

```cypher
MATCH (caller)-[:CALLS]->(target)
WHERE target.qualified_name = '<qualified_symbol>'
RETURN caller.qualified_name AS caller,
       caller.file_path AS file, caller.line_number AS line
ORDER BY file, line
```

Find tests structurally connected to a changed symbol — anchor the traversal on
the symbol, not on the tests:

```cypher
MATCH (changed {qualified_name: '<qualified_symbol>'})<-[*1..4]-(test)
WHERE test.is_test = true
RETURN DISTINCT test.qualified_name AS test,
       test.file_path AS file, test.line_number AS line
LIMIT 100
```

An anchored spelling starts from a point lookup instead of scanning every node
and post-filtering a sparse property, and it is immune to the traversal seed
caps — which since kglite 0.16.6 are advisory, with the pass re-run exactly when
it hits a cap and comes back short of the `LIMIT`. Before that, the unanchored
shape could silently return partial results.

For a yes/no reachability question, ask for one witness instead of the whole
set:

```cypher
MATCH (changed {qualified_name: '<qualified_symbol>'})
WHERE EXISTS { (changed)<-[*1..4]-({is_test: true}) }
RETURN changed.qualified_name AS reachable_from_a_test
```

Since kglite 0.16.6 `EXISTS { … }` stops at the first witness rather than
expanding the pattern in full — the deep-existence shape had been costing
hundreds of times its fixed-hop equivalent. The inline `{is_test: true}` map
keeps that fast path; an inner `WHERE` inside the braces does not.

For a multi-revision graph, prefer the built-in delta procedure shown by
`describe()`:

```cypher
CALL rev_diff({from: '<base>', to: '<head>'})
YIELD bucket, type, qualified_name, name, file, line
RETURN bucket, type, qualified_name, name, file, line
ORDER BY bucket, type, qualified_name
```

Prefer `$placeholders` over splicing values into the query text: since kglite
0.16.6 the MCP `cypher_query` tools take a `params` object, which binds both the
`WHERE n.x = $p` and the inline `{x: $p}` spellings as data that can never be
read as Cypher syntax. An unbound `$param` now raises `Missing parameter: $p` —
it used to answer `0` or an empty result on aggregate and inline-map shapes, so
a missing binding read as a fact about the graph. CLI one-shot queries still
take literal Cypher: there, replace the angle-bracket placeholders only with
trusted git or source identifiers and escape Cypher string quotes, or use the
JSONL session API's parameter support for untrusted values.

Two more behaviours, in place since kglite 0.16.6, worth knowing while reading
results:

- `=~` matches the **whole** value, not a substring. `n.name =~ 'admin'` no
  longer selects `'superadmin'`; write `CONTAINS`, or `=~ '.*admin.*'`.
- A result may carry a trailing `warnings:` block — an unknown projection
  property with a "did you mean?" hint, or a relationship pattern written in the
  wrong direction. Treat it as a signal that the query shape is wrong, not as
  noise: those are exactly the mistakes that return a confident empty answer.
