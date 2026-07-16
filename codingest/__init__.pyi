"""Type stubs for the codingest wheel.

`build()` / `repo_tree()` return a `kglite.KnowledgeGraph` — the object the
installed `kglite` wheel produces, reached through the `.kgl`-bytes handoff
(build native → serialize → `kglite.load`). Every downstream kglite API
(`.cypher()`, `.describe()`, …) is available on the returned value.
"""

from typing import Any, Optional

from kglite import KnowledgeGraph

__version__: str

def build(
    src_dir: str,
    *,
    save_to: Optional[str] = None,
    verbose: bool = False,
    include_tests: bool = True,
    max_loc_per_file: Optional[int] = None,
    include_docs: bool = False,
    rev: Optional[str] = None,
    revs: Optional[list[str]] = None,
    repo_root: Optional[str] = None,
) -> KnowledgeGraph:
    """Parse a codebase at ``src_dir`` into a :class:`kglite.KnowledgeGraph`.

    The stable, public entry point for code-graph building (tree-sitter grammars
    are bundled in the native extension — nothing else to install). codingest
    builds the graph with its own native builder, serializes it to a ``.kgl``,
    and loads it back through the installed ``kglite`` wheel, so the returned
    object is a real ``kglite.KnowledgeGraph``.

    Pass ``include_docs=True`` to also ingest the repo's markdown as ``:Doc``
    nodes linked to the code they mention
    (``(:Doc)-[:MENTIONS]->(:Function|:Class|…)`` and
    ``(:Doc)-[:DOCUMENTS]->(:Doc|:File)``). Off by default (code-only graph).

    Pass ``rev=<tag|branch|sha>`` to build the codebase as it existed at that git
    revision instead of the working tree. The revision's tracked files are
    materialized into a tempdir via ``git archive`` — ``HEAD`` and the working
    tree are never touched, and uncommitted changes are excluded. The git root
    is auto-resolved from ``src_dir`` (override with ``repo_root``); a bad rev or
    non-git directory raises a clear error. The built graph's ``describe()``
    records the revision it represents.

    Pass ``revs=[<rev>, ...]`` (oldest → newest, mutually exclusive with
    ``rev``) to merge N revisions into ONE multi-rev graph: one node per entity
    across revs, each node carrying native list props ``revs: [str]`` (revisions
    it appears in) + ``rev_fp: [int]`` (a per-rev shape fingerprint), and each
    edge carrying ``revs: [str]``. Unchanged entities are stored once, so the
    graph is ≈ base + deltas. Ordinary properties (``signature``,
    ``value_preview``, …) report the NEWEST rev an entity appears in
    (newest-wins). Because one graph holds every rev, an **unscoped**
    ``MATCH (n:Function) RETURN count(n)`` over-counts across revs — scope a
    query to one rev with membership::

        MATCH (n:Function) WHERE 'v2' IN n.revs RETURN n.name

    and use ``CALL rev_diff({from: 'v1', to: 'v2'})`` for added / removed /
    changed deltas between two revs. ``describe()`` lists the loaded revs and
    teaches this scoping idiom.

    Args:
        src_dir: Path to the directory (or manifest-rooted project) to parse.
        save_to: If given, also write the built graph to this ``.kgl`` path
            (the loaded graph is still returned).
        verbose: Emit build progress to stderr.
        include_tests: Include test files/dirs in the graph (default True).
        max_loc_per_file: Skip files longer than this many lines (None = no cap).
        include_docs: Ingest markdown as ``:Doc`` nodes (see above).
        rev: A single git revspec to build (mutually exclusive with ``revs``).
        revs: A list of git revspecs to merge into a multi-rev graph.
        repo_root: Override the auto-resolved git root for ``rev``/``revs``.

    Returns:
        A :class:`kglite.KnowledgeGraph` of the parsed codebase.
    """
    ...

def repo_tree(
    repo: str,
    *,
    save_to: Optional[str] = None,
    clone_to: Optional[str] = None,
    branch: Optional[str] = None,
    token: Optional[str] = None,
    verbose: bool = False,
    include_tests: bool = True,
    max_loc_per_file: Optional[int] = None,
    include_docs: bool = False,
) -> KnowledgeGraph:
    """Clone a GitHub repository and build its code knowledge graph.

    Shallow-clones ``repo`` (shelling out to ``git``) into a tempdir (or
    ``clone_to``) and builds it, returning a :class:`kglite.KnowledgeGraph`.
    Set ``include_docs=True`` to also ingest the repo's markdown as ``:Doc``
    nodes linked to the code they mention (see :func:`build`).

    Args:
        repo: ``owner/name`` or a full clone URL.
        save_to: If given, also write the built graph to this ``.kgl`` path.
        clone_to: Directory to clone into (default: a tempdir, removed after).
        branch: Branch / tag / ref to check out (default: the repo default).
        token: Auth token for private repos.
        verbose: Emit build progress to stderr.
        include_tests: Include test files/dirs in the graph (default True).
        max_loc_per_file: Skip files longer than this many lines.
        include_docs: Ingest markdown as ``:Doc`` nodes (see :func:`build`).

    Returns:
        A :class:`kglite.KnowledgeGraph` of the cloned repository.
    """
    ...

def read_manifest(path: str) -> Optional[dict[str, Any]]:
    """Read a project manifest and return a dict of project metadata.

    Recognises ``pyproject.toml`` / ``Cargo.toml`` (and more). Returns ``None``
    when no manifest is found at ``path``. The dict carries ``name``,
    ``version``, ``description``, ``languages``, ``authors``, ``license``,
    ``repository_url``, ``manifest_path``, ``build_system``, ``source_roots``,
    and ``test_roots`` (the last two as lists of directory paths).
    """
    ...

def language_for_path(path: str) -> Optional[str]:
    """Map a file path to its codingest parser language, or ``None`` if no
    parser handles the file (e.g. ``"src/app.py"`` → ``"python"``)."""
    ...
