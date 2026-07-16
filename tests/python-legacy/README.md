# Legacy Python behavioral suite (DORMANT)

## Provenance

Copied **verbatim** (no edits) on **2026-07-16** from the KGLite repository,
branch `refactor/codingest-handover`, immediately before KGLite dropped its
`kglite.code_tree` Python surface. Source paths:

- `KGLite/tests/test_code_tree_*.py` (every one, including the new
  `test_code_tree_defines_determinism.py`)
- `KGLite/tests/test_integration_code_tree.py`
- `KGLite/tests/test_hardening_code_tree_characterization.py`
- `KGLite/tests/test_integration_php_html_css.py`
- `KGLite/tests/benchmarks/test_bench_code_tree_new.py`
  → `benchmarks/test_bench_code_tree_new.py` here

## Status: DORMANT — do not run

These tests `import kglite.code_tree`, the Python module built from KGLite's
`kglite-py` crate. That module **no longer exists** once KGLite drops the
surface, so the suite cannot run today. It is preserved here as the
**behavioral specification** for the code-tree component: the exact,
byte-for-byte set of behaviors the KGLite Python binding guaranteed.

Revive it when a codingest Python binding (**codingest-py**) ships. At that
point, re-point the imports from `kglite.code_tree` to the codingest binding
and run the suite as the acceptance gate for that binding.

## Not collected by any test runner

- **Cargo** ignores this tree naturally: it only discovers `tests/*.rs`, and the
  directory name (`python-legacy`) is not a Rust integration-test target.
- **Pytest** collects nothing here because codingest ships no pytest
  configuration or Python package at all (it is a Rust workspace). Do not add
  a `pytest.ini` / `pyproject.toml` / `conftest.py` that would sweep this
  directory in — it must stay dormant until codingest-py exists.
