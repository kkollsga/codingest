# codingest

Parse polyglot codebases into queryable
[kglite](https://github.com/kkollsga/kglite) knowledge graphs — tree-sitter
parsers for 14 languages, call / type / inheritance / route edges, an optional
markdown-docs pass, and multi-git-revision merged graphs.

codingest is the standalone home of KGLite's former in-tree `code_tree`
component. The graph engine (storage, the Cypher pipeline, `.kgl` persistence)
and the MCP protocol server are **imported from kglite as cargo libraries** —
this project owns only the code-tree component. It ships four surfaces from one
workspace: a **CLI**, an **MCP server**, a **Python wheel**, and a **Rust
crate**.

## Requires kglite ≥ 0.14

codingest builds against 0.14-only engine APIs (`kglite::api::code_entities`,
`kglite_mcp_server::run_with_code_tree_hooks`) that KGLite added when it removed
its in-tree builder. These are **not** in any 0.13.x release, so:

- The Rust crates cannot resolve, and `pip install codingest`'s builder half
  cannot function, until **kglite 0.14.0** is published.
- Keep `kglite` installed for the graph engine; codingest adds the builder on
  top and returns ordinary `kglite.KnowledgeGraph` objects.

## Install

```bash
pip install codingest          # Python wheel (grammars bundled); install kglite>=0.14 too
cargo install codingest-cli    # the `codingest` builder CLI
cargo install codingest-mcp    # the code-graph MCP server
```

## 60-second quickstart

Build a graph, then query it with kglite:

```python
import codingest

g = codingest.build(".")                 # returns a real kglite.KnowledgeGraph
g.cypher("MATCH (f:Function) RETURN f.name LIMIT 10")
codingest.build(".", save_to="code.kgl") # also persist the .kgl
```

Or from the shell:

```bash
codingest build /path/to/repo            # → /path/to/repo/.kglite/code-review.kgl
codingest build /path/to/repo --revs v1.0 v2.0   # multi-rev merged graph
```

Or as a live agent workbench:

```bash
codingest-mcp --root-dir /path/to/repo   # MCP server over stdio
```

## Contents

```{toctree}
:maxdepth: 1
:caption: Surfaces

cli
mcp
python-api
```

```{toctree}
:maxdepth: 1
:caption: About

parity-and-goldens
mcp-parity
migrating-from-kglite-code-tree
```

## License

MIT © Kristian dF Kollsgård. codingest is an independent project; it depends on
`kglite` at runtime but is not otherwise affiliated with it.
