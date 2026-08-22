"""A test that imports the installed package by its ABSOLUTE name.

`pkg` lives under `src/`, not under `tests/` — so the importer's own ancestor
chain (`<root>`, `<root>.tests`) can never reach it. Resolving this needs the
`src/` directory itself offered as an import root, and only because the file
set proves a `src/` directory exists at the project root.
"""

from pkg.util import helper


def test_helper():
    assert helper(2) == 4
