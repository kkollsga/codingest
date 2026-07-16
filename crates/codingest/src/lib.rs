//! Code-tree: parse polyglot codebases into kglite knowledge graphs.
//!
//! Originally extracted from kglite's in-tree `code_tree` component (which
//! KGLite removed on 2026-07-16, leaving this crate the sole builder) —
//! it depends on the [`kglite`] engine crate for the graph core
//! ([`kglite::api::DirGraph`]), the columnar data model
//! ([`kglite::datatypes`]), and `.kgl` persistence
//! ([`kglite::api::io`]). Tree-sitter grammars are direct
//! dependencies of this crate.
//!
//! Entry points:
//! - [`builder::run_with_options`] — parse a directory or
//!   manifest-rooted project, returns `Arc<DirGraph>`
//! - [`manifest::read_manifest`] — extract project metadata
//! - [`repo::clone_and_build`] — shallow-clone a GitHub repo and
//!   build, returns `Arc<DirGraph>`

pub mod builder;
/// Cross-language HTTP boundary edges — links client calls to server routes.
pub mod cross_lang;
/// Optional docs pass — ingests a repo's markdown as `:Doc` nodes and links them
/// to code symbols. Reuses the OKF parser, so it's gated on the `docs` feature.
#[cfg(feature = "docs")]
pub mod docs;
pub mod manifest;
pub mod models;
pub mod parsers;
pub mod repo;
/// Build a code graph from a git revision (git-archive → tempdir → build),
/// without disturbing the working tree. Exposed as `code_tree.build(rev=…)`.
pub mod rev;

// ── Curated crate-root re-export surface ───────────────────────────────────
// One-stop access spanning the build side (owned by this crate now that
// kglite's in-tree `code_tree` builder is gone) and the read side
// (`kglite::api::code_entities`). The builder / parser / rev entry points come
// from this crate; the entity-handle helpers + source-location types are
// graph-handle capabilities that stay in kglite's `code_entities` module (they
// operate on any `DirGraph`).
pub use builder::run_with_options as build_code_tree;
pub use parsers::language_for_path;
pub use rev::{archive_and_build, build_code_tree_revs, dedup_revs};

pub use kglite::api::code_entities::{
    code_entity_context, find_code_entities, resolve_code_entity, source_location,
    CodeContextLookup, CodeEntityContext, CodeEntityMatch, SourceLocation, SourceLookup,
    CODE_TYPES,
};
