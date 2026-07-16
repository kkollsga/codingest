"""Shared fixtures for the codingest-py wheel test suite.

These tests exercise the installed `codingest` wheel against the installed
`kglite` wheel (the build-then-load handoff), so they require both to be
importable. `maturin develop` into a venv that already has `kglite` (see the
Makefile `pytest-py` target).
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest


def _git(repo: Path, *args: str) -> str:
    out = subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True,
        capture_output=True,
        text=True,
    )
    return out.stdout.strip()


@pytest.fixture()
def sample_tree(tmp_path: Path) -> Path:
    """A small manifest-rooted Python project: source, a test file, and a doc.

    A ``pyproject.toml`` naming the project ``demo`` makes the manifest reader
    resolve ``demo/`` as a source root and ``tests/`` as a test root — which is
    what lets the ``include_tests`` toggle actually drop the test file (without
    a manifest, the whole tree is scanned and the toggle is a no-op). Shaped to
    exercise both toggles:
      - ``demo/core.py``       — a Function + a Class (always present).
      - ``tests/test_core.py`` — a test file (dropped when include_tests=False).
      - ``README.md``          — markdown (only ingested when include_docs=True).
    """
    (tmp_path / "pyproject.toml").write_text(
        '[project]\nname = "demo"\nversion = "0.1.0"\nrequires-python = ">=3.10"\n'
    )
    (tmp_path / "demo").mkdir()
    (tmp_path / "demo" / "__init__.py").write_text("")
    (tmp_path / "demo" / "core.py").write_text(
        "def greet(name):\n"
        "    return f'hi {name}'\n\n\n"
        "class Greeter:\n"
        "    def hello(self):\n"
        "        return greet('world')\n"
    )
    (tmp_path / "tests").mkdir()
    (tmp_path / "tests" / "test_core.py").write_text(
        "from demo.core import greet\n\n\n"
        "def test_greet():\n"
        "    assert greet('x') == 'hi x'\n"
    )
    (tmp_path / "README.md").write_text(
        "# Sample\n\nThe `greet` function returns a greeting for a name.\n"
    )
    return tmp_path


@pytest.fixture()
def git_repo(tmp_path: Path) -> Path:
    """A git repo with two commits and a tag ``v1`` on the first.

    commit1 (tag v1): mod.py defines ``fn_old``.
    commit2:          mod.py defines ``fn_new`` (fn_old removed).
    """
    root = tmp_path / "repo"
    root.mkdir()
    _git(root, "init", "-q")
    _git(root, "config", "user.email", "test@example.com")
    _git(root, "config", "user.name", "Test")

    mod = root / "mod.py"
    mod.write_text("def fn_old():\n    return 1\n")
    _git(root, "add", "mod.py")
    _git(root, "commit", "-q", "-m", "commit1")
    _git(root, "tag", "v1")

    mod.write_text("def fn_new():\n    return 2\n")
    _git(root, "add", "mod.py")
    _git(root, "commit", "-q", "-m", "commit2")
    return root
