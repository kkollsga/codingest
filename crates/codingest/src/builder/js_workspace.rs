//! TS/JS workspace resolution table — `tsconfig.json` `paths` aliases and
//! workspace package names.
//!
//! Relative specifiers (`./x`) are resolvable from the importing file's path
//! alone; the other two shapes a TS monorepo uses are not:
//!
//!   * `@/mcp/index` — a `compilerOptions.paths` alias, whose meaning depends
//!     on which `tsconfig.json` governs the importing file. Real repos put the
//!     aliases in **per-package** tsconfigs, not the root one (opencode's root
//!     tsconfig has no `paths` at all — it only `extends` a base), so a
//!     root-only reader resolves nothing. Discovery is therefore
//!     nearest-ancestor.
//!   * `@opencode-ai/core/foo` — a workspace package specifier, resolvable
//!     only through the `name` field of some `package.json` in the tree.
//!
//! This module builds one deterministic table per build and answers both.
//! Everything it returns is a *candidate string*; whether a candidate becomes
//! an edge is decided by `other_edges`, which only accepts candidates naming a
//! module the project actually defines. Nothing here can invent a target.
//!
//! **Documented limitations** (deliberate, not oversights):
//!   * `extends` chains are not resolved. A tsconfig's own `paths` block is
//!     read literally; aliases inherited from a base config are not seen.
//!   * `exports` / `imports` maps in `package.json` are not interpreted. The
//!     `<pkgdir>/src/<rest>` probe covers the near-universal
//!     `"./*": "./src/*.ts"` convention without parsing the map.
//!   * A `tsconfig.json` that cannot be parsed is skipped silently — and
//!     deterministically, since the skip depends only on file content.

use crate::manifest::walk_filter;
use std::collections::BTreeMap;
use std::path::Path;
use walkdir::WalkDir;

/// The alias table of one `tsconfig.json`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TsPathsConfig {
    /// Directory that pattern targets resolve against: the tsconfig's own
    /// directory joined with `compilerOptions.baseUrl`. Project-relative,
    /// `""` at the project root.
    pub base: String,
    /// Alias pattern → substitution targets, in the order the config lists
    /// them (target order is meaningful: TS tries them in sequence).
    pub paths: BTreeMap<String, Vec<String>>,
}

/// Per-build workspace table. Both maps are `BTreeMap`s because their
/// iteration order can reach edge construction, and row order becomes
/// persisted graph topology.
#[derive(Debug, Default)]
pub struct JsWorkspace {
    /// tsconfig directory (project-relative, `""` = root) → its alias table.
    /// Only directories whose tsconfig actually declares a non-empty `paths`.
    configs: BTreeMap<String, TsPathsConfig>,
    /// `package.json` `name` → that package's directory (project-relative).
    packages: BTreeMap<String, String>,
}

impl JsWorkspace {
    /// Test-only constructor, so sibling modules can assert against a table
    /// without materialising a directory tree. Production tables come only
    /// from [`JsWorkspace::discover`].
    #[cfg(test)]
    pub(crate) fn from_raw(configs: &[(&str, TsPathsConfig)], packages: &[(&str, &str)]) -> Self {
        Self {
            configs: configs
                .iter()
                .map(|(dir, config)| ((*dir).to_string(), config.clone()))
                .collect(),
            packages: packages
                .iter()
                .map(|(name, dir)| ((*name).to_string(), (*dir).to_string()))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.configs.is_empty() && self.packages.is_empty()
    }

    /// Walk `root` once, collecting every `tsconfig.json` alias table and
    /// every `package.json` name. Skips the same directories the source walk
    /// skips (`node_modules`, hidden dirs, `target`, …).
    pub fn discover(root: &Path) -> Self {
        let mut out = Self::default();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            // `walk_filter` is applied to the walk ROOT too, and it rejects any
            // directory whose name starts with `.` — so pointing it at, say,
            // `~/.config/thing` prunes the entire walk before it starts (a
            // known defect of the shared filter, tracked separately). Exempting
            // depth 0 keeps this new walk from reproducing it.
            .filter_entry(|entry| entry.depth() == 0 || walk_filter(entry))
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            let is_tsconfig = name == "tsconfig.json";
            let is_package = name == "package.json";
            if !is_tsconfig && !is_package {
                continue;
            }
            let Some(dir) = relative_dir(root, entry.path()) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            if is_tsconfig {
                if let Some(config) = parse_tsconfig(&text, &dir) {
                    out.configs.insert(dir, config);
                }
            } else if let Some(pkg) = parse_package_name(&text) {
                // Last writer wins would be walk-order dependent; keep the
                // shallowest directory (fewest segments, then lexicographic)
                // so the table is a pure function of the tree.
                match out.packages.get(&pkg) {
                    Some(existing) if shallower(existing, &dir) => {}
                    _ => {
                        out.packages.insert(pkg, dir);
                    }
                }
            }
        }
        out
    }

    /// Alias substitutions for `specifier`, governed by the nearest ancestor
    /// of `importer_dir` that declares `paths`. Empty when no ancestor config
    /// matches. Returned paths are project-relative and un-normalized (they
    /// may still carry a file extension).
    pub fn alias_targets(&self, importer_dir: &str, specifier: &str) -> Vec<String> {
        let Some(config) = self.nearest_config(importer_dir) else {
            return Vec::new();
        };
        let Some((pattern, targets)) = best_pattern(&config.paths, specifier) else {
            return Vec::new();
        };
        let stem = match pattern.split_once('*') {
            Some((prefix, suffix)) => &specifier[prefix.len()..specifier.len() - suffix.len()],
            None => "",
        };
        targets
            .iter()
            .map(|target| join_rel(&config.base, &target.replace('*', stem)))
            .collect()
    }

    /// Candidate paths for a workspace-package specifier
    /// (`@scope/pkg` or `@scope/pkg/sub/path`), longest package-name prefix
    /// winning. Empty when no package name prefixes the specifier.
    pub fn package_targets(&self, specifier: &str) -> Vec<String> {
        let mut best: Option<(&String, &String)> = None;
        for (name, dir) in &self.packages {
            // Boundary check without allocating a `{name}/` probe string: this
            // runs once per package per import specifier, which on a real
            // monorepo is hundreds of thousands of calls per build.
            let matches = match specifier.strip_prefix(name.as_str()) {
                Some("") => true,
                Some(rest) => rest.starts_with('/'),
                None => false,
            };
            if matches && best.is_none_or(|(current, _)| name.len() > current.len()) {
                best = Some((name, dir));
            }
        }
        let Some((name, dir)) = best else {
            return Vec::new();
        };
        let rest = specifier[name.len()..].trim_start_matches('/');
        if rest.is_empty() {
            // Bare package name: the package's own entry point. `<pkgdir>` and
            // `<pkgdir>/src` cover a root index; `<pkgdir>/src/index` is left
            // to the caller's `/index`-stripping pass.
            vec![
                dir.clone(),
                join_rel(dir, "src"),
                join_rel(dir, "src/index"),
            ]
        } else {
            // `<pkgdir>/src/<rest>` covers the `"./*": "./src/*.ts"` exports
            // convention without interpreting the exports map.
            vec![join_rel(dir, rest), join_rel(dir, &format!("src/{rest}"))]
        }
    }

    fn nearest_config(&self, importer_dir: &str) -> Option<&TsPathsConfig> {
        let mut dir = importer_dir;
        loop {
            if let Some(config) = self.configs.get(dir) {
                return Some(config);
            }
            match dir.rsplit_once('/') {
                Some((parent, _)) => dir = parent,
                None if dir.is_empty() => return None,
                None => dir = "",
            }
        }
    }
}

/// True when `a` is closer to the project root than `b` (fewer path segments,
/// then lexicographically) — the tie-break that keeps duplicate package names
/// from making the table depend on directory-walk order.
fn shallower(a: &str, b: &str) -> bool {
    let depth = |s: &str| {
        if s.is_empty() {
            0
        } else {
            s.matches('/').count() + 1
        }
    };
    (depth(a), a) < (depth(b), b)
}

/// Directory of `file`, relative to `root`, `/`-separated; `""` at the root.
fn relative_dir(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let parent = rel.parent()?;
    Some(parent.to_string_lossy().replace('\\', "/"))
}

/// Join a project-relative base with a possibly-`./`-prefixed relative path,
/// collapsing `.` and `..`. Returns a project-relative path.
fn join_rel(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in base.split('/').chain(rel.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }
    parts.join("/")
}

/// The `paths` entry that best matches `specifier`: an exact (wildcard-free)
/// pattern wins outright; among wildcard patterns the longest literal prefix
/// wins, ties broken lexicographically by pattern (a total order, so the
/// choice never depends on iteration order).
fn best_pattern<'a>(
    paths: &'a BTreeMap<String, Vec<String>>,
    specifier: &str,
) -> Option<(&'a str, &'a Vec<String>)> {
    let mut best: Option<(&str, &Vec<String>)> = None;
    for (pattern, targets) in paths {
        let matches = match pattern.split_once('*') {
            None => pattern == specifier,
            Some((prefix, suffix)) => {
                specifier.len() >= prefix.len() + suffix.len()
                    && specifier.starts_with(prefix)
                    && specifier.ends_with(suffix)
            }
        };
        if !matches {
            continue;
        }
        if pattern.contains('*') {
            let prefix_len = pattern.split_once('*').map_or(0, |(p, _)| p.len());
            match best {
                // An exact pattern already won; nothing wildcard beats it.
                Some((current, _)) if !current.contains('*') => {}
                Some((current, _)) => {
                    let current_len = current.split_once('*').map_or(0, |(p, _)| p.len());
                    if (prefix_len, pattern.as_str()) > (current_len, current) {
                        best = Some((pattern, targets));
                    }
                }
                None => best = Some((pattern, targets)),
            }
        } else {
            return Some((pattern, targets));
        }
    }
    best
}

/// `compilerOptions.baseUrl` + `compilerOptions.paths` of one tsconfig, or
/// `None` when it declares no usable aliases.
fn parse_tsconfig(text: &str, dir: &str) -> Option<TsPathsConfig> {
    let value: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).ok()?;
    let options = value.get("compilerOptions")?;
    let base_url = options
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let paths_obj = options.get("paths")?.as_object()?;
    let mut paths = BTreeMap::new();
    for (pattern, targets) in paths_obj {
        let list: Vec<String> = targets
            .as_array()?
            .iter()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect();
        if !list.is_empty() {
            paths.insert(pattern.clone(), list);
        }
    }
    if paths.is_empty() {
        return None;
    }
    Some(TsPathsConfig {
        base: join_rel(dir, base_url),
        paths,
    })
}

fn parse_package_name(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).ok()?;
    let name = value.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Strip JSONC comments and trailing commas so `serde_json` can read a
/// `tsconfig.json`. String-literal-aware: a `//` or `/*` inside a string, and
/// an escaped quote, must not be treated as syntax.
fn strip_jsonc(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    // Iterating `char`s (not bytes) is what keeps multi-byte UTF-8 intact.
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        let next = chars.get(i + 1).copied();
        match c {
            '"' => {
                in_string = true;
                out.push('"');
                i += 1;
            }
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            }
            ',' => {
                // Drop the comma only if the next non-whitespace char closes a
                // container — the trailing-comma case.
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if matches!(chars.get(j), Some('}') | Some(']')) {
                    i += 1;
                } else {
                    out.push(',');
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_stripper_removes_comments_and_trailing_commas() {
        let src = r#"{
  // line comment
  "compilerOptions": {
    /* block
       comment */
    "baseUrl": ".",
    "paths": { "@/*": ["./src/*"], },
  },
}"#;
        let value: serde_json::Value = serde_json::from_str(&strip_jsonc(src)).expect("parses");
        assert_eq!(value["compilerOptions"]["baseUrl"], ".");
        assert_eq!(value["compilerOptions"]["paths"]["@/*"][0], "./src/*");
    }

    #[test]
    fn jsonc_stripper_is_string_literal_aware() {
        // `//`, `/*` and a comma-before-brace inside string literals are DATA,
        // not syntax — a naive regex stripper corrupts every one of these.
        let src = r#"{
  "name": "https://example.com/pkg",
  "a": "/* not a comment */",
  "b": "trailing, }",
  "c": "escaped \" quote // still in string",
  "d": "unicode ✓ ok"
}"#;
        let value: serde_json::Value = serde_json::from_str(&strip_jsonc(src)).expect("parses");
        assert_eq!(value["name"], "https://example.com/pkg");
        assert_eq!(value["a"], "/* not a comment */");
        assert_eq!(value["b"], "trailing, }");
        assert_eq!(value["c"], "escaped \" quote // still in string");
        assert_eq!(value["d"], "unicode ✓ ok");
    }

    fn config(base: &str, entries: &[(&str, &[&str])]) -> TsPathsConfig {
        TsPathsConfig {
            base: base.into(),
            paths: entries
                .iter()
                .map(|(p, ts)| ((*p).to_string(), ts.iter().map(|t| t.to_string()).collect()))
                .collect(),
        }
    }

    fn workspace(configs: &[(&str, TsPathsConfig)], packages: &[(&str, &str)]) -> JsWorkspace {
        JsWorkspace::from_raw(configs, packages)
    }

    #[test]
    fn alias_uses_the_nearest_ancestor_tsconfig() {
        let ws = workspace(
            &[
                ("", config(".", &[("@/*", &["./shared/*"])])),
                (
                    "packages/app",
                    config("packages/app", &[("@/*", &["./src/*"])]),
                ),
            ],
            &[],
        );
        assert_eq!(
            ws.alias_targets("packages/app/src/deep", "@/util"),
            vec!["packages/app/src/util"]
        );
        // A file outside packages/app falls back to the root config.
        assert_eq!(ws.alias_targets("other/dir", "@/util"), vec!["shared/util"]);
        // Nothing matches a specifier no pattern covers.
        assert!(ws.alias_targets("packages/app/src", "zod").is_empty());
    }

    #[test]
    fn alias_patterns_prefer_exact_then_longest_literal_prefix() {
        let ws = workspace(
            &[(
                "",
                config(
                    ".",
                    &[
                        ("@/*", &["./generic/*"]),
                        ("@/deep/*", &["./specific/*"]),
                        ("@/deep/exact", &["./pinned"]),
                    ],
                ),
            )],
            &[],
        );
        // Exact pattern beats both wildcards.
        assert_eq!(ws.alias_targets("", "@/deep/exact"), vec!["pinned"]);
        // Longest literal prefix wins between wildcards.
        assert_eq!(ws.alias_targets("", "@/deep/thing"), vec!["specific/thing"]);
        assert_eq!(ws.alias_targets("", "@/other"), vec!["generic/other"]);
    }

    #[test]
    fn alias_keeps_every_target_in_config_order() {
        let ws = workspace(
            &[("", config(".", &[("@/*", &["./first/*", "./second/*"])]))],
            &[],
        );
        assert_eq!(
            ws.alias_targets("", "@/x"),
            vec!["first/x", "second/x"],
            "TS tries path targets in order; so must we"
        );
    }

    #[test]
    fn alias_substitutes_a_suffixed_wildcard() {
        let ws = workspace(
            &[("", config(".", &[("#db/*.js", &["./src/db/*.ts"])]))],
            &[],
        );
        // Extension stripping is the caller's job, so the target keeps `.ts`.
        assert_eq!(
            ws.alias_targets("", "#db/schema.js"),
            vec!["src/db/schema.ts"]
        );
    }

    #[test]
    fn workspace_packages_match_the_longest_name_prefix() {
        let ws = workspace(
            &[],
            &[
                ("@scope/core", "packages/core"),
                ("@scope/core-extra", "packages/core-extra"),
            ],
        );
        assert_eq!(
            ws.package_targets("@scope/core"),
            vec![
                "packages/core",
                "packages/core/src",
                "packages/core/src/index"
            ]
        );
        assert_eq!(
            ws.package_targets("@scope/core/sub/path"),
            vec!["packages/core/sub/path", "packages/core/src/sub/path"]
        );
        // The longer name wins, and is not shadowed by the shorter prefix.
        assert_eq!(
            ws.package_targets("@scope/core-extra/x"),
            vec!["packages/core-extra/x", "packages/core-extra/src/x"]
        );
        assert!(ws.package_targets("@other/thing").is_empty());
        // `@scope/corely` must NOT match `@scope/core` — the boundary is a `/`.
        assert!(ws.package_targets("@scope/corely").is_empty());
    }

    #[test]
    fn discovery_reads_nested_configs_and_skips_node_modules() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("packages/app/src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/evil")).unwrap();
        // NOTE: `tempfile`'s default prefix is `.tmp…`, so this root is a
        // hidden directory — the test doubles as the guard that discovery does
        // not prune its own walk root.
        std::fs::write(root.join("package.json"), r#"{"name": "root-pkg"}"#).unwrap();
        std::fs::write(
            root.join("tsconfig.json"),
            r#"{ "extends": "@tsconfig/bun" }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("packages/app/package.json"),
            r#"{"name": "@scope/app"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("packages/app/tsconfig.json"),
            "{\n // aliases\n \"compilerOptions\": { \"paths\": { \"@/*\": [\"./src/*\"] } }\n}",
        )
        .unwrap();
        std::fs::write(
            root.join("node_modules/evil/package.json"),
            r#"{"name": "@scope/app"}"#,
        )
        .unwrap();

        let ws = JsWorkspace::discover(root);
        assert_eq!(
            ws.packages.get("@scope/app").map(String::as_str),
            Some("packages/app"),
            "node_modules must not contribute packages"
        );
        assert_eq!(ws.packages.get("root-pkg").map(String::as_str), Some(""));
        // The root tsconfig only `extends` — no literal `paths`, so no entry
        // (extends chains are an explicit non-goal).
        assert!(!ws.configs.contains_key(""));
        assert_eq!(
            ws.alias_targets("packages/app/src", "@/util"),
            vec!["packages/app/src/util"]
        );
    }

    #[test]
    fn unparseable_config_is_skipped_rather_than_fatal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("tsconfig.json"), "{ this is not json").unwrap();
        std::fs::write(tmp.path().join("package.json"), "}{").unwrap();
        let ws = JsWorkspace::discover(tmp.path());
        assert!(ws.is_empty());
    }
}
