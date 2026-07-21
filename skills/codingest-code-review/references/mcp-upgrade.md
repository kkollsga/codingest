# When to use the MCP server

The CLI skill is the low-friction path: no agent configuration, one process per
command, and a review artifact that can be rebuilt explicitly.

Upgrade to `codingest-mcp` when the work benefits from:

- a graph kept warm across many queries;
- watch mode and automatic refresh after file changes;
- typed tool input/output schemas rather than shell quoting;
- switching among several repository roots;
- cached public-repository lifecycle and GitHub/source tools; or
- long collaborative sessions where process startup becomes noticeable.

The local-code-review MCP workspace covers a changing checkout. The
open-source workspace covers cached public repositories and adds constrained
source and GitHub tooling. Codingest builds the code graph; both workflows use
KGLite's graph engine, query language, and read-tool surface.
