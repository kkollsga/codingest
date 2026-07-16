"""codingest — parse polyglot codebases into kglite knowledge graphs.

    import codingest

    g = codingest.build(".")          # -> kglite.KnowledgeGraph
    g.cypher("MATCH (f:Function) RETURN f.name LIMIT 10")

`build()` parses the tree with codingest's native tree-sitter builder (grammars
bundled — nothing else to install), serializes the result to a `.kgl`, and hands
off to the separately installed `kglite` wheel: the returned object is a real
`kglite.KnowledgeGraph`, so every downstream kglite API works unchanged. This
restores the builder surface that kglite 0.14 removed (`kglite.code_tree`).

Entry points:
    build            - parse a directory (with optional git `rev`/`revs`).
    repo_tree        - clone a GitHub repo and build.
    read_manifest    - extract project metadata from a manifest file.
    language_for_path- map a path to its parser language, or None.
"""

from __future__ import annotations

from importlib import metadata as _metadata

# The native extension registers `build` / `repo_tree` / `read_manifest` /
# `language_for_path`. Renamed to `codingest.codingest` by maturin's
# `module-name`; this pulls its public functions up to the package root.
from .codingest import (  # noqa: F401
    build,
    language_for_path,
    read_manifest,
    repo_tree,
)

try:
    __version__ = _metadata.version("codingest")
except _metadata.PackageNotFoundError:  # pragma: no cover - source checkout
    from .codingest import __version__ as __version__

__all__ = [
    "build",
    "repo_tree",
    "read_manifest",
    "language_for_path",
    "__version__",
]
