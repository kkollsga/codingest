"""Independent oracle for Python import extraction/resolution (findings 3-5,
mcp-servers report 2026-08-14).

WHY THIS SUITE EXISTS
---------------------
The builder's own tests (parser units, resolver units, the py_import golden)
all share one lineage: the tree-sitter parse and the Rust resolver. A bug in
that shared machinery can pin its own wrong answer. This oracle derives the
intra-corpus import edge set for `tests/corpus/py_import/` a second,
independent way - Python's own `ast` module plus a from-scratch resolution of
relative imports against the real file layout, sharing nothing with
tree-sitter or `other_edges.rs` - then builds the corpus with the release
binary, reads the `.kgl` through the installed `kglite` wheel, and compares
`(File)-[:IMPORTS]->(File)` edges against it.

SCOPE RULE (set in the plan, do not exceed): the hard assertion is recall of
EXPECTED_EDGES - the exact set the B2 fix is designed to resolve, one comment
per edge naming its finding. It does NOT set a general recall bar; any
ast-derived edge beyond that set is printed as a gap for the summary, never
asserted on.

Unlike its siblings here, this suite needs the release binary and the kglite
wheel. CI's `release-gates` job installs neither, so the whole module skips
when `kglite` is not importable; locally (a venv with kglite) it runs in
full, building the binary first if absent.
"""

from __future__ import annotations

import ast
import subprocess
import sys
from pathlib import Path

import pytest

kglite = pytest.importorskip("kglite")

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS = REPO_ROOT / "tests" / "corpus" / "py_import"
BINARY = REPO_ROOT / "target" / "release" / "codingest"

# The exact File->File edge set the B2 fix is designed to resolve. Paths are
# corpus-relative, exactly as the graph's File.path property stores them.
EXPECTED_EDGES = {
    # F3: `from . import util` in the package __init__ (relative import).
    ("pkg/__init__.py", "pkg/util.py"),
    # F3: `from . import util` / `from .util import helper as h` in a.py.
    ("pkg/a.py", "pkg/util.py"),
    # F3: `from .sub.deeper import deep_thing` - multi-segment relative.
    ("pkg/a.py", "pkg/sub/deeper.py"),
    # F4: `import pkg.util as u` (aliased) + `from pkg import util`
    #     (module named by a from-import, not just its package).
    ("pkg/b.py", "pkg/util.py"),
    # F4: `from pkg.sub import deeper` - edge lands on the module, not the
    #     package __init__.
    ("pkg/b.py", "pkg/sub/deeper.py"),
    # F5: `from .a import a_fn` under `if TYPE_CHECKING:` - a nested block
    #     the old root-level-only walk never saw (also exercises F3).
    ("pkg/c.py", "pkg/a.py"),
    # F3: `from ..util import helper` - a two-dot pop from pkg/sub/deeper.py.
    ("pkg/sub/deeper.py", "pkg/util.py"),
}


# -- ast-based derivation (shares nothing with the tree-sitter pipeline) -----


def corpus_modules() -> dict[tuple[str, ...], str]:
    """Map module-name tuples to corpus-relative file paths.

    `pkg/util.py` -> ('pkg', 'util'); `pkg/__init__.py` -> ('pkg',).
    """
    modules: dict[tuple[str, ...], str] = {}
    for path in sorted(CORPUS.rglob("*.py")):
        rel = path.relative_to(CORPUS)
        parts = list(rel.parts[:-1])
        stem = rel.stem
        if stem != "__init__":
            parts.append(stem)
        modules[tuple(parts)] = rel.as_posix()
    return modules


def ast_import_edges() -> set[tuple[str, str]]:
    """Every intra-corpus (source_file, target_file) import edge, per `ast`.

    Walks the WHOLE tree (ast.walk), so imports under `if`, `try` and inside
    function bodies count - that is finding 5's contract. Resolution follows
    Python's own semantics: an absolute `import a.b` names module `a.b`;
    `from base import name` first tries `base.name` as a module, then falls
    back to `base` (name is a symbol); a relative import anchors at the
    importing file's package, popping one segment per dot past the first.
    Self-imports are not edges. Targets outside the corpus resolve to nothing.
    """
    modules = corpus_modules()
    edges: set[tuple[str, str]] = set()
    for path in sorted(CORPUS.rglob("*.py")):
        rel = path.relative_to(CORPUS)
        source_file = rel.as_posix()
        # The importing file's package, per Python: for pkg/a.py and for
        # pkg/__init__.py alike this is ('pkg',).
        package = tuple(rel.parts[:-1])
        tree = ast.parse(path.read_text(encoding="utf-8"))

        def add(target_parts: tuple[str, ...], fallback: tuple[str, ...] | None) -> None:
            target = modules.get(target_parts)
            if target is None and fallback is not None:
                target = modules.get(fallback)
            if target is not None and target != source_file:
                edges.add((source_file, target))

        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    add(tuple(alias.name.split(".")), None)
            elif isinstance(node, ast.ImportFrom):
                if node.level == 0:
                    base = tuple(node.module.split(".")) if node.module else ()
                else:
                    anchor = package[: len(package) - (node.level - 1)]
                    base = anchor + (tuple(node.module.split(".")) if node.module else ())
                for alias in node.names:
                    if alias.name == "*":
                        add(base, None)
                    else:
                        # `from base import name`: module base.name if it is
                        # one, else symbol -> the base module itself.
                        add(base + (alias.name,), base)
    return edges


# -- graph-based derivation (the machinery under test) -----------------------


@pytest.fixture(scope="module")
def graph_edges(tmp_path_factory: pytest.TempPathFactory) -> set[tuple[str, str]]:
    if not BINARY.exists():
        subprocess.run(
            ["cargo", "build", "--release", "-p", "codingest-cli"],
            cwd=REPO_ROOT,
            check=True,
        )
    kgl = tmp_path_factory.mktemp("oracle") / "py_import.kgl"
    subprocess.run(
        [str(BINARY), "build", str(CORPUS), "--output", str(kgl)],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
    )
    graph = kglite.load(str(kgl))
    rows = graph.cypher(
        "MATCH (a:File)-[:IMPORTS]->(b:File) RETURN a.path AS src, b.path AS dst"
    )
    return {(row["src"], row["dst"]) for row in rows}


def test_oracle_derivation_covers_the_expected_set() -> None:
    """Sanity for the oracle itself: the independent ast derivation must
    reproduce every expected edge, or the oracle could not catch their loss."""
    missing = EXPECTED_EDGES - ast_import_edges()
    assert not missing, f"ast derivation lost expected edges: {sorted(missing)}"


def test_graph_recall_of_the_expected_edge_set(graph_edges: set[tuple[str, str]]) -> None:
    missing = EXPECTED_EDGES - graph_edges
    assert not missing, (
        "graph is missing edges the B2 fix is designed to resolve: "
        f"{sorted(missing)}; graph has {sorted(graph_edges)}"
    )


def test_report_recall_against_the_full_ast_set(graph_edges: set[tuple[str, str]]) -> None:
    """Recall against everything ast finds - REPORTED, not asserted, beyond
    the expected set (scope rule). A gap here goes in the phase summary."""
    expected = ast_import_edges()
    hit = expected & graph_edges
    recall = len(hit) / len(expected) if expected else 1.0
    gaps = sorted(expected - graph_edges)
    extras = sorted(graph_edges - expected)
    print(
        f"\noracle recall: {len(hit)}/{len(expected)} = {recall:.2f}\n"
        f"gaps (ast-only): {gaps}\nextras (graph-only): {extras}",
        file=sys.stderr,
    )
    # Only the in-scope subset is load-bearing.
    assert EXPECTED_EDGES <= expected
