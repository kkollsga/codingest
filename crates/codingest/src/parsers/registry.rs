//! Canonical registry for language-level parser metadata.

use super::{
    cpp, csharp, css, dart, go, html, java, php, python, rust_lang, swift, typescript,
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
        module_sep: "::",
        edge_sep: "/",
        noise_names: cpp::C_NOISE_NAMES,
        make_parser: c_parser,
    },
    "cpp" => {
        extensions: ["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        module_sep: "::",
        edge_sep: "::",
        noise_names: cpp::CPP_NOISE_NAMES,
        make_parser: cpp_parser,
    },
    "swift" => {
        extensions: ["swift"],
        module_sep: ".",
        edge_sep: "/",
        noise_names: swift::SWIFT_NOISE_NAMES,
        make_parser: swift_parser,
    },
    "php" => {
        extensions: ["php"],
        module_sep: ".",
        edge_sep: "\\",
        noise_names: php::PHP_NOISE_NAMES,
        make_parser: php_parser,
    },
    "html" => {
        extensions: ["html", "htm"],
        module_sep: ".",
        edge_sep: "/",
        noise_names: &[],
        make_parser: html_parser,
    },
    "css" => {
        extensions: ["css"],
        module_sep: ".",
        edge_sep: "/",
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
        // These values preserve the two legacy switches exactly. The six
        // disagreements (c/php/swift/html/css/unknown) are pinned, not endorsed.
        let expected = [
            ("rust", "::", "::"),
            ("python", ".", "."),
            ("typescript", "/", "/"),
            ("javascript", "/", "/"),
            ("go", "/", "/"),
            ("java", ".", "."),
            ("csharp", ".", "."),
            ("c", "::", "/"),
            ("cpp", "::", "::"),
            ("swift", ".", "/"),
            ("php", ".", "\\"),
            ("html", ".", "/"),
            ("css", ".", "/"),
            ("dart", ".", "."),
            ("unknown", ".", "/"),
        ];

        for (language, expected_module, expected_edge) in expected {
            assert_eq!(module_sep(language), expected_module, "{language}");
            assert_eq!(edge_sep(language), expected_edge, "{language}");
        }
    }
}
