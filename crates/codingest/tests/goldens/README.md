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

The additive `dart_part_of` digest was captured on 2026-08-22 with the corpus
itself. `dart_import`, the only other Dart corpus, contains no `part` file, so
Dart's `part of` handling — the one place a file's module path comes from
something other than its own location on disk — had no golden coverage and
could be changed, or broken, with every digest staying green. The corpus pins
the property the feature exists for: `lib/collection.dart` and its two parts
under `lib/src/` are ONE module, with all three files hanging off
`dart_part_of.lib.collection`. The fix that landed with it is what makes that
true — the previous derivation kept only the URI's stem (`{pkg}.{stem}`), so
the parts landed in a phantom `dart_part_of.collection` module that no file
declared and the parent library never joined, and the `../collection.dart`
spelling is what makes the dropped directory segments visible. `fromA` calling
the parent file's `seed` pins that the cross-file CALLS edge still resolves
once the three files share a module. Verified additive the strict way:
`capture_goldens` rewrote every file, and afterwards `git status` reported only
`dart_part_of.sha256` as new — the one pre-existing digest that also shows as
modified, `julia_basic.sha256`, moved in the preceding EXTENDS-typing commit
and not in this capture.

The additive `rust_inline_mod` digest was captured on 2026-08-22 with the
corpus itself. Every Rust `use` in `rust_import` and `rust_xfile` sits at file
level, so the scope a `use` is written in never mattered to any golden — and a
`use` written *inside* an inline `mod` block was resolved one level too
shallow, because `FileInfo::imports` is a flat list with no scope column and
the resolver anchors every path at the FILE's module. `mod tests { use
super::*; }`, the most common shape in Rust source, is exactly that bug: it
recorded a bare `super`, popped the file's own module, and produced a
plausible-but-wrong IMPORTS edge to the parent module's file. On this corpus
the pre-fix build emitted four such edges (three `src/alpha.rs -> crate::src`
Module edges and one `src/alpha.rs -> src/lib.rs` File edge) that the fix
removes. The corpus pins all three arms of the re-anchoring: supers equal to
the inline depth land on the file itself and form no edge (the self-guard eats
them), a surplus super still pops the file's real parent and reaches
`beta.rs`, and two nested inline levels cancel two supers. `beta.rs`'s
file-level `use crate::alpha::helper` is the control that must not move.
Verified additive the strict way: `capture_goldens` rewrote every file, and
afterwards `git status` reported only `rust_inline_mod.sha256` as new — and a
canonical dump of all corpora before and after the fix differed in
`rust_inline_mod` alone.

The additive `py_src_layout` digest was captured on 2026-08-22 with the corpus
itself. All four pre-existing Python corpora put their packages where the
importing file's own ancestor chain can already reach them, so the ONE layout
`pyproject.toml` made standard had no golden at all: with `pkg` under `src/`
and the test under `tests/`, the candidate roots for `from pkg.util import
helper` are `<prefix>` and `<prefix>.tests`, and neither can ever spell
`<prefix>.src.pkg.util` — the import resolved to nothing, and could have gone
on resolving to nothing forever with every digest green. The corpus pins the
evidence-gated `src` root from both ends: the two edges it adds
(`tests/test_util.py` → `src/pkg/util.py` and → the `…src.pkg.util` Module),
and the gate itself — the root is offered only because the FILE SET contains a
`src/` directory, never because the specifier looked like it wanted one.
`py_import`, which has no `src/`, is the control that must not move. Verified
additive the strict way: `capture_goldens` rewrote every file, and afterwards
`git status` reported only the three new corpora's digests as new, with
`docs_mdx.sha256` the sole modification.

The additive `cpp_extern_c` digest was captured on 2026-08-22 with the corpus
itself. Every `#include` in `cpp_include` sits at a file's top level, so the
single most common real-world header prologue — `#ifdef __cplusplus` /
`extern "C" {` / `#endif` — had no coverage at all, and every include inside
it was silently dropped. Measured on DaveGamble/cJSON @ `fb16e5cf3587`: 96
quoted includes expected, 91 found, 0 false positives, and the misses included
`cJSON_Utils.h` → `cJSON.h`, the library's own core edge. The corpus pins both
ways tree-sitter renders that prologue, because they are different defects
wearing one symptom: `utils.h` CLOSES the block, so tree-sitter-c builds a
`linkage_specification` that `parse_c_top_level` had no arm for; `decls_begin.h`
leaves it OPEN (a `*_begin.h` closed by its caller), so the rest of the file
collapses into one top-level `ERROR` recovery subtree. It also pins what must
NOT be extracted, which is the harder half: the `<vector.h>` angle include
inside the same region names a REAL project file and is excluded by its
delimiters alone, and `phantom.h` is named by an include-SHAPED line with no
`#` — reachable only by a text scan of the ERROR region, which is exactly the
false-positive route the fix refuses to take. `cpp_include` is the control and
does not move.

The additive `web_served_root` digest was captured on 2026-08-22 with the
corpus itself. No pre-existing corpus contains a leading-`/` web reference at
all, so the served-root question was invisible: a built site under `dist/` is
served with `dist/` as `/`, and `/_astro/app.css` was resolved against the
PROJECT root only — never reaching `dist/_astro/app.css`, which is in the graph
the whole time. The corpus pins the new candidate and its precedence together:
`/shared/reset.css` resolves at the project root today and must keep resolving
there, so the project root is still tried first and the linking file's own
directory only after it. `dup_minified_assets` and `html_js_lang_group`, the
other web corpora, are the controls and do not move.

The additive `csharp_using_alias`, `php_group_use`, `java_javadoc`,
`go_interface` and `swift_basic` digests were captured on 2026-08-22 with the
corpora themselves. Before them the golden set contained not one `.cs`, `.php`,
`.java`, `.go` or `.swift` file, so five whole parsers — and the one CALLS tier
that only a namespace-shaped import can reach — could have been changed, or
deleted, with every digest staying green. Three of the five pin a defect fixed
earlier in the same program, and because each fix landed before its corpus, the
pre-fix behaviour was measured on these exact trees rather than asserted:

* `csharp_using_alias` — `using_directive` took its first identifier child, so
  `using Log = MyApp.Logging;` recorded the ALIAS as the imported namespace.
  The corpus makes that mistake land somewhere real: `src/Decoy/Log/Logger.cs`
  declares an actual `namespace Log` with an actual `Logger.Emit`. Measured
  pre-fix, `src/App/Service.cs` imported `Log` and `Run` called
  `Log.Logger.Emit` at `namespace_import`/1 candidate — the same shape as the
  right answer, onto the wrong node. Post-fix the import is `MyApp.Logging` and
  the call is `MyApp.Logging.Logger.Emit`. This is the only corpus that reaches
  the `namespace_import` tier at all; the tier needs a `.`/`::` after the
  imported prefix, which PHP's `\` can never supply.
* `php_group_use` — `extract_use_imports` never matched the
  `namespace_use_group` body, so `use App\Domain\{Billing\Invoice,
  Catalog\Product};` recorded one import, the bare ancestor `App\Domain`, and
  dropped both members. Measured pre-fix: two IMPORTS edges; post-fix: three
  (`App\Models`, `App\Domain\Billing`, `App\Domain\Catalog`). The group members
  are sub-namespace-qualified on purpose — a bare `{User, Post}` trims straight
  back to the ancestor and pins nothing.
* `php_group_use` and `swift_basic` together — `parse_block` passed
  `owner_prefix.is_empty()` as `is_method`, so every TOP-LEVEL function was
  stored `is_method=true`. Measured pre-fix, PHP's `build_report` and Swift's
  `trim` were both `true`; both are now `false`, with every class/struct/enum
  method in the two corpora unchanged at `true`.

The other two pin extraction that had no defect behind it, only no coverage.
`java_javadoc` pins `get_doc_comment` from both sides — eight javadoc'd
declarations (four types and one method on each) carry a docstring while
`quiet`, the one method preceded by a plain `//` line comment instead of a
javadoc block, must carry NULL — plus the second reader of the same comment
vocabulary, a `// TODO:` that must land in that file's `annotations`, and the
namespace walk's one-segment `min_end` bound with live bait:
`import com.acme.Formatter` must form no edge even though Module `com` is in
the graph, while `import com.example.util.Text` beside it resolves.
`go_interface` pins the `method_elem` arm (tree-sitter-go 0.25's rename of
`method_spec`), the sole producer of interface-method Functions and their
HAS_METHOD edges, by two independent detectors: the `Reader.Fetch`/
`Reader.Reset` nodes with their two HAS_METHOD edges, and the `s.Fetch(…)`
call in `main` fanning out to both `Reader.Fetch` and `Memory.Fetch` as a
two-candidate `lang_group` resolution — a shape that collapses to one
`unique_name` edge, silently, if the arm dies.

Four absences in these corpora are pinned deliberately, so that a fix has to
move a digest rather than slip in unobserved: Swift's `struct Greeter:
Greeting` produces no IMPLEMENTS edge (the inheritance specifier is unparsed);
Go's `Memory` satisfying `Reader` produces none either (implicit conformance is
unmodeled); Swift call extraction keeps only a call's terminal segment, never
its receiver; and `go_interface`'s `import "demo/store"` — the ordinary Go
shape, `go.mod`'s module path plus the package directory — produces NO
File→Module IMPORTS edge. That last one is a live defect, not a modelling
choice: `go.mod` is never read (the manifest reader takes only `pyproject.toml`
and `Cargo.toml`), and a Go module path is built package-name-FIRST
(`store/store` for `store/*.go` in package `store`), so no prefix of a real
import specifier can match one. Only a single-segment specifier resolves, and
only onto the ancestor Module the package-first path accidentally creates.

Verified additive the strict way: `capture_goldens` rewrote every file, and
afterwards `git status` reported exactly the five new digests as additions,
with no pre-existing golden modified.

**`docs_mdx` was deliberately regenerated on 2026-08-22** for the markdown
link-classification fix. `discover_docs` has always matched doc extensions
case-insensitively — `README.MD` is why the corpus exists — but the link
scanner stripped `.md`/`.mdx` case-SENSITIVELY, so an upper-cased destination
was classified as a source File. The thing it names is a `:Doc` node and never
a `:File` node, so the existence check dropped it and the link produced no edge
at all: silent, and invisible to every digest. `docs/overview.md` now links
`../README.MD`, and the diff is exactly one edge — `DOCUMENTS docs/overview →
README` — plus that doc's own body-derived properties. Extending this corpus
rather than adding one was the right home: its stated purpose already is the
case-insensitive doc-extension arm, and the uppercase `README.MD` it needed as
a target was already there.

**`dup_minified_assets` and `html_js_lang_group` were deliberately regenerated
on 2026-08-22** for the CSS/HTML id-collision fix. A `Selector` id was
`{rel_path}:{line}:{slug}` and an `Element` id `{rel_path}:{tag}:{line}:{slug}`
— neither identifies a node in a MINIFIED file, where every rule and every
element sits on line 1. `dup_minified_assets` is the corpus that reproduces it:
`.card{…}.card{…}` produced two `app.min.css:1:card` rules and
`<span id="x">…<span id="x">` two `index.html:span:1:x` elements, and the build
printed `warning: 1 duplicate id(s) on type 'Element'` /
`warning: 2 duplicate id(s) on type 'Selector'` — ids the engine collapses, so
`MATCH (n {id: …})` returned one node per pair and each collision lost a real
node. Both id builders now carry the 1-based start COLUMN (`{rel_path}:{line}:
{col}:{slug}`, `{rel_path}:{tag}:{line}:{col}:{slug}`), matching the 1-based
line convention the same call sites already use. The digest movement is the
record of the fix: `dup_minified_assets` goes from 5 to 8 DEFINES edges (two
recovered `Selector`s, one recovered `Element`) and the build prints no
duplicate-id warning at all. `html_js_lang_group` moved too — it is the only
other corpus with an HTML element node, and its single `index.html:div:4:panel`
became `index.html:div:4:5:panel` with no count changing anywhere, which is
what shows the change is pure identity and not behaviour. Verified the strict
way: `capture_goldens` rewrote every file, and afterwards `git status` reported
exactly those two digests as modified.

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
cargo test --workspace --test parity -- --ignored capture_goldens
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
