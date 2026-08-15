# Language support

Seventeen languages, one graph. Extraction (functions, types, calls) works
the same way everywhere; **file-level dependency edges do not**, because
import mechanisms differ in kind. This page states the contract per language,
so an empty result can be read correctly: some zeros are bugs, some are the
design working.

## The rule behind the table

A `File -[:IMPORTS]-> File` edge exists only where the import string names
**exactly one file** — a path, or a module coordinate that maps one-to-one
onto a file. Namespace-shaped references (a Java package, a C# namespace, a
Julia `using`, an R `library()`) name *collections*; resolving them to a
single file by string matching manufactures wrong edges (measured before this
rule: 438/438 false on a real Java repo), so those mechanisms **abstain** —
no edge, rather than a guessed one. Real support for them requires
build-system indices (Cargo/go.mod/PSR-4-class work), which is planned, not
faked.

| Language | File→File via | Abstains (by design) | Notes |
|---|---|---|---|
| Python | absolute + relative imports (`from .util import x`, aliased, `if TYPE_CHECKING:`/`try:`/function-body) | — | multi-name from-imports expand per name; a package under `src/` is a known gap |
| Rust | `use crate::…` / `super::` / `self::` / bare local paths, `use … as`, `use a::{b, c}` | external crate names | resolved in the importing file's own crate coordinates; workspace members never cross-resolve |
| TypeScript / JavaScript | relative specifiers, tsconfig `paths`, workspace packages | bare package names | unchanged by the 2026-08 resolver work; regression-tested |
| C / C++ | quoted `#include`, dir-first then root, `../` normalized | angle `<includes>` — structurally edge-free even when the name collides with a project file | includes inside `extern "C" {` blocks under `#ifdef` are a known parse-recovery gap (miss-only) |
| Dart | `package:` URIs (directory structure preserved) + relative URIs | `dart:`, foreign packages | package-name matching uses the checkout directory name, not pubspec `name:` (miss-only when they differ) |
| Julia | `include("path.jl")` chains | `using` / `import` — namespace references, no edge even on a name collision | multiple dispatch keeps one node per method; a call to a dispatched name fans out to all candidates |
| R | `source("path.R")` (both `.R`/`.r`) | `library()` / `require()` | **an R package showing zero File→File edges is correct**: packages load via `DESCRIPTION` collation, not `source()` — the edges you'll see on real packages are typically its C code's `#include`s |
| Go | — | all imports (go.mod module prefixes) | extraction full (incl. interface methods); file edges await a go.mod index |
| Java | — | all imports (packages are many files) | `File→Module` edges land on the exact package, never ancestors |
| C# | — | all `using` directives | see `PARITY.md`/release notes — C# extraction has known depth issues chartered for rework |
| Swift | — | module imports | extension methods merge onto the extended type's node |
| PHP | — | namespace `use`; `require`/`include` not yet extracted | symbol-level extraction is complete |
| HTML / CSS | `<script src>` / `<link href>` (file-relative) | CDN URLs; root-absolute (`/x.css`) paths are a known gap for subdirectory-served sites | inline `<script>` bodies parse as JS under the host file |
| AGC assembly | `$FILE.agc` directives, exact filename match only | — | a typo'd directive resolves to nothing, never to a parent module; see `docs/agc-assembly.md` |

`import_backed` on CALLS edges is `true` when the call crosses a resolved
import — see the caveats in [cli.md](cli.md#interpreting-calls-edges) for how
to read `false`.

## What the parsers extract (beyond imports)

All languages: functions/methods with signatures and locations, classes/
structs with fields, same-file call edges, and the conservative cross-file
call tiers (`resolution` property names which tier matched). Language
specifics worth knowing: Julia — long-form, short-form (`f(x) = …`) and stub
definitions; `struct`/`mutable struct` with `<:` supertypes; macros and
anonymous functions are out of scope. R — `f <- function` and `f = function`
(plus `\(x)` lambdas); S4 via string-literal `setClass`/`setGeneric`/
`setMethod`; R6 and `assign()`-defined functions are out of scope. Every
exclusion is stated in the parser's module doc rather than half-implemented.
