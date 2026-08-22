"""DEFINES edge determinism on minified same-line repeats.

History: the 2026-07-15 bug report — total edge counts flapped across
processes on repos where minified CSS/HTML repeats a selector/element
name on one line, because ids embedded file + line only, so same-line
repeats COLLIDED and duplicate (file, entity) DEFINES rows raced
`add_connections`' initial-load fast path.

Since the id-column fix, CSS/HTML ids embed the start column
({file}:{line}:{col}:{slug}), so same-line repeats get DISTINCT ids and
the duplicate-id shape is unreachable from these emitters by design.
This module now pins that fix from the wheel side: every minified repeat
becomes its own node (nothing is silently dropped behind duplicate-id
warnings), zero duplicate ids exist, DEFINES stays consolidated, and
repeated builds agree exactly.


(Revived from tests/python-legacy/ and retargeted `kglite.code_tree` ->
`codingest`. See tests/python/README.md.)
"""

import pytest

from codingest import build

# Single-line (minified) sources: repeats share the line number, so their
# qualified ids collide — the duplicate-id shape that triggered the bug.
MINIFIED_CSS = ".card{color:red}.card{padding:1em}#hero{margin:0}#hero{border:none}\n"
MINIFIED_HTML = (
    '<html><body><div class="card">one</div><div class="card">two</div>'
    '<span id="x">a</span><span id="x">b</span></body></html>\n'
)
PY_MODULE = """def alpha():
    return 1


def beta():
    return alpha()
"""


@pytest.fixture()
def dup_repo(tmp_path):
    (tmp_path / "app.min.css").write_text(MINIFIED_CSS)
    (tmp_path / "index.html").write_text(MINIFIED_HTML)
    (tmp_path / "app.py").write_text(PY_MODULE)
    return tmp_path


def _edge_counts(graph):
    rows = graph.cypher("MATCH ()-[r]->() RETURN type(r) AS t, count(*) AS n ORDER BY t").to_dicts()
    return {row["t"]: row["n"] for row in rows}


def test_minified_repeats_get_distinct_ids(dup_repo):
    # The pre-fix shape: same-line repeats collided on {file}:{line}:{slug}.
    # Post-fix ids carry the start column, so the SAME fixture must now
    # produce zero duplicate-id groups — this is the wheel-side pin of the
    # id-column fix, inverted from the old duplicate-id guard.
    graph = build(str(dup_repo))
    dup_groups = graph.cypher(
        "MATCH (n) WITH labels(n)[0] AS t, n.id AS id, count(*) AS c "
        "WHERE c > 1 RETURN t, count(*) AS groups ORDER BY t"
    ).to_dicts()
    assert dup_groups == [], (
        "minified same-line repeats must get column-distinct ids; a collision "
        "here means the id shape regressed to {file}:{line}:{slug}"
    )


def test_no_parallel_duplicate_defines_edges(dup_repo):
    graph = build(str(dup_repo))
    dupes = graph.cypher(
        "MATCH (a)-[r:DEFINES]->(b) WITH a, b, count(r) AS c WHERE c > 1 RETURN count(*) AS pairs"
    ).to_dicts()
    assert dupes == [{"pairs": 0}], (
        "duplicate (file, entity) DEFINES rows must consolidate onto one edge "
        "regardless of which type-pair hits the initial-load fast path first"
    )


def test_every_minified_repeat_becomes_its_own_node(dup_repo):
    # Pre-fix, colliding repeats were silently dropped behind duplicate-id
    # warnings (2 of 4 selectors survived). Post-fix all four selectors and
    # both spans are distinct nodes.
    graph = build(str(dup_repo))
    counts = {
        row["t"]: row["n"]
        for row in graph.cypher(
            "MATCH (n) WHERE n:Selector OR n:Element RETURN labels(n)[0] AS t, count(*) AS n"
        ).to_dicts()
    }
    assert counts == {"Selector": 4, "Element": 2}


def test_repeated_builds_agree_exactly(dup_repo):
    baseline = _edge_counts(build(str(dup_repo)))
    for _ in range(3):
        assert _edge_counts(build(str(dup_repo))) == baseline
