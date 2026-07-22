# codingest

Give AI agents a live, queryable map of any codebase. codingest builds
[KGLite](https://github.com/kkollsga/kglite) knowledge graphs with tree-sitter
parsers for 15 languages, call / type / inheritance / route edges, an optional
documentation pass, and multi-git-revision merged graphs.

codingest owns code-graph construction: KGLite's former in-tree `code_tree`
component, its CLI and Python surfaces, the builder-backed MCP executable, and
the code-review Agent Skill. KGLite owns the graph engine and reusable
query/read infrastructure: storage, Cypher, `.kgl` persistence, code-entity
reads, and the underlying MCP server.

## Requires kglite ≥ 0.14.5

codingest builds against 0.14-only engine APIs (`kglite::api::code_entities`,
`WorkspaceGraphHooks`, and `ServerExtensions`) exposed after KGLite removed its
in-tree builder. The 0.14.5 floor brings the generic workspace lifecycle and
generation-safe activation. Cargo and pip install this compatible engine
automatically; codingest adds the builder and returns ordinary
`kglite.KnowledgeGraph` objects.

## Install

```bash
pip install codingest          # Python wheel + compatible kglite engine
cargo install codingest-cli    # the `codingest` builder CLI
cargo install codingest-mcp    # the code-graph MCP server
codingest skill install        # the code-review Agent Skill
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
codingest-mcp --watch /absolute/path/to/repo   # live MCP server over stdio
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
