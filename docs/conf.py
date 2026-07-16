# Configuration file for the Sphinx documentation builder.

project = "codingest"
copyright = "2024, Kristian dF Kollsgård"
author = "Kristian dF Kollsgård"

extensions = [
    "myst_parser",
    "sphinx.ext.napoleon",
    "sphinx_copybutton",
]

# -- MyST (Markdown) settings ------------------------------------------------

myst_enable_extensions = [
    "colon_fence",
    "deflist",
    "fieldlist",
]
myst_heading_anchors = 6

# -- General settings ---------------------------------------------------------

exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]
source_suffix = {
    ".rst": "restructuredtext",
    ".md": "markdown",
}

# The Cypher snippets use a Pygments lexer that doesn't recognize every KGLite
# expression; keep that presentation-only warning narrow so `-W` stays useful.
suppress_warnings = ["misc.highlighting_failure"]

# -- HTML output --------------------------------------------------------------

html_theme = "furo"
html_title = "codingest"
html_theme_options = {
    "source_repository": "https://github.com/kkollsga/codingest",
    "source_branch": "main",
    "source_directory": "docs/",
}
