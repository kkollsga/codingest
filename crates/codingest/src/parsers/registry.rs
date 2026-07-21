//! Canonical registry for language-level parser metadata.
//!
//! # Adding a language
//!
//! 1. Add its parser module beside this file, expose the module from
//!    `parsers/mod.rs`, and implement [`LanguageParser`] with matching
//!    `language_name()`, `file_extensions()`, and emitted
//!    [`FileInfo::language`](crate::models::FileInfo::language).
//! 2. Add one entry to `language_registry!` below: canonical id, extensions,
//!    both separators, noise names, and parser factory. Separators may differ
//!    when a language uses distinct file-path and namespace grammars (C++ is
//!    the canonical example).
//! 3. Add focused parser/edge tests and a representative parity corpus. Capture
//!    a new golden only for that additive corpus; never regenerate an existing
//!    golden to silence an unexplained digest change.
//!
//! Three behavior-sensitive seams deliberately remain outside the registry:
//! `.h` files are rerouted from C to C++ when C++ sources are present in
//! `builder/mod.rs`; call resolution infers broad language groups from qualified
//! name separators in `builder/call_edges.rs`; and manifests describe project
//! languages independently in `manifest/mod.rs`.

use super::{
    agc, cpp, csharp, css, dart, go, html, java, php, python, rust_lang, swift, typescript,
    LanguageParser,
};

/// Parser construction and language metadata shared by builder consumers.
pub struct LanguageSpec {
    pub id: &'static str,
    pub extensions: &'static [&'static str],
    pub module_sep: &'static str,
    pub edge_sep: &'static str,
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

fn agc_parser() -> Box<dyn LanguageParser + Send + Sync> {
    Box::new(agc::AgcParser::new())
}

macro_rules! language_registry {
    (
        $(
            $id:literal => {
                extensions: [$($extension:literal),+ $(,)?],
                module_sep: $module_sep:literal,
                edge_sep: $edge_sep:literal,
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
        noise_names: rust_lang::RUST_NOISE_NAMES,
        make_parser: rust_parser,
    },
    "python" => {
        extensions: ["py", "pyi"],
        module_sep: ".",
        edge_sep: ".",
        noise_names: python::PYTHON_NOISE_NAMES,
        make_parser: python_parser,
    },
    "typescript" => {
        extensions: ["ts", "tsx"],
        module_sep: "/",
        edge_sep: "/",
        noise_names: typescript::JSTS_NOISE_NAMES,
        make_parser: typescript_parser,
    },
    "javascript" => {
        extensions: ["js", "jsx", "mjs"],
        module_sep: "/",
        edge_sep: "/",
        noise_names: typescript::JSTS_NOISE_NAMES,
        make_parser: javascript_parser,
    },
    "go" => {
        extensions: ["go"],
        module_sep: "/",
        edge_sep: "/",
        noise_names: go::GO_NOISE_NAMES,
        make_parser: go_parser,
    },
    "java" => {
        extensions: ["java"],
        module_sep: ".",
        edge_sep: ".",
        noise_names: java::JAVA_NOISE_NAMES,
        make_parser: java_parser,
    },
    "csharp" => {
        extensions: ["cs"],
        module_sep: ".",
        edge_sep: ".",
        noise_names: csharp::CSHARP_NOISE_NAMES,
        make_parser: csharp_parser,
    },
    "c" => {
        extensions: ["c", "h"],
        module_sep: "/",
        edge_sep: "/",
        noise_names: cpp::C_NOISE_NAMES,
        make_parser: c_parser,
    },
    "cpp" => {
        extensions: ["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        module_sep: "/",
        edge_sep: "::",
        noise_names: cpp::CPP_NOISE_NAMES,
        make_parser: cpp_parser,
    },
    "swift" => {
        extensions: ["swift"],
        module_sep: ".",
        edge_sep: ".",
        noise_names: swift::SWIFT_NOISE_NAMES,
        make_parser: swift_parser,
    },
    "php" => {
        extensions: ["php"],
        module_sep: "\\",
        edge_sep: "\\",
        noise_names: php::PHP_NOISE_NAMES,
        make_parser: php_parser,
    },
    "html" => {
        extensions: ["html", "htm"],
        module_sep: ".",
        edge_sep: ".",
        noise_names: &[],
        make_parser: html_parser,
    },
    "css" => {
        extensions: ["css"],
        module_sep: ".",
        edge_sep: ".",
        noise_names: &[],
        make_parser: css_parser,
    },
    "dart" => {
        extensions: ["dart"],
        module_sep: ".",
        edge_sep: ".",
        noise_names: &[],
        make_parser: dart_parser,
    },
    "agc" => {
        extensions: ["agc"],
        module_sep: "/",
        edge_sep: "/",
        noise_names: agc::AGC_NOISE_NAMES,
        make_parser: agc_parser,
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
    matches!(language, "c" | "cpp" | "html" | "css")
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
                ("agc", "agc"),
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
            ("agc", "/", "/"),
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
            ("rust", false, false),
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
            agc::AGC_NOISE_NAMES,
        ]
        .into_iter()
        .flatten()
        .copied()
        .collect();

        assert_eq!(registry_union, legacy_union);
    }
}
