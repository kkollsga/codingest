# Golden parity digests

Each `<corpus>.sha256` here is a single lowercase-hex SHA-256 line: the digest
of a **canonical, exhaustive rendering** of the knowledge graph built from
`tests/corpus/<corpus>/`. The rendering (see
`canonical_graph_string` in `../parity.rs`) covers exactly what the
two-builder equivalence check sweeps — node-type counts, edge-type counts, the
sorted `(node_type, id)` identity set, the full per-node property sweep, and
the full per-edge property sweep — so two graphs share a digest iff they are
graph-equivalent.

## Provenance and authority

**Captured 2026-07-16 from the last in-sync in-tree builder** — i.e. from
`kglite::code_tree::builder::run_with_options`, KGLite's in-tree component,
while the `corpus_parity` test verified it was byte-for-byte identical to the
`codingest` builder (parity suite green, all 6 original corpora). **These
digests remain the anchor.** The additive `agc_basic` digest was reviewed and
captured with the new AGC parser on 2026-07-21, then deliberately refreshed on
2026-07-22 for its architecture-aware control/data relationship model. It
supplements the six historical authority digests without rewriting them.

The additive `ts_monorepo` digest was captured on 2026-08-01 together with the
corpus itself, for the same reason: no pre-existing corpus contained a single
TypeScript `import` statement, so TS/JS import resolution had no golden
coverage at all and could be changed — or broken — with zero golden movement.
Its capture was verified additive the strict way: `capture_goldens` rewrites
all eight files, and afterwards `git status` reported only
`ts_monorepo.sha256` as new, so the seven pre-existing digests came back
byte-identical.

The additive `ts_hof_binding` digest was captured on 2026-08-01 with the
corpus itself, for the same reason again: no pre-existing corpus contained a
`const` bound to a function literal, a `function*` in any spelling, or a
factory-wrapped binding (`Effect.fn(…)(function*…)`), so depth-0 higher-order
bindings and the TS grammar vocabulary had no golden coverage — the parser
could emit a `Constant` where a `Function` belongs, or no node at all, with
every digest staying green. Verified additive the strict way: `capture_goldens`
rewrote all nine files, and afterwards `git status` reported only
`ts_hof_binding.sha256` as new, so the eight pre-existing digests came back
byte-identical. The gate was then mutation-tested — disabling the factory
unwrap, restoring the dead `"function"` match arm, and dropping the
`wrapped_by` column promotion each turned it red, and each restore turned it
back green.

The additive `ts_closure_scope` digest was captured on 2026-08-01 with the
corpus itself. Same reason a third time, one level down: no pre-existing
corpus declared *anything* below the top level of a TS file — not a nested
named binding, not a `namespace`, not a closure-scoped helper — so the whole
nested scope walk could be changed or deleted with every digest staying green.
It pins both directions of the inclusion criterion: the shapes that must
become nodes (a closure-scoped `Effect.fn` binding, a nested named arrow, a
depth-2 chain, a namespace member, a class-method-local) and the shapes that
must not (a named binding under an anonymous callback, `arr.map(f)` at
depth > 0, a plain IIFE), plus the `#{line}` duplicate tie-break and D3's
same-file-only CALLS participation. Verified additive the strict way:
`capture_goldens` rewrote all ten files, and afterwards `git status` reported
only `ts_closure_scope.sha256` as new, so the nine pre-existing digests came
back byte-identical. The gate was then mutation-tested five ways — see the
Phase 3 commit message.

The additive `py_nested_defs` digest was captured on 2026-08-01 with the
corpus itself, for the same reason once more, on the other side of the
language split: the four committed Python corpora contain nothing but
top-level `def`s and plain classes — not one nested definition between them —
so the Python scope walk could be changed or deleted with every digest staying
green. It pins the shapes that must become nodes (a decorator factory two
levels deep, a closure factory, a nested helper, a method-local, a
function-local class's methods) alongside Python's answer to D1 clause 5: `if`
/ `try` / `with` blocks are transparent, and a `lambda` names no scope and
keeps its calls with the enclosing `def`. It also pins the `#{line}` tie-break
on both conditional-definition idioms, and D3 from both sides — a cross-file
caller whose five nested-name calls must resolve to nothing, against same-file
CALLS, REFERENCES_FN and DECORATES edges into nested definitions that must.
Verified additive the strict way: `capture_goldens` rewrote all eleven files,
and afterwards `git status` reported only `py_nested_defs.sha256` as new, so
the ten pre-existing digests came back byte-identical. The gate was then
mutation-tested five ways — see the Phase 4 commit message.

The additive `docs_mdx` digest was captured on 2026-08-01 with the corpus
itself. This one guards the docs pass rather than a parser: not one of the
eleven pre-existing corpora contains an `.mdx`, an upper-cased `.MD`, or a
`.txt` that must stay out, so `discover_docs`'s accepted-extension match could
be widened — `.txt` is the tempting "helpful" widening — or narrowed back with
every digest staying green. It pins the `.mdx` arm end to end (frontmatter
properties, heading outline, backtick MENTIONS, a doc→doc DOCUMENTS edge whose
target is an `.mdx` and whose source is a `.md`, and a doc→File edge), the
extension-stripped `:Doc` id (`README.MD` → `README`), and the inertness of
embedded JSX/ESM. `NOTES.txt` is markdown-shaped in every respect — heading,
backtick symbol, markdown link — and must contribute nothing; if `.txt` were
ever admitted it would add a Doc node, a MENTIONS edge and a DOCUMENTS edge,
and this digest would move. Verified additive the strict way: `capture_goldens`
rewrote all twelve files, and afterwards `git status` reported only
`docs_mdx.sha256` as new, so the eleven pre-existing digests came back
byte-identical. The gate was then mutation-tested — see the Phase 5 commit
message.

The additive `docs_ext_collide` digest was captured on 2026-08-10 with the
corpus itself. It guards the other half of the docs pass's identity rules: no
pre-existing corpus contains two docs in one directory whose names differ only
by markup extension, so the concept-id collision policy could be changed — or
regress to the silent last-write-wins overwrite it replaced — with every digest
staying green. `docs/guide.md` and `docs/guide.mdx` both strip to `docs/guide`;
precedence is `.mdx` > `.md` > `.rst`, the winner keeps the id and the losers
are dropped from doc-node emission entirely. The digest pins the survivor's
identity (the `.mdx` file's `file_path`, frontmatter and MENTIONS), the loser's
total absence (`shadowed_only_symbol` is mentioned ONLY by `docs/guide.md`, so
a MENTIONS edge to it means a dropped doc still reached the graph), a link
written against the DROPPED spelling (`Notes.MD` links to `docs/guide.md`)
still resolving to the surviving `docs/guide` node, an uncolliding `intro.rst`
surviving untouched, and the mixed-case `Notes.MD` stripping to `Notes`.
Verified additive the strict way: `capture_goldens` rewrote all thirteen files,
and afterwards `git status` reported only `docs_ext_collide.sha256` as new, so
the twelve pre-existing digests came back byte-identical. The gate was then
mutation-tested — see the commit message.

The additive `py_routes_dup` digest was captured on 2026-08-10 with the corpus
itself, alongside the ONE deliberate movement of a pre-existing digest described
below. No pre-existing corpus registers the same route path from two different
files, so Route identity — `{framework}::{method}::{path}` at the time — could
collapse every methodless `@app.route('/')` in a repo into a single node, whose
`file_path`/`line_number` described whichever file the sorted walk reached first
and mislocated all the rest, with every digest staying green. The digest pins
both sides of the registration model: `public/views.py` and `admin/views.py`
each register `/` and are TWO Route nodes, each carrying its own truthful source
location, while the two `/dup` registrations inside `admin/views.py` remain ONE
node with parallel HANDLES edges (the id carries the declaring file,
deliberately not the line — a line-bearing id would churn whenever an unrelated
line is inserted above a decorator, and would not disambiguate Django at all,
whose urlpattern entries all share the constant's line).

**`cross_ts_py` was deliberately regenerated in that same commit** — the only
pre-existing digest that moved. Route ids gained the declaring file, so its four
Route ids (`/api/session` and `/api/unused`, each emitted under both the `flask`
and `fastapi` labels) changed shape, together with the HANDLES and CALLS_SERVICE
endpoints naming them. Verified section-by-section with `dump_canonical` before
and after: `node_type_counts` and `edge_type_counts` are byte-identical and the
only lines that differ are id strings gaining the `::server/app.py` suffix.
Cross-language linking matches on the `path` PROPERTY, never by parsing the id,
so the change is inert for `CALLS_SERVICE` except for honest fan-out (a path
with N registrations now links to all N). Verified additive the strict way:
`capture_goldens` rewrote all fourteen files, and afterwards `git status`
reported exactly `py_routes_dup.sha256` as new and `cross_ts_py.sha256` as
modified, so the twelve untouched pre-existing digests came back byte-identical.

The additive `html_js_lang_group` digest was captured on 2026-08-10 with the
corpus itself. It guards the `lang_group` CALLS tier, which had **no golden
coverage at all**: reaching tier 3 needs one bare name defined in two different
language families that survives the same-owner, namespace-import and same-file
tiers, and no corpus had that shape — `cross_ts_py` shares no bare name across
its halves (`createSession` vs `create_session`) and `dup_minified_assets`'s
`index.html` contains no `<script>`. The whole grouping could therefore be
changed, or deleted outright, with every digest staying green. The corpus pins
the case that motivated declaring groups in `parsers::registry`: `index.html`'s
inline script is rescoped to `index.html:script_N.main` — a qname whose
punctuation is dots, so the old separator sniff filed it with Python — and its
`render()` call must resolve to `web/widgets.render`, NOT to the equally-named
`server/app.py` definition. Verified additive the strict way: `capture_goldens`
rewrote all fifteen files, and afterwards `git status` reported only
`html_js_lang_group.sha256` as new, so the fourteen pre-existing digests came
back byte-identical. The gate was then mutation-tested: restoring the separator
sniff in `infer_lang_group`'s place turned it red (the call re-resolved to the
Python definition) and the registry lookup turned it back green.

The four import-corpus digests (`rust_import`, `py_import`, `cpp_include`,
`dart_import`) were captured additively on 2026-08-15 (post-016 program B0)
with a provenance unique in this file: **they deliberately pin the then-BROKEN
resolver output** — 2/4/8/3 IMPORTS edges against far more declared imports —
so that each fix phase's regeneration diff is itself the record of exactly the
edges the fix gained (B1 Rust, B2 Python, B3 C/C++ incl. a later
angle-collision corpus pin, B4 Dart; regeneration reasons live in those
commit messages per this file's convention). Verified strictly additive at
capture: only the four new `.sha256` files appeared.

The additive `r_basic` digest was captured on 2026-08-15 (post-016 E2) with R
language support: no pre-existing corpus contained a `.R`/`.r` file. It pins
both assignment shapes plus the `\(x)` lambda, conservative S4
(setClass/setGeneric/setMethod), a `source()` chain A→B→C through a
subdirectory (file-anchored path route), a lowercase `.r` file end-to-end, and
a `library(tools)` bait against a local `tools.R` that must produce no edge of
either kind. Verified strictly additive; mutation-tested (assignment-shape and
path-route breaks each turned the golden red).

The additive `julia_basic` digest was captured on 2026-08-15 with the corpus
itself and with Julia language support (post-016 program phase E): no
pre-existing corpus contained a `.jl` file, so the entire Julia parser had no
golden coverage. The corpus pins long-form and short-form (`f(x) = …`)
functions, a multiple-dispatch `area` pair the builder's overload pass must
keep as two `#<sha256>`-decorated nodes, a `module` block with qualified
members and a HAS_SUBMODULE declaration, structs with fields and a `<:`
supertype (EXTENDS), and Julia's split import model from both sides: an
`include("…")` chain (`Main.jl → geometry.jl → shapes/circle.jl`) that must
form File→File IMPORTS edges through the path route, and a `using Downloads`
colliding with a never-included `src/Downloads.jl` bait that must form none —
`using`/`import` are namespace references and julia is deliberately absent
from the raw prefix walk's file-anchored allowlist. Verified additive the
strict way: `capture_goldens` rewrote all the files, and afterwards
`git status` reported only `julia_basic.sha256` as new. The gate was then
mutation-tested: removing `"julia"` from `uses_path_imports` (include edges
vanish) and breaking short-form function extraction each turned the suite red,
and each restore turned it back green.

## The 2026-08-10 bulk regeneration (13 of 14 digests)

**This is the first bulk regeneration since the extraction**, and the only one
so far in which most digests moved at once. Every capture described above was
additive by construction — one new corpus, pre-existing digests byte-identical
— and the single earlier pre-existing movement (`cross_ts_py`) was one file.

**Reason.** Only `pyproject.toml` and `Cargo.toml` were ever recognised as
project manifests, so every other repository — all JS/TS, Go, Java and C++
trees, and any plain directory — built with **no `:Project` node at all**: no
owner for its files, and docs attached to nothing structural. Manifestless
repositories now get an *inferred* project: named after the project root,
languages reconciled from the files actually parsed, `manifest` set to the
sentinel `(inferred)`, owning every source file via `HAS_SOURCE` and every doc
via the new `Project HAS_DOC Doc` edge. Thirteen of the fourteen corpora are
manifestless, so thirteen digests gained a `Project` node plus its ownership
edges. `docs_mdx` and `docs_ext_collide` gained `HAS_DOC` edges as well.

**The frozen exception is the proof.** `rust_xfile` is the only corpus with a
manifest (`Cargo.toml`), and its digest came back **byte-identical** — that is
what demonstrates the change is purely additive for manifest-backed graphs. The
constraint that makes this hold is that the inferred project adds **no new
property column** to the `:Project` dataframe: "inferred" is encoded in the
existing `manifest` property rather than in a new one, precisely because the
per-node property sweep above would otherwise move every manifest-backed digest
too. Verified the strict way: `capture_goldens` rewrote all fourteen files, and
afterwards `git status` reported exactly thirteen `.sha256` files modified,
with `rust_xfile.sha256` absent from the list.

`HAS_DOC` on a *manifest-backed* project is corpus-uncovered — no corpus has
both a manifest and docs — so it is pinned by a unit test instead
(`manifest_backed_project_keeps_the_same_property_columns` in
`builder/mod.rs`), alongside the no-new-column invariant.

Regeneration rationale for a change of this size belongs in the commit message;
see the `feat(builder): anchor manifestless repositories with an inferred
Project` commit, per the convention stated under *Regenerating* below.

KGLite deleted its in-tree builder on 2026-07-16, so `corpus_parity` (the live
in-tree vs codingest check) is gone. `golden_parity` — which builds each corpus
with only the `codingest` builder and compares to these frozen digests — is now
the sole guardian that codingest still produces the historically-correct graph.

`golden_parity` builds each corpus **three times** (`BUILDS_PER_CORPUS`), so it
is also the builder-determinism gate: hash iteration order is randomized per
`HashMap` instance, and `dup_minified_assets` reproduces the DEFINES-edge
ordering bug that once flapped edge totals run-to-run. A disagreement *between
builds* is nondeterminism and is never a reason to regenerate; agreement between
builds that differs from the golden is a behaviour change and may be. The test
reports the two cases distinctly.

## Regenerating (deliberate builder-behavior changes only)

Do **not** regenerate to make a red `golden_parity` go green. A digest change
means the graph a corpus produces changed; regenerate only when that change is
intended (a parser fix, a new edge kind, a property-shape change). The capture
path is:

```bash
cargo test -p codingest --test parity -- --ignored capture_goldens
```

This rewrites every `<corpus>.sha256`. Review the diff, and land the digest
change in the same commit as the builder change that caused it.

**Post-deletion note (done 2026-07-16).** KGLite's in-tree builder has been
deleted, and `capture_goldens` (in `../parity.rs`) has been retargeted from
`kglite::code_tree::builder::run_with_options` to
`codingest::builder::run_with_options` — codingest is now its own oracle.
These frozen digests, captured 2026-07-16 from the in-tree authority's
last-known-good output, remain the anchor a regeneration is reviewed against.

## Why there is no rev-fixture golden

The multi-rev fixture (`rev_self_consistency`) builds a throwaway 2-commit git
repo in a tempdir. Its canonical digest is **not stable across from-scratch
runs**: each run creates fresh commits whose SHAs depend on the commit
timestamp, and those SHAs are stored verbatim in the `revs` node and edge
property. Identical file content therefore yields different `revs` values
run-to-run, so the digest flaps. (`rev_fp`, a content-shape hash, is stable;
`revs` is not.)

Rather than freeze an unstable digest, the multi-rev path is guarded by
`rev_self_consistency`: it builds the **same** repo (identical SHAs) twice with
the `codingest` builder and asserts the two graphs are equivalent, including
the stamped `revs`/`rev_fp` provenance — a determinism check that needs no
second builder.
