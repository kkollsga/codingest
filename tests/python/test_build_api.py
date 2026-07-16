"""Acceptance suite for the codingest wheel's public Python surface.

Covers the resurrected `kglite.code_tree` API (`build` / `repo_tree` /
`read_manifest` / `language_for_path`), and — critically — PROVES the
build-then-load handoff: `build()` returns the *installed kglite wheel's own*
`KnowledgeGraph`, not a codingest type.
"""

from __future__ import annotations

from pathlib import Path

import kglite

import codingest


def _count(g, cypher: str) -> int:
    return g.cypher(cypher).to_list()[0]["c"]


# ── The handoff proof ──────────────────────────────────────────────────────


def test_build_returns_real_kglite_knowledge_graph(sample_tree: Path) -> None:
    g = codingest.build(str(sample_tree))
    # The returned object must be the kglite wheel's own KnowledgeGraph — the
    # whole point of the .kgl-bytes handoff between the two extensions.
    assert type(g).__module__.startswith("kglite")
    assert isinstance(g, kglite.KnowledgeGraph)


def test_build_populates_graph(sample_tree: Path) -> None:
    g = codingest.build(str(sample_tree))
    assert _count(g, "MATCH (n) RETURN count(n) AS c") > 0
    assert _count(g, "MATCH ()-[r]->() RETURN count(r) AS c") > 0
    # Function nodes must be present (greet, hello, test_greet).
    assert _count(g, "MATCH (f:Function) RETURN count(f) AS c") >= 2


def test_returned_graph_is_queryable(sample_tree: Path) -> None:
    g = codingest.build(str(sample_tree))
    names = {r["n"] for r in g.cypher("MATCH (f:Function) RETURN f.name AS n")}
    assert "greet" in names


# ── save_to persistence ────────────────────────────────────────────────────


def test_save_to_writes_loadable_kgl(sample_tree: Path, tmp_path: Path) -> None:
    out = tmp_path / "graph.kgl"
    g = codingest.build(str(sample_tree), save_to=str(out))
    assert out.exists() and out.stat().st_size > 0
    # The .kgl on disk reloads to the same node count as the returned graph.
    reloaded = kglite.load(str(out))
    assert _count(reloaded, "MATCH (n) RETURN count(n) AS c") == _count(
        g, "MATCH (n) RETURN count(n) AS c"
    )


# ── Toggles change the output ───────────────────────────────────────────────


def test_include_tests_toggle_changes_output(sample_tree: Path) -> None:
    with_tests = codingest.build(str(sample_tree), include_tests=True)
    without_tests = codingest.build(str(sample_tree), include_tests=False)
    fns_with = {r["n"] for r in with_tests.cypher("MATCH (f:Function) RETURN f.name AS n")}
    fns_without = {r["n"] for r in without_tests.cypher("MATCH (f:Function) RETURN f.name AS n")}
    assert "test_greet" in fns_with
    assert "test_greet" not in fns_without
    assert "greet" in fns_without  # non-test code still present


def test_include_docs_toggle_changes_output(sample_tree: Path) -> None:
    without_docs = codingest.build(str(sample_tree), include_docs=False)
    with_docs = codingest.build(str(sample_tree), include_docs=True)
    docs_off = _count(without_docs, "MATCH (d:Doc) RETURN count(d) AS c")
    docs_on = _count(with_docs, "MATCH (d:Doc) RETURN count(d) AS c")
    assert docs_off == 0
    assert docs_on > 0


# ── rev / revs on a throwaway git repo ─────────────────────────────────────


def _fn_names(g) -> set[str]:
    return {r["n"] for r in g.cypher("MATCH (f:Function) RETURN f.name AS n")}


def test_working_tree_build_sees_head(git_repo: Path) -> None:
    names = _fn_names(codingest.build(str(git_repo)))
    assert "fn_new" in names and "fn_old" not in names


def test_rev_by_tag_sees_old_commit(git_repo: Path) -> None:
    names = _fn_names(codingest.build(str(git_repo), rev="v1"))
    assert "fn_old" in names and "fn_new" not in names


def test_revs_merges_multiple_revisions(git_repo: Path) -> None:
    g = codingest.build(str(git_repo), revs=["v1", "HEAD"])
    # Both revisions' functions live in the one multi-rev graph.
    names = _fn_names(g)
    assert {"fn_old", "fn_new"} <= names
    # Rev-scoped membership: fn_old only in v1, fn_new only in HEAD.
    old_revs = g.cypher(
        "MATCH (f:Function {name: 'fn_old'}) RETURN f.revs AS r"
    ).to_list()[0]["r"]
    assert "v1" in old_revs


def test_rev_and_revs_mutually_exclusive(git_repo: Path) -> None:
    import pytest

    with pytest.raises(ValueError):
        codingest.build(str(git_repo), rev="v1", revs=["v1", "HEAD"])


# ── read_manifest + language_for_path ──────────────────────────────────────


def test_read_manifest_reads_pyproject(tmp_path: Path) -> None:
    (tmp_path / "pyproject.toml").write_text(
        '[project]\nname = "demo"\nversion = "1.2.3"\n'
        'description = "d"\nrequires-python = ">=3.10"\n'
    )
    (tmp_path / "demo").mkdir()
    (tmp_path / "demo" / "__init__.py").write_text("")
    info = codingest.read_manifest(str(tmp_path))
    assert info is not None
    assert info["name"] == "demo"
    assert info["version"] == "1.2.3"
    assert isinstance(info["source_roots"], list)


def test_read_manifest_none_when_absent(tmp_path: Path) -> None:
    assert codingest.read_manifest(str(tmp_path)) is None


def test_language_for_path() -> None:
    assert codingest.language_for_path("src/app.py") == "python"
    assert codingest.language_for_path("main.rs") == "rust"
    assert codingest.language_for_path("notes.unknownext") is None


def test_version_present() -> None:
    assert isinstance(codingest.__version__, str)
    assert codingest.__version__
