//! Embedded source for the canonical codingest code-review Agent Skill.

pub const SKILL_DIR: &str = "codingest-code-review";

pub const FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../skills/codingest-code-review/SKILL.md"),
    ),
    (
        "references/queries.md",
        include_str!("../skills/codingest-code-review/references/queries.md"),
    ),
    (
        "references/public-repositories.md",
        include_str!("../skills/codingest-code-review/references/public-repositories.md"),
    ),
    (
        "references/mcp-upgrade.md",
        include_str!("../skills/codingest-code-review/references/mcp-upgrade.md"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn embedded_artifact_matches_canonical_skill() {
        for (relative, embedded) in FILES {
            let canonical = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../skills")
                .join(SKILL_DIR)
                .join(relative);
            assert_eq!(std::fs::read_to_string(canonical).unwrap(), *embedded);
        }
    }

    #[test]
    fn skill_frontmatter_names_codingest() {
        let mut lines = FILES[0].1.lines();
        assert_eq!(lines.next(), Some("---"));
        assert_eq!(lines.next(), Some("name: codingest-code-review"));
    }
}
