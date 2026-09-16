//! The code-review methodology codingest ships for the MCP route: one lazy
//! `code_review` skill and the `code_review/*` Cypher recipe catalogue.
//!
//! Two consumers, one source. `codingest-mcp` registers [`skill_record`] and
//! [`recipe_catalog`] through kglite's producer hooks
//! (`ServerExtensions::with_skills` / `with_recipes`), so every graph the
//! embedded server serves — in every mode, including the manifest-less
//! workspace boot — carries the methodology. `codingest build --embed-skills`
//! writes the same records into a `.kgl` through [`attach`] for the graph
//! handed to a plain `kglite-mcp-server --graph`. The default build writes
//! nothing: the parity goldens never see these records.
//!
//! The assets are embedded from `assets/code_review/` rather than derived from
//! the Claude Code agent skill under `skills/`: that skill drives the
//! `codingest query` CLI with literal heredocs, this body drives MCP tools and
//! names recipes. The two stay in step through tests, not generation — the
//! recipe corpus is pinned row-for-row to the documented queries by
//! `crates/codingest-cli/tests/skill_recipes.rs`.
//!
//! Records are constructed directly rather than through
//! `kglite::api::skills::parse_markdown`: that parser needs kglite's `okf`
//! feature, which this crate's `docs` feature turns on by default but a
//! `--no-default-features` build does not.

use std::collections::BTreeMap;

use kglite::api::recipes::{self, RecipeCatalog, RecipeCatalogError, RecipeRecord};
use kglite::api::skills::{self, Delivery, SkillRecord};
use kglite::api::{DirGraph, KgError};
use serde_json::Value;

/// The skill's name and the recipe group: `run_recipe_query("code_review", …)`.
pub const SKILL_NAME: &str = "code_review";
/// The recipe group every query in [`recipe_records`] belongs to.
pub const RECIPE_GROUP: &str = "code_review";
/// Tools whose descriptions carry the skill's when-to-use paragraph.
pub const REFERENCES_TOOLS: &[&str] = &["cypher_query", "run_recipe_query", "read_code_source"];

/// The when-to-use paragraph. Under lazy delivery (mcp-methods ≥ 0.4.11) this
/// is all an agent sees until it calls `skill("code_review")`, and the raw
/// `cypher_query` route does not carry the first-call "load skill" footer —
/// hence the explicit instruction to load the body before the first query.
pub const SKILL_DESCRIPTION: &str =
    "TRIGGER when reviewing a change to an indexed codebase or answering a \
    structural question about it: who calls what, what a change would break, which tests reach a \
    symbol, bounded call paths, type consumers, trait implementors. Call skill(\"code_review\") \
    before your first cypher_query — it names the run_recipe_query recipes that answer these \
    exactly. SKIP for literal text search (use grep) and for whole-file reads that need no \
    structure (read_code_source directly).";

const SKILL_BODY: &str = include_str!("../assets/code_review/skill.md");
const RECIPES_JSON: &str = include_str!("../assets/code_review/recipes.json");

/// The `code_review` skill as kglite's producer hook and graph store take it.
pub fn skill_record() -> SkillRecord {
    SkillRecord {
        name: SKILL_NAME.to_string(),
        description: SKILL_DESCRIPTION.to_string(),
        body: SKILL_BODY.to_string(),
        references_tools: REFERENCES_TOOLS
            .iter()
            .map(|tool| tool.to_string())
            .collect(),
        delivery: Delivery::Lazy,
    }
}

/// The recipe catalogue in the manifest `extensions.cypher_recipes` shape,
/// exactly as `RecipeCatalog::from_manifest_value` and a `.kgl` record expect.
pub fn recipe_catalog_value() -> Value {
    serde_json::from_str(RECIPES_JSON)
        .expect("assets/code_review/recipes.json is valid JSON (pinned by test)")
}

/// The compiled catalogue `codingest-mcp` registers. Compiling here runs the
/// same gate the server runs at boot, so a recipe that would refuse a user's
/// boot refuses the unit test instead.
pub fn recipe_catalog() -> Result<RecipeCatalog, RecipeCatalogError> {
    RecipeCatalog::from_manifest_value(Some(&recipe_catalog_value()))
}

/// The catalogue as individual records, in `(recipe, name)` order, for
/// writing into a graph with [`attach`].
pub fn recipe_records() -> Vec<RecipeRecord> {
    let value = recipe_catalog_value();
    let groups = value.as_object().expect("recipes.json root is a mapping");
    let mut records = BTreeMap::new();
    for (recipe, group) in groups {
        let recipe_description = group["description"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let queries = group["queries"]
            .as_object()
            .expect("recipe group carries a queries mapping");
        for (name, query) in queries {
            records.insert(
                (recipe.clone(), name.clone()),
                RecipeRecord {
                    recipe: recipe.clone(),
                    name: name.clone(),
                    description: query["description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    parameters: query["parameters"].clone(),
                    cypher: query["cypher"].as_str().unwrap_or_default().to_string(),
                    recipe_description: recipe_description.clone(),
                },
            );
        }
    }
    records.into_values().collect()
}

/// What [`attach`] wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachSummary {
    pub skills: usize,
    pub recipes: usize,
}

/// Write the skill and every recipe into `graph` as graph-carried records
/// (`KgliteSkill` / `KgliteRecipe` system labels), so a saved `.kgl` served by
/// a plain `kglite-mcp-server --graph` carries the methodology with it.
///
/// Records go in from a constant, sorted order so two builds of the same
/// source produce identical bytes. The error is boxed because `KgError` is
/// wide enough to trip clippy's `result_large_err` on every `?` site. Only the opt-in build paths call this;
/// the builder itself never does.
pub fn attach(graph: &mut DirGraph) -> Result<AttachSummary, Box<KgError>> {
    skills::set(graph, &skill_record()).map_err(Box::new)?;
    let records = recipe_records();
    for record in &records {
        recipes::set(graph, record).map_err(Box::new)?;
    }
    Ok(AttachSummary {
        skills: 1,
        recipes: records.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite::api::skills::MAX_BODY_BYTES;
    use kglite::api::storage::{new_dir_graph_in_mode, StorageMode};

    const EXPECTED_RECIPES: &[&str] = &[
        "bounded_call_path",
        "caller_coverage",
        "callers_page",
        "direct_callees",
        "target_coverage",
        "trait_implementations",
        "type_consumers",
    ];

    #[test]
    fn recipes_json_compiles_as_a_producer_catalogue() {
        let catalog = recipe_catalog()
            .unwrap_or_else(|error| panic!("recipes.json would refuse the MCP boot: {error}"));
        let group = catalog
            .get(RECIPE_GROUP)
            .expect("the catalogue carries the code_review group");
        assert_eq!(group.queries().len(), EXPECTED_RECIPES.len());
        assert_eq!(catalog.recipes().len(), 1);
    }

    #[test]
    fn every_recipe_record_validates_and_the_corpus_is_the_expected_set() {
        let records = recipe_records();
        let names: Vec<&str> = records.iter().map(|record| record.name.as_str()).collect();
        assert_eq!(names, EXPECTED_RECIPES);
        for record in &records {
            assert_eq!(record.recipe, RECIPE_GROUP);
            assert!(!record.recipe_description.is_empty(), "{}", record.name);
            recipes::validate(record)
                .unwrap_or_else(|error| panic!("recipe {} is invalid: {error}", record.name));
        }
    }

    #[test]
    fn skill_record_validates_fits_the_body_ceiling_and_carries_no_frontmatter() {
        let record = skill_record();
        skills::validate(&record)
            .unwrap_or_else(|error| panic!("skill record is invalid: {error}"));
        assert!(
            record.body.len() <= MAX_BODY_BYTES,
            "body is {} bytes, ceiling is {MAX_BODY_BYTES}",
            record.body.len()
        );
        assert!(
            !record.body.starts_with("---"),
            "frontmatter belongs in the record fields, not the body"
        );
        assert_eq!(record.delivery, Delivery::Lazy);
        assert_eq!(record.references_tools, REFERENCES_TOOLS);
    }

    #[test]
    fn skill_body_names_every_recipe_and_the_description_loads_it_before_cypher() {
        for record in recipe_records() {
            let reference = format!("`{RECIPE_GROUP}/{}`", record.name);
            assert!(
                SKILL_BODY.contains(&reference),
                "skill body does not name {reference}"
            );
        }
        assert!(SKILL_DESCRIPTION.contains("skill(\"code_review\")"));
        assert!(SKILL_DESCRIPTION.contains("cypher_query"));
    }

    #[test]
    fn attach_writes_the_records_into_a_fresh_graph() {
        let mut graph = new_dir_graph_in_mode(StorageMode::Memory, None).unwrap();
        let summary = attach(&mut graph).unwrap();
        assert_eq!(
            summary,
            AttachSummary {
                skills: 1,
                recipes: EXPECTED_RECIPES.len()
            }
        );
        assert_eq!(skills::list(&graph).len(), 1);
        // `list` omits bodies by design; `get` is the full record.
        assert_eq!(skills::get(&graph, SKILL_NAME).unwrap(), skill_record());
        let stored: Vec<String> = recipes::list(&graph)
            .into_iter()
            .map(|record| record.name)
            .collect();
        assert_eq!(stored, EXPECTED_RECIPES);
        // A second attach upserts in place rather than duplicating.
        attach(&mut graph).unwrap();
        assert_eq!(skills::list(&graph).len(), 1);
        assert_eq!(recipes::list(&graph).len(), EXPECTED_RECIPES.len());
    }
}
