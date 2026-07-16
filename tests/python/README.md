# codingest-py test suite (LIVE)

Acceptance tests for the `codingest` Python wheel (`crates/codingest-py`). They
run against the **installed** `codingest` wheel and the **installed** `kglite`
wheel, exercising the build-then-load `.kgl` handoff end to end.

## Running

The wheel must be built into the active venv first (a venv that already has
`kglite` installed):

```bash
make pytest-py          # builds the wheel via maturin develop, then runs these
```

or by hand:

```bash
VENV=../KGLite/.venv
env -u CONDA_PREFIX VIRTUAL_ENV=$VENV $VENV/bin/maturin develop --release
$VENV/bin/python -m pytest tests/python
```

`pyproject.toml`'s `[tool.pytest.ini_options] testpaths = ["tests/python"]`
scopes collection here, so the dormant `tests/python-legacy/` tree is never
swept in.

## Layout

- `test_build_api.py` — the new acceptance suite: the handoff proof
  (`build()` returns a real `kglite.KnowledgeGraph`), node/edge/Function
  presence, `save_to` round-trip, `include_tests` / `include_docs` toggles,
  `rev` / `revs` on a throwaway git repo, `read_manifest`, `language_for_path`.
- `test_code_tree_defines_determinism.py`, `test_code_tree_python_references_fn.py`,
  `test_code_tree_procedures.py` — three representative files revived from
  `tests/python-legacy/`, mechanically retargeted
  (`from kglite.code_tree import build` → `from codingest import build`; the
  `tree_sitter` Python-package guard dropped, since codingest bundles grammars).

## Reviving more of the legacy suite

`tests/python-legacy/` holds KGLite's full 47-file behavioral spec, dormant
because it imports the removed `kglite.code_tree` module. Any of those files
retargets the same mechanical way as the three above:

1. Copy it into `tests/python/`.
2. Replace `from kglite.code_tree import build` with `from codingest import build`
   (and `import kglite; kglite.code_tree.build(...)` call sites with
   `codingest.build(...)`).
3. Delete the `pytest.importorskip("tree_sitter")` line.

Files that assert kglite-specific surface (e.g. `test_code_tree_public_api.py`
checks `kglite.__all__` / the `kglite.code_tree` submodule) do **not** retarget
cleanly — the equivalent public-surface checks for codingest live in
`test_build_api.py` instead.
