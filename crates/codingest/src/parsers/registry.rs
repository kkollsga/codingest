//! Canonical registry for language-level parser metadata.
//!
//! # Adding a language
//!
//! 1. Add its parser module beside this file, expose the module from
//!    `parsers/mod.rs`, and implement [`LanguageParser`] with matching
//!    `language_name()`, `file_extensions()`, and emitted
//!    [`FileInfo::language`](crate::models::FileInfo::language).
//! 2. Add one entry to `language_registry!` below: canonical id, extensions,
//!    both separators, call-resolution group, noise names, and parser factory.
//!    Separators may differ when a language uses distinct file-path and
//!    namespace grammars (C++ is the canonical example).
//! 3. Add focused parser/edge tests and a representative parity corpus. Capture
//!    a new golden only for that additive corpus; never regenerate an existing
//!    golden to silence an unexplained digest change.
//!
//! Two behavior-sensitive seams deliberately remain outside the registry:
//! `.h` files are rerouted from C to C++ when C++ sources are present in
//! `builder/mod.rs`, and manifests describe project languages independently in
//! `manifest/mod.rs`.

use super::{
    agc, cpp, csharp, css, dart, go, html, java, julia, php, python, r, rust_lang, swift,
    typescript, LanguageParser,
};

/// The language family a call may resolve within — tier 3 of
/// `builder/call_edges.rs`, which narrows a call's surviving candidates to
/// those defined in the caller's own group.
///
/// Membership is DECLARED per language here. It was previously guessed by
/// sniffing a qualified name for separators (`::` → rust/cpp, `/` → go/ts/js,
/// otherwise python/java), which read the wrong answer whenever a qname's
/// punctuation did not match its language: HTML-embedded JavaScript is
/// rescoped to `index.html:script_N.<name>` and so grouped with Python, and a
/// TypeScript file at the repository root has no `/` to sniff at all.
///
/// The three groups are the ones the sniff approximated; they are coarse on
/// purpose, because tier 3 only runs after same-owner, same-file and
/// namespace-import have all failed to narrow, and a group that is too narrow
/// there just falls through to the global fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangGroup {
    /// Systems languages that link directly against one another. C sits here
    /// with C++ despite its `/` module separator: the two share headers (see
    /// the `.h` reroute in `builder/mod.rs`) and call each other's symbols
    /// without ceremony, which is exactly what tier 3 asks about.
    RustCpp,
    /// Path-addressed web and service languages. HTML and CSS belong here
    /// because their embedded content *is* JavaScript — the HTML parser hands
    /// `<script>` bodies to the JS parser and keeps the resulting functions
    /// under the host file. (CSS emits selectors and custom properties but
    /// never a `FunctionInfo`, so it never actually reaches tier 3; it is
    /// grouped with the other web assets for want of a truer answer.)
    GoTsJs,
    /// Dotted-namespace languages. AGC is here on the evidence of its qnames:
    /// its registry separator is `/`, but the parser emits `Program.LABEL`, so
    /// this is also where the old sniff put it.
    PythonJava,
}

/// Parser construction and language metadata shared by builder consumers.
pub struct LanguageSpec {
    pub id: &'static str,
    pub extensions: &'static [&'static str],
    pub module_sep: &'static str,
    pub edge_sep: &'static str,
    /// Call-resolution family — see [`LangGroup`]. Not optional: a new
    /// language cannot be registered without choosing one, so the compiler is
    /// the completeness gate.
    pub group: LangGroup,
    pub noise_names: &'static [&'static str],
    pub make_parser: fn() -> Box<dyn LanguageParser + Send + Sync>,
}

fn rust_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(rust_lang::RustParser::new())
}

fn python_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(python::PythonParser::new())
}

fn typescript_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(typescript::JstsParser::typescript())
}

fn javascript_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(typescript::JstsParser::javascript())
}

fn go_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(go::GoParser::new())
}

fn java_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(java::JavaParser::new())
}

fn csharp_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(csharp::CSharpParser::new())
}

fn c_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(cpp::CppParser::c())
}

fn cpp_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(cpp::CppParser::cpp())
}

fn swift_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(swift::SwiftParser::new())
}

fn php_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(php::PhpParser::new())
}

fn html_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(html::HtmlParser::new())
}

fn css_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(css::CssParser::new())
}

fn dart_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(dart::DartParser::new())
}

fn julia_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(julia::JuliaParser::new())
}

fn agc_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(agc::AgcParser::new())
}

fn r_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(r::RParser::new())
}

macro_rules! language_registry {
    (
        $(
            $id:literal => {
                extensions: [$($extension:literal),+ $(,)?],
                module_sep: $module_sep:literal,
                edge_sep: $edge_sep:literal,
                group: $group:expr,
                noise_names: $noise_names:expr,
                make_parser: $make_parser:path $(,)?
            }
        ),+ $(,)?
    ) => {
        /// All supported source languages and their builder-facing metadata.
        pub static LANGUAGES: &[LanguageSpec] = &[
            $(LanguageSpec {
                id: $id,
                extensions: &[$($extension),+],
                module_sep: $module_sep,
                edge_sep: $edge_sep,
                group: $group,
                noise_names: $noise_names,
                make_parser: $make_parser,
            }),+
        ];

        /// File extension → language identifier, derived from [`LANGUAGES`].
        pub const EXTENSION_MAP: &[(&str, &str)] = &[
            $($(($extension, $id)),+),+
        ];
    };
}

language_registry! {
    "rust" => {
        extensions: ["rs"],
        module_sep: "::",
        edge_sep: "::",
        group: LangGroup::RustCpp,
        noise_names: rust_lang::RUST_NOISE_NAMES,
        make_parser: rust_parser,
    },
    "python" => {
        extensions: ["py", "pyi"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::PythonJava,
        noise_names: python::PYTHON_NOISE_NAMES,
        make_parser: python_parser,
    },
    "typescript" => {
        extensions: ["ts", "tsx"],
        module_sep: "/",
        edge_sep: "/",
        group: LangGroup::GoTsJs,
        noise_names: typescript::JSTS_NOISE_NAMES,
        make_parser: typescript_parser,
    },
    "javascript" => {
        extensions: ["js", "jsx", "mjs"],
        module_sep: "/",
        edge_sep: "/",
        group: LangGroup::GoTsJs,
        noise_names: typescript::JSTS_NOISE_NAMES,
        make_parser: javascript_parser,
    },
    "go" => {
        extensions: ["go"],
        module_sep: "/",
        edge_sep: "/",
        group: LangGroup::GoTsJs,
        noise_names: go::GO_NOISE_NAMES,
        make_parser: go_parser,
    },
    "java" => {
        extensions: ["java"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::PythonJava,
        noise_names: java::JAVA_NOISE_NAMES,
        make_parser: java_parser,
    },
    "csharp" => {
        extensions: ["cs"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::PythonJava,
        noise_names: csharp::CSHARP_NOISE_NAMES,
        make_parser: csharp_parser,
    },
    "c" => {
        extensions: ["c", "h"],
        module_sep: "/",
        edge_sep: "/",
        group: LangGroup::RustCpp,
        noise_names: cpp::C_NOISE_NAMES,
        make_parser: c_parser,
    },
    "cpp" => {
        extensions: ["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        module_sep: "/",
        edge_sep: "::",
        group: LangGroup::RustCpp,
        noise_names: cpp::CPP_NOISE_NAMES,
        make_parser: cpp_parser,
    },
    "swift" => {
        extensions: ["swift"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::PythonJava,
        noise_names: swift::SWIFT_NOISE_NAMES,
        make_parser: swift_parser,
    },
    "php" => {
        extensions: ["php"],
        module_sep: "\\",
        edge_sep: "\\",
        group: LangGroup::PythonJava,
        noise_names: php::PHP_NOISE_NAMES,
        make_parser: php_parser,
    },
    "html" => {
        extensions: ["html", "htm"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::GoTsJs,
        noise_names: &[],
        make_parser: html_parser,
    },
    "css" => {
        extensions: ["css"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::GoTsJs,
        noise_names: &[],
        make_parser: css_parser,
    },
    "dart" => {
        extensions: ["dart"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::PythonJava,
        noise_names: &[],
        make_parser: dart_parser,
    },
    // Julia's coordinates are dotted throughout: `file_to_module_path` joins
    // with `.` and every qualified name the parser emits is dotted
    // (`pkg.src.Geometry.area`). Calls pool with the dotted-namespace family.
    "julia" => {
        extensions: ["jl"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::PythonJava,
        noise_names: julia::JULIA_NOISE_NAMES,
        make_parser: julia_parser,
    },
    // AGC's `/` separators are correct despite its dotted qualified names
    // (`Program.LABEL`): the separator fields are only ever applied to
    // `FileInfo.module_path` and `FileInfo.imports`, and the AGC parser
    // emits BOTH slash-shaped (`"Comanche055/MAIN"`, `"Comanche055/SUB"`).
    // Qnames never flow through these fields — they only informed the
    // `LangGroup::PythonJava` placement above. Load-bearing proof:
    // `builder::load::tests::build_modules_splits_agc_paths_on_slash`.
    "agc" => {
        extensions: ["agc"],
        module_sep: "/",
        edge_sep: "/",
        group: LangGroup::PythonJava,
        noise_names: agc::AGC_NOISE_NAMES,
        make_parser: agc_parser,
    },
    // Added 2026-08-15. Both extension casings are registered because
    // `language_for_extension` matches EXACTLY (no case folding anywhere in
    // the walk): `.R` is the dominant convention and `.r` occurs in the
    // wild — registering only one would silently drop the other's files.
    "r" => {
        extensions: ["R", "r"],
        module_sep: ".",
        edge_sep: ".",
        group: LangGroup::PythonJava,
        noise_names: &[],
        make_parser: r_parser,
    },
}

pub fn language_for_extension(extension: &str) -> Option<&'static str> {
    EXTENSION_MAP
        .iter()
        .find(|(candidate, _)| *candidate == extension)
        .map(|(_, language)| *language)
}

pub fn spec(language: &str) -> Option<&'static LanguageSpec> {
    LANGUAGES.iter().find(|candidate| candidate.id == language)
}

pub fn parser(language: &str) -> Option<Box<dyn LanguageParser + Send + Sync>> {
    spec(language).map(|language| (language.make_parser)())
}

pub fn module_sep(language: &str) -> &'static str {
    spec(language).map_or(".", |language| language.module_sep)
}

pub fn edge_sep(language: &str) -> &'static str {
    spec(language).map_or("/", |language| language.edge_sep)
}

pub fn uses_path_imports(language: &str) -> bool {
    // Dart is here for its *relative* URIs only: `import 'a/x.dart'` is a
    // real file path resolved against the importing file's directory,
    // exactly the C/HTML/CSS shape. `dart:`/`package:` URIs never match a
    // file path and fall through to the module-path walk (see
    // `normalize_dart_import`).
    //
    // R is here for `source("path.R")` only: the parser keeps the string
    // verbatim, and it is a real file path (importing-file-relative or
    // project-root-relative — the route tries both). `library(pkg)` /
    // `require(pkg)` names carry no extension and never match a file path,
    // so they fall through to the module walk; they are namespace-shaped and
    // R stays OUT of `build_file_import_edges`'s file-anchored allowlist.
    //
    // Julia is here for `include("relative/path.jl")` — a literal file path
    // resolved against the including file's directory, the C/HTML/CSS shape
    // exactly. Its `using`/`import` module references carry no `.jl` suffix,
    // so they can never match a file path here; they are namespace-shaped
    // and julia is deliberately NOT in the file-anchored allowlist of the
    // raw prefix walk (`other_edges.rs`), so a `using` of an external name
    // colliding with a project file forms no File→File edge.
    matches!(
        language,
        "c" | "cpp" | "html" | "css" | "dart" | "r" | "julia"
    )
}

/// Languages whose import specifiers name a *path* but whose modules are
/// identified by an extension-stripped, `index`-collapsed module path rather
/// than a file path.
///
/// Distinct from [`uses_path_imports`]: C/HTML/CSS specifiers resolve against
/// the file set verbatim (`"../include/header.h"` **is** a file), whereas
/// `import "./util"` names no file that exists — `util.ts`, `util.tsx` and
/// `util/index.ts` all satisfy it, and all three collapse to the same module
/// path. So TS/JS resolve against the *module* set instead, which is also what
/// makes the resolution incapable of inventing a target: a candidate becomes
/// an edge only if it names a module the project actually defines.
pub fn uses_module_path_imports(language: &str) -> bool {
    matches!(language, "typescript" | "javascript")
}

pub fn has_implicit_module_hierarchy(language: &str) -> bool {
    matches!(language, "c" | "cpp" | "swift" | "php")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn extension_map_is_pinned() {
        assert_eq!(
            EXTENSION_MAP,
            &[
                ("rs", "rust"),
                ("py", "python"),
                ("pyi", "python"),
                ("ts", "typescript"),
                ("tsx", "typescript"),
                ("js", "javascript"),
                ("jsx", "javascript"),
                ("mjs", "javascript"),
                ("go", "go"),
                ("java", "java"),
                ("cs", "csharp"),
                ("c", "c"),
                ("h", "c"),
                ("cpp", "cpp"),
                ("cc", "cpp"),
                ("cxx", "cpp"),
                ("hpp", "cpp"),
                ("hh", "cpp"),
                ("hxx", "cpp"),
                ("swift", "swift"),
                ("php", "php"),
                ("html", "html"),
                ("htm", "html"),
                ("css", "css"),
                ("dart", "dart"),
                ("jl", "julia"),
                ("agc", "agc"),
                ("R", "r"),
                ("r", "r"),
            ]
        );
    }

    #[test]
    fn registry_ids_are_unique() {
        let mut ids = HashSet::new();
        for language in LANGUAGES {
            assert!(ids.insert(language.id), "duplicate id: {}", language.id);
        }
    }

    #[test]
    fn parser_language_names_match_registry_ids() {
        for language in LANGUAGES {
            let parser = (language.make_parser)();
            assert_eq!(parser.language_name(), language.id);
        }
    }

    #[test]
    fn separator_matrix_is_pinned() {
        // Separators match the representations emitted by each parser. C++ is
        // intentionally split: file modules are paths, declared namespaces use ::.
        let expected = [
            ("rust", "::", "::"),
            ("python", ".", "."),
            ("typescript", "/", "/"),
            ("javascript", "/", "/"),
            ("go", "/", "/"),
            ("java", ".", "."),
            ("csharp", ".", "."),
            ("c", "/", "/"),
            ("cpp", "/", "::"),
            ("swift", ".", "."),
            ("php", "\\", "\\"),
            ("html", ".", "."),
            ("css", ".", "."),
            ("dart", ".", "."),
            ("julia", ".", "."),
            ("agc", "/", "/"),
            ("r", ".", "."),
            ("unknown", ".", "/"),
        ];

        for (language, expected_module, expected_edge) in expected {
            assert_eq!(module_sep(language), expected_module, "{language}");
            assert_eq!(edge_sep(language), expected_edge, "{language}");
        }
    }

    #[test]
    fn structural_edge_modes_are_pinned() {
        let expected = [
            ("c", true, true),
            ("cpp", true, true),
            ("swift", false, true),
            ("php", false, true),
            ("html", true, false),
            ("css", true, false),
            ("dart", true, false),
            ("julia", true, false),
            ("rust", false, false),
            // R: path imports for `source("path.R")`, no implicit hierarchy.
            ("r", true, false),
        ];
        for (language, path_imports, hierarchy) in expected {
            assert_eq!(uses_path_imports(language), path_imports, "{language}");
            assert_eq!(
                has_implicit_module_hierarchy(language),
                hierarchy,
                "{language}"
            );
        }
    }

    #[test]
    fn registry_noise_union_matches_declared_language_unions() {
        let registry_union: HashSet<&str> = LANGUAGES
            .iter()
            .flat_map(|language| language.noise_names.iter().copied())
            .collect();

        let legacy_union: HashSet<&str> = [
            python::PYTHON_NOISE_NAMES,
            rust_lang::RUST_NOISE_NAMES,
            typescript::JSTS_NOISE_NAMES,
            go::GO_NOISE_NAMES,
            java::JAVA_NOISE_NAMES,
            csharp::CSHARP_NOISE_NAMES,
            cpp::CPP_NOISE_NAMES,
            swift::SWIFT_NOISE_NAMES,
            php::PHP_NOISE_NAMES,
            julia::JULIA_NOISE_NAMES,
            agc::AGC_NOISE_NAMES,
        ]
        .into_iter()
        .flatten()
        .copied()
        .collect();

        assert_eq!(registry_union, legacy_union);
    }
}
