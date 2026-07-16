# Python API

`pip install codingest` restores the Python builder that kglite 0.14 removed
(`kglite.code_tree`). The tree-sitter grammars are bundled in the native
extension — nothing else to install — and every function returns a real
`kglite.KnowledgeGraph`.

```bash
pip install codingest        # also install kglite>=0.14 for the engine
```

```python
import codingest

g = codingest.build(".")
g.cypher("MATCH (f:Function) RETURN f.name LIMIT 10")
```

## The `.kgl`-bytes handoff

The `codingest` and `kglite` wheels are two separate compiled extensions and
cannot share live Rust objects. So `build()`:

1. constructs the graph with codingest's native builder,
2. serializes it to a `.kgl`, and
3. calls the **installed** `kglite` wheel's `load()` and returns *that* object.

The result is a genuine `kglite.KnowledgeGraph`, so every downstream kglite API
(`.cypher()`, `.describe()`, `.source()`, persistence, …) works unchanged. The
save+load round-trip is the wheel's only per-build overhead (the deserialize
half is cheap; the serialize half dominates). This is why `pip install
codingest` depends on `kglite` at runtime.

## `build`

```python
codingest.build(
    src_dir,
    *,
    save_to=None,          # also write the graph to this .kgl path
    verbose=False,
    include_tests=True,    # include test files/dirs
    max_loc_per_file=None, # skip files longer than N lines
    include_docs=False,    # ingest markdown as :Doc nodes
    rev=None,              # build a single git revision instead of the working tree
    revs=None,             # merge a list of revisions into one multi-rev graph
    repo_root=None,        # override the auto-resolved git root for rev/revs
) -> kglite.KnowledgeGraph
```

Parse a codebase at `src_dir`. Pass `include_docs=True` to also ingest markdown
as `:Doc` nodes linked to the code they mention
(`(:Doc)-[:MENTIONS]->(:Function|:Class|…)`).

**Git revisions.** `rev="v1.0"` builds the codebase as it existed at that
revision — the revision's tracked files are materialized into a tempdir via
`git archive`; `HEAD` and the working tree are never touched. `revs=["v1", "v2"]`
(oldest → newest) merges N revisions into **one** graph: one node per entity
across revs, each carrying native list props `revs: [str]` and `rev_fp: [int]`.
Because one graph holds every rev, an **unscoped** `MATCH (n:Function) RETURN
count(n)` over-counts — scope to one rev with membership:

```python
g = codingest.build(".", revs=["v1.0", "v2.0"])
g.cypher("MATCH (n:Function) WHERE 'v2.0' IN n.revs RETURN n.name")
```

## `repo_tree`

```python
codingest.repo_tree(
    repo,                  # "owner/name" or a full clone URL
    *,
    save_to=None,
    clone_to=None,         # default: a tempdir, removed after
    branch=None,
    token=None,            # auth token for private repos
    verbose=False,
    include_tests=True,
    max_loc_per_file=None,
    include_docs=False,
) -> kglite.KnowledgeGraph
```

Shallow-clones `repo` (shelling out to `git`) and builds it, returning a
`kglite.KnowledgeGraph`.

## `read_manifest`

```python
codingest.read_manifest(path) -> dict | None
```

Read a project manifest (`pyproject.toml` / `Cargo.toml` / …) and return a dict
of metadata: `name`, `version`, `description`, `languages`, `authors`,
`license`, `repository_url`, `manifest_path`, `build_system`, `source_roots`,
`test_roots`. Returns `None` when no manifest is found.

## `language_for_path`

```python
codingest.language_for_path(path) -> str | None
```

Map a file path to its parser language (`"src/app.py"` → `"python"`), or `None`
if no parser handles the file.

## Reference stubs

The full signatures + docstrings live in the shipped type stubs
(`codingest/__init__.pyi`) — your editor and `help(codingest.build)` surface
them directly.
