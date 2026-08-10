"""Shared fixtures for the codingest-py wheel test suite.

These tests exercise the installed `codingest` wheel against the installed
`kglite` wheel (the build-then-load handoff), so they require both to be
importable. `maturin develop` into a venv that already has `kglite` (see the
Makefile `pytest-py` target).
"""

from __future__ import annotations

import importlib.machinery
import importlib.util
import subprocess
from pathlib import Path

import pytest

# ---------------------------------------------------------------------------
# Artifact-freshness guard
#
# This suite imports whatever `codingest` extension happens to be installed. It
# has no idea whether that .so was built from the working tree or from source
# three branches ago, so a stale extension makes the whole suite a FALSE GREEN:
# it passes, loudly, while testing code nobody has touched in days. That is not
# hypothetical — it was hit live during this program, where a suite reported
# green against an extension predating the change under test.
#
# Two conditions, two DELIBERATELY DIFFERENT outcomes:
#
#   STALE  (extension present, older than the newest crates/**/*.rs)
#       -> HARD ERROR. The tests would run and report a verdict about the wrong
#          binary. A skip here would be worse than useless: the guard would be
#          disarmed by exactly the condition it exists to catch.
#
#   ABSENT (no extension installed at all)
#       -> SKIP. Nothing was built, so there is no false verdict to prevent and
#          nothing to be stale about — a fresh checkout that has never run
#          maturin is "not applicable", not "broken". This mirrors the
#          Makefile's SKIP-vs-FAIL rule for a missing venv.
#
# The absent branch substitutes the module collector (pytest_pycollect_makemodule)
# rather than letting collection proceed, because every test module does
# `import codingest` at module scope: allowing the import would turn the intended
# skip into a collection ERROR. Note that a module-level
# `pytest.skip(allow_module_level=True)` in a conftest is NOT a usable
# alternative — pytest 9 propagates it as an internal crash, not a skip.
# ---------------------------------------------------------------------------

_REPO_ROOT = Path(__file__).resolve().parents[2]
_REBUILD_CMD = ".venv/bin/maturin develop --release"
_INSTALL_HINT = (
    "no codingest extension is installed in this interpreter; build one with:\n"
    f"    {_REBUILD_CMD}"
)


def _installed_extension() -> Path | None:
    """Path of the installed `codingest.codingest` extension, or None.

    Resolved WITHOUT importing the package: `find_spec` on a top-level name
    locates it without executing its ``__init__.py`` (which would itself fail
    when the extension is the missing piece). Every platform suffix is tried, so
    this works for the repo-local `maturin develop` layout (``python-source =
    "."``, so the .so lands in ``<repo>/codingest/``) and for a real wheel
    installed under ``site-packages/codingest/`` alike.
    """
    try:
        spec = importlib.util.find_spec("codingest")
    except (ImportError, ValueError):
        return None
    if spec is None or not spec.submodule_search_locations:
        return None
    for location in spec.submodule_search_locations:
        pkg_dir = Path(location)
        for suffix in importlib.machinery.EXTENSION_SUFFIXES:
            for candidate in sorted(pkg_dir.glob(f"codingest*{suffix}")):
                return candidate
    return None


def _newest_rust_source() -> tuple[Path, float] | None:
    """The most recently modified ``crates/**/*.rs`` file and its mtime."""
    crates = _REPO_ROOT / "crates"
    if not crates.is_dir():
        # Running against an installed wheel outside a source checkout: there is
        # no Rust source to be stale relative to, so there is nothing to check.
        return None
    newest: tuple[Path, float] | None = None
    for path in crates.rglob("*.rs"):
        mtime = path.stat().st_mtime
        if newest is None or mtime > newest[1]:
            newest = (path, mtime)
    return newest


def _freshness_status() -> tuple[str, str]:
    """('ok'|'stale'|'absent', message)."""
    extension = _installed_extension()
    if extension is None:
        return "absent", _INSTALL_HINT

    newest = _newest_rust_source()
    if newest is None:
        return "ok", ""
    source, source_mtime = newest
    ext_mtime = extension.stat().st_mtime
    if ext_mtime < source_mtime:
        lag = source_mtime - ext_mtime
        return "stale", (
            "STALE codingest extension - refusing to run the acceptance suite "
            "against it.\n"
            f"  extension: {extension}\n"
            f"             built {lag:.0f}s BEFORE the newest Rust source\n"
            f"  newest source: {source.relative_to(_REPO_ROOT)}\n"
            "These tests would pass or fail based on code that is no longer in "
            "the working tree. Rebuild first:\n"
            f"    {_REBUILD_CMD}"
        )
    return "ok", ""


_STATUS, _STATUS_MESSAGE = _freshness_status()


def pytest_configure(config: pytest.Config) -> None:
    # UsageError, not a failed test: nothing has run yet and nothing should.
    # Exits non-zero with the message, so `make`/CI cannot read it as a pass.
    if _STATUS == "stale":
        raise pytest.UsageError(_STATUS_MESSAGE)


class _AbsentExtensionItem(pytest.Item):
    """A single reported SKIP standing in for a whole uncollectable module."""

    def runtest(self) -> None:
        pytest.skip(_STATUS_MESSAGE)

    def reportinfo(self) -> tuple[Path, int, str]:
        return self.path, 0, "codingest extension not installed"


class _AbsentExtensionModule(pytest.File):
    def collect(self):
        yield _AbsentExtensionItem.from_parent(
            self, name="codingest-extension-not-installed"
        )


def pytest_pycollect_makemodule(module_path: Path, parent):
    # firstresult hook: returning None leaves normal collection alone.
    if _STATUS == "absent":
        return _AbsentExtensionModule.from_parent(parent, path=module_path)
    return None


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
