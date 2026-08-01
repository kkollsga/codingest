//! CALLS edge resolution — scope-aware name matching with import context.
//!
//! Ported from builder.py::_build_call_edges, then extended with a
//! namespace-import tier so that languages with explicit `using`/`import`
//! directives (C#, Java, TS, Python, Go) disambiguate same-named symbols
//! across namespaces. On dotnet/runtime this lifts CALLS resolution from
//! ~9% to a meaningfully higher rate by pinning calls like
//! `Assert.True` to the Assert class actually imported by the caller.

use crate::models::{FileInfo, FunctionInfo, TypeRelationship};
use crate::parsers::registry;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

/// One resolved caller → callee edge, with call-site line numbers.
pub struct CallEdge {
    pub caller: String,
    pub callee: String,
    /// Comma-separated sorted unique line numbers.
    pub call_lines: String,
    pub call_count: i64,
    /// Optional parser-preserved transfer spelling/mechanism. Empty for the
    /// ordinary language call resolver, populated by typed semantic sites.
    pub raw_targets: Option<String>,
    pub offsets: Option<String>,
    pub via: Option<String>,
    pub address_lines: Option<String>,
    /// Which tier pinned this edge — see [`Resolution`]. `None` for semantic
    /// (AGC control-transfer) edges, which do not go through the tiers.
    pub resolution: Option<String>,
    /// How many candidates survived the tiers at the emit point. `1` means the
    /// resolution was unambiguous; `> 1` means the site fanned out and this
    /// edge is one of several guesses for the same call.
    pub candidates: Option<i64>,
    /// True when the caller's file is the callee's file, or **directly**
    /// imports it. The structural check a name-resolved edge otherwise lacks:
    /// on a large corpus, `fetch()` resolving to a project function named
    /// `fetch` in a file the caller never imports is what makes "who calls X"
    /// unusable.
    ///
    /// **One hop, by construction.** A caller that imports a barrel which
    /// re-exports the callee — `import { Auth } from "@scope/llm/route"`
    /// reaching `route/auth.ts` through `route/index.ts` — is `false` here even
    /// though the call is real. Measured on a 3,293-file monorepo against a
    /// hand-labeled truth set, this accounts for every true edge the property
    /// mis-marks. Treat `import_backed = false` as *unconfirmed*, not as
    /// *refuted*: it is a strong filter for exploration and NOT safe as a
    /// deletion criterion. See `dev-docs/plans/graph-resolution-precision.md`
    /// Phase 5 for the numbers.
    pub import_backed: Option<bool>,
}

/// The tier that pinned a call site to its target, best precision first.
///
/// The declaration order IS the precision ranking, and it is load-bearing: one
/// edge aggregates every call site between the same pair, and when two sites
/// disagree the edge keeps the *best* tier (with `candidates` taking the
/// minimum). Without a fixed total order that merge would depend on iteration
/// order and the graph would stop being deterministic.
///
/// The ranking is by how much evidence pinned the target: an exact qualified
/// name is the call text itself; a receiver type is a real type constraint;
/// same-owner / namespace-import / same-file are scope constraints of
/// decreasing tightness; `UniqueName` is only "no other symbol in the project
/// has this name" (which is exactly how a project `fetch` absorbs every call
/// to the web global); `LangGroup` narrows by separator convention alone; and
/// `GlobalFallback` means no tier narrowed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Resolution {
    ExactQualified,
    Receiver,
    Inherited,
    SameOwner,
    NamespaceImport,
    SameFile,
    UniqueName,
    LangGroup,
    GlobalFallback,
}

impl Resolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Resolution::ExactQualified => "exact_qualified",
            Resolution::Receiver => "receiver",
            Resolution::Inherited => "inherited",
            Resolution::SameOwner => "same_owner",
            Resolution::NamespaceImport => "namespace_import",
            Resolution::SameFile => "same_file",
            Resolution::UniqueName => "unique_name",
            Resolution::LangGroup => "lang_group",
            Resolution::GlobalFallback => "global_fallback",
        }
    }
}

/// Aggregate counters describing how the resolver classified every call
/// site in one `build_call_edges` pass — the measurement substrate the
/// re-resolution phases (and the `code_tree_stats` dev bin) track.
///
/// The denominator for resolver *quality* is `total_calls - excluded_noise`.
/// Of those: `no_candidate` reference a bare name absent from the project
/// (external / stdlib — nothing we could resolve to); `ambiguous_dropped`
/// still had more than `max_targets` candidates after every tier;
/// `resolved_call_sites` matched at least one in-project symbol.
/// `resolved_edges` is the de-duplicated caller→callee pair count actually
/// emitted (one call site can fan out to several when tiers can't separate
/// overloads, and repeated calls on different lines collapse to one edge).
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct CallResolutionStats {
    pub total_calls: u64,
    pub excluded_noise: u64,
    pub no_candidate: u64,
    pub ambiguous_dropped: u64,
    pub resolved_call_sites: u64,
    pub resolved_edges: u64,
    /// Subset of `resolved_call_sites` pinned via the inheritance tier — a
    /// `self.method()` whose method is defined on an ancestor (EXTENDS /
    /// IMPLEMENTS), not the caller's own type. The headline win of the
    /// inheritance-aware resolution.
    pub resolved_via_inheritance: u64,
}

impl CallResolutionStats {
    pub(crate) fn merge(&mut self, other: Self) {
        self.total_calls += other.total_calls;
        self.excluded_noise += other.excluded_noise;
        self.no_candidate += other.no_candidate;
        self.ambiguous_dropped += other.ambiguous_dropped;
        self.resolved_call_sites += other.resolved_call_sites;
        self.resolved_edges += other.resolved_edges;
        self.resolved_via_inheritance += other.resolved_via_inheritance;
    }
}

/// Per-function scratch counters, summed into [`CallResolutionStats`] after
/// the parallel match loop. Kept `Copy` so the rayon reduce stays alloc-free.
#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    total: u64,
    excluded: u64,
    no_candidate: u64,
    ambiguous: u64,
    resolved: u64,
    inherited: u64,
}

/// Names excluded from call resolution for languages present in this build.
pub(crate) fn noise_names_for_files(files: &[FileInfo]) -> HashSet<&'static str> {
    let present_languages: HashSet<&str> =
        files.iter().map(|file| file.language.as_str()).collect();

    registry::LANGUAGES
        .iter()
        .filter(|language| present_languages.contains(language.id))
        .flat_map(|language| language.noise_names.iter().copied())
        .collect()
}

/// Terminal segment of a `::` / `.` / `/`-separated type name — the form
/// stored in `qname_to_owner`, so ancestor lookups match call candidates.
fn short_type_name(name: &str) -> &str {
    let mut cut = 0usize;
    for sep in ["::", ".", "/"] {
        if let Some(i) = name.rfind(sep) {
            let after = i + sep.len();
            if after > cut {
                cut = after;
            }
        }
    }
    &name[cut..]
}

/// type short-name → transitive ancestor short-names, derived from the
/// EXTENDS / IMPLEMENTS relationships in the parse. Borrowed from
/// `rels`, so the map lives as long as the caller's `type_relationships`.
fn build_ancestor_map(rels: &[TypeRelationship]) -> HashMap<&str, HashSet<&str>> {
    let mut parents: HashMap<&str, Vec<&str>> = HashMap::new();
    for tr in rels {
        if tr.relationship == "extends" || tr.relationship == "implements" {
            if let Some(tgt) = tr.target_type.as_deref() {
                parents
                    .entry(short_type_name(&tr.source_type))
                    .or_default()
                    .push(short_type_name(tgt));
            }
        }
    }
    let mut out: HashMap<&str, HashSet<&str>> = HashMap::with_capacity(parents.len());
    for &child in parents.keys() {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = parents.get(child).cloned().unwrap_or_default();
        while let Some(p) = stack.pop() {
            // Guard against inheritance cycles (malformed input) via `seen`.
            if p != child && seen.insert(p) {
                if let Some(gp) = parents.get(p) {
                    stack.extend(gp.iter().copied());
                }
            }
        }
        out.insert(child, seen);
    }
    out
}

/// Per-function output of the parallel match loop: borrowed
/// `(caller, callee, line, tier, surviving-candidate-count)` tuples plus that
/// function's [`Counts`].
type FnMatchResult<'a> = (Vec<(&'a str, &'a str, u32, Resolution, usize)>, Counts);

/// True if `qname` lives under any of the namespace prefixes in `scopes`.
/// A "lives under" match requires `qname` to start with `scope` followed by
/// a `.` or `::` separator — `System` matches `System.IO.Stream` but not
/// `Systemic`.
fn qname_starts_with_any(qname: &str, scopes: &[String]) -> bool {
    for scope in scopes {
        if scope.is_empty() {
            continue;
        }
        if qname.len() > scope.len()
            && qname.starts_with(scope.as_str())
            && (qname.as_bytes()[scope.len()] == b'.'
                || (qname.len() > scope.len() + 1 && &qname[scope.len()..scope.len() + 2] == "::"))
        {
            return true;
        }
    }
    false
}

/// D3: a closure-scoped definition — one the parser tagged with a non-zero
/// `nesting_depth` — is lexically reachable only inside its enclosing scope
/// unless it escapes. Every bare-name index over `functions` has to keep it
/// out of the global tier and offer it to its own file only.
pub(super) fn is_nested_function(function: &FunctionInfo) -> bool {
    function
        .metadata
        .get("nesting_depth")
        .and_then(|value| value.as_u64())
        .is_some_and(|depth| depth > 0)
}

/// D3 candidate assembly: the globally visible targets for a call name plus
/// the nested (closure-scoped) targets declared in the *caller's own file*.
///
/// Borrows both sides untouched in the common case — `storage` is filled only
/// when a name has candidates on both sides, which is rare.
pub(super) fn merge_candidates<'a, 'b>(
    global: Option<&'b Vec<&'a str>>,
    local: Option<&'b Vec<&'a str>>,
    storage: &'b mut Vec<&'a str>,
) -> &'b [&'a str] {
    match (global, local) {
        (Some(g), None) => g.as_slice(),
        (None, Some(l)) => l.as_slice(),
        (None, None) => &[],
        (Some(g), Some(l)) => {
            storage.reserve(g.len() + l.len());
            storage.extend_from_slice(g);
            storage.extend_from_slice(l);
            storage.as_slice()
        }
    }
}

fn infer_lang_group(qname: &str) -> &'static str {
    if qname.contains("::") {
        "rust_cpp"
    } else if qname.contains('/') {
        "go_ts_js"
    } else {
        "python_java"
    }
}

/// Run the 5-tier resolution over every parsed function's call sites.
///
/// Tiers (first non-empty wins):
///   0. Receiver hint: `Receiver.method` → narrow by owner short-name
///   0b. Inheritance: a self-call to a method defined on an ancestor (EXTENDS/IMPLEMENTS, not the caller's own type) resolves to the unique inherited definition
///   1. Same owner: caller and target share qualified prefix
///   2. Same file
///   3. Same language group (separator convention)
///   4. Global fallback (all targets with matching bare name)
///
/// Calls whose bare name appears in `excluded_names` are skipped (stdlib noise).
/// Calls with more than `max_targets` resolvable targets are dropped as too
/// ambiguous.
pub fn build_call_edges(
    functions: &[FunctionInfo],
    files: &[FileInfo],
    excluded_names: &std::collections::HashSet<&str>,
    max_targets: usize,
    type_relationships: &[TypeRelationship],
    file_import_pairs: &HashSet<(&str, &str)>,
) -> (Vec<CallEdge>, CallResolutionStats) {
    let verbose = std::env::var_os("KGLITE_CODE_TREE_VERBOSE").is_some();
    let t0 = std::time::Instant::now();
    // D3 — a closure-scoped definition (`nesting_depth > 0`) is lexically
    // callable only inside its enclosing scope unless it escapes. It
    // therefore never joins the global lookups; it is offered only to
    // callers in its own file. Without this the ~2 270 nested names opencode
    // gains would flood the bare-name index: the Phase 1 spike measured
    // multi-candidate names going 664 → 1 562 and 293 currently-unique names
    // turning ambiguous, which is exactly the false-positive class v0.1.5's
    // resolution metadata was built to keep out. Escape analysis (which
    // nested functions really do leak cross-file) is the deliberate
    // non-goal; same-file is the conservative approximation. The predicate
    // itself is `is_nested_function` above — `build_references_fn_edges` and
    // `build_decorates_edges` are bare-name indexes over the same population
    // and gate on it too.
    let is_nested = is_nested_function;

    // Bare name → every qualified_name that matches. Top-level definitions
    // only; nested ones go to `nested_names` below, keyed by file.
    let mut name_lookup: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut nested_names: HashMap<&str, HashMap<&str, Vec<&str>>> = HashMap::new();
    for fn_info in functions {
        let target = if is_nested(fn_info) {
            nested_names
                .entry(fn_info.file_path.as_str())
                .or_default()
                .entry(fn_info.name.as_str())
                .or_default()
        } else {
            name_lookup.entry(fn_info.name.as_str()).or_default()
        };
        target.push(fn_info.qualified_name.as_str());
    }
    // Exact qualified call text → one ordinary function or every overload in
    // that same-scope group. Parser call sites keep the source-level base ID,
    // while graph identities carry a signature discriminator.
    let mut qualified_lookup: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut nested_qualified: HashMap<&str, HashMap<&str, Vec<&str>>> = HashMap::new();
    for function in functions {
        let qualified_name = function.qualified_name.as_str();
        let mut keys: Vec<&str> = vec![qualified_name];
        if let Some(base) = super::overload_base_qualified_name(qualified_name) {
            keys.push(base);
        }
        for key in keys {
            if is_nested(function) {
                nested_qualified
                    .entry(function.file_path.as_str())
                    .or_default()
                    .entry(key)
                    .or_default()
                    .push(qualified_name);
            } else {
                qualified_lookup
                    .entry(key)
                    .or_default()
                    .push(qualified_name);
            }
        }
    }

    // qualified_name → owner short name (last segment of owner prefix).
    // qualified_name → owner prefix (everything before the final separator).
    let mut qname_to_owner: HashMap<&str, &str> = HashMap::new();
    let mut qname_to_prefix: HashMap<&str, &str> = HashMap::new();
    for fn_info in functions {
        let qn = fn_info.qualified_name.as_str();
        for sep in ["::", ".", "/"] {
            if let Some(idx) = qn.rfind(sep) {
                let owner_path = &qn[..idx];
                qname_to_prefix.insert(qn, owner_path);
                // Find the last separator inside owner_path (any of ::, ., /).
                let mut short = owner_path;
                for sep2 in ["::", ".", "/"] {
                    if let Some(i2) = owner_path.rfind(sep2) {
                        short = &owner_path[i2 + sep2.len()..];
                        break;
                    }
                }
                qname_to_owner.insert(qn, short);
                break;
            }
        }
    }

    // qualified_name → file_path (for tier 2).
    let qname_to_file: HashMap<&str, &str> = functions
        .iter()
        .map(|f| (f.qualified_name.as_str(), f.file_path.as_str()))
        .collect();

    // file_path → imported namespace prefixes. Empty for files whose
    // language doesn't track imports as namespace names.
    let file_imports: HashMap<&str, &Vec<String>> = files
        .iter()
        .filter(|f| !f.imports.is_empty())
        .map(|f| (f.path.as_str(), &f.imports))
        .collect();

    // type short-name → transitive ancestors, for the inheritance tier.
    let ancestors = build_ancestor_map(type_relationships);

    if verbose {
        eprintln!(
            "[calls]     lookup build: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }
    let t_match = std::time::Instant::now();

    // Parallelise the per-function match loop: each caller's edges are
    // independent, so we collect per-function edge vectors and merge.
    // Keys stay as &str (borrowed from `functions`) to avoid alloc per edge.
    let per_fn: Vec<FnMatchResult> = functions
        .par_iter()
        .map(|fn_info| {
            let caller_qn = fn_info.qualified_name.as_str();
            let caller_lang = infer_lang_group(caller_qn);
            let caller_prefix = qname_to_prefix.get(caller_qn).copied();
            let caller_owner = qname_to_owner.get(caller_qn).copied();
            let caller_file = fn_info.file_path.as_str();
            // D3: the nested definitions this caller may see — its own file's,
            // and nothing else's.
            let local_names = nested_names.get(caller_file);
            let local_qualified = nested_qualified.get(caller_file);

            let mut out: Vec<(&str, &str, u32, Resolution, usize)> = Vec::new();
            let mut counts = Counts::default();

            for (called_name, line) in &fn_info.calls {
                counts.total += 1;
                let noise_name = called_name.rsplit('.').next().unwrap_or(called_name);
                if excluded_names.contains(noise_name) {
                    counts.excluded += 1;
                    continue;
                }
                let mut exact_storage: Vec<&str> = Vec::new();
                let exact: &[&str] = merge_candidates(
                    qualified_lookup.get(called_name.as_str()),
                    local_qualified.and_then(|m| m.get(called_name.as_str())),
                    &mut exact_storage,
                );
                if !exact.is_empty() {
                    counts.resolved += 1;
                    for &target in exact {
                        if target != caller_qn {
                            out.push((
                                caller_qn,
                                target,
                                *line,
                                Resolution::ExactQualified,
                                exact.len(),
                            ));
                        }
                    }
                    continue;
                }
                let (explicit_hint, method_name) = match called_name.rfind('.') {
                    Some(idx) => (Some(&called_name[..idx]), &called_name[idx + 1..]),
                    None => (None, called_name.as_str()),
                };

                let mut name_storage: Vec<&str> = Vec::new();
                let candidates: &[&str] = merge_candidates(
                    name_lookup.get(method_name),
                    local_names.and_then(|m| m.get(method_name)),
                    &mut name_storage,
                );
                if candidates.is_empty() {
                    counts.no_candidate += 1;
                    continue;
                }

                if candidates.len() == 1 {
                    counts.resolved += 1;
                    let target = candidates[0];
                    if target != caller_qn {
                        out.push((caller_qn, target, *line, Resolution::UniqueName, 1));
                    }
                    continue;
                }

                let mut targets: &[&str] = candidates;
                let mut filtered: Vec<&str>;

                // Tier 0: receiver-type filter. Two sources of hints —
                // `(explicit_hint, owner_short_match)`:
                //
                //   - Explicit hint from `obj.method()` — the receiver
                //     identifier's text. Already extracted at parse time
                //     (e.g. `cfg.read` becomes `("cfg", "read")`).
                //   - Implicit hint from `self.method()` / bare-name
                //     calls inside a method body — use the caller's own
                //     owner short name as the receiver type. Resolves
                //     `Foo::caller -> Foo::method` correctly when the
                //     same method name exists on multiple structs.
                let implicit_hint = if explicit_hint.is_none() {
                    caller_owner
                } else {
                    None
                };
                let mut owner_hint_hit = false;
                // Tracks the LAST tier that actually narrowed the candidate
                // set. `GlobalFallback` means none of them did.
                let mut tier = Resolution::GlobalFallback;
                if let Some(hint) = explicit_hint.or(implicit_hint) {
                    filtered = targets
                        .iter()
                        .copied()
                        .filter(|t| qname_to_owner.get(t).copied() == Some(hint))
                        .collect();
                    if !filtered.is_empty() {
                        targets = &filtered[..];
                        owner_hint_hit = true;
                        tier = Resolution::Receiver;
                    }
                }

                // Inheritance tier: a `self.method()` whose method isn't
                // defined on the caller's own type resolves to the method
                // *inherited* from an ancestor (EXTENDS / IMPLEMENTS). Only
                // fires for implicit (self) calls whose direct-owner filter
                // above found nothing — `obj.method()` is left alone (we
                // can't infer obj's type). Conservative: a unique inherited
                // definition resolves immediately; a diamond narrows the set
                // and defers to the later tiers.
                if !owner_hint_hit && targets.len() > 1 {
                    if let Some(owner) = implicit_hint {
                        if let Some(anc) = ancestors.get(owner) {
                            let inh: Vec<&str> = candidates
                                .iter()
                                .copied()
                                .filter(|t| {
                                    qname_to_owner
                                        .get(t)
                                        .copied()
                                        .is_some_and(|o| anc.contains(o))
                                })
                                .collect();
                            if inh.len() == 1 {
                                counts.resolved += 1;
                                counts.inherited += 1;
                                if inh[0] != caller_qn {
                                    out.push((caller_qn, inh[0], *line, Resolution::Inherited, 1));
                                }
                                continue;
                            } else if !inh.is_empty() {
                                filtered = inh;
                                targets = &filtered[..];
                                tier = Resolution::Inherited;
                            }
                        }
                    }
                }

                if targets.len() > 1 {
                    if let Some(prefix) = caller_prefix {
                        let narrowed: Vec<&str> = targets
                            .iter()
                            .copied()
                            .filter(|t| qname_to_prefix.get(t).copied() == Some(prefix))
                            .collect();
                        if !narrowed.is_empty() {
                            filtered = narrowed;
                            targets = &filtered[..];
                            tier = Resolution::SameOwner;
                        }
                    }
                }

                // Tier 2.5: namespace-import scope. Prefer candidates whose
                // qname lives under a namespace the caller's file imports
                // (or under the caller's own namespace). Critical for
                // disambiguating `Assert.True` across xunit / fluentassertions
                // / project-local assertion helpers — the caller's `using`
                // list pins the correct one.
                if targets.len() > 1 {
                    if let Some(imports) = file_imports.get(caller_file) {
                        let narrowed: Vec<&str> = targets
                            .iter()
                            .copied()
                            .filter(|t| qname_starts_with_any(t, imports))
                            .collect();
                        if !narrowed.is_empty() {
                            filtered = narrowed;
                            targets = &filtered[..];
                            tier = Resolution::NamespaceImport;
                        }
                    }
                }

                if targets.len() > 1 {
                    let narrowed: Vec<&str> = targets
                        .iter()
                        .copied()
                        .filter(|t| qname_to_file.get(t).copied() == Some(caller_file))
                        .collect();
                    if !narrowed.is_empty() {
                        filtered = narrowed;
                        targets = &filtered[..];
                        tier = Resolution::SameFile;
                    }
                }

                if targets.len() > 1 {
                    let narrowed: Vec<&str> = targets
                        .iter()
                        .copied()
                        .filter(|t| infer_lang_group(t) == caller_lang)
                        .collect();
                    if !narrowed.is_empty() {
                        filtered = narrowed;
                        targets = &filtered[..];
                        tier = Resolution::LangGroup;
                    }
                }

                if targets.len() > max_targets {
                    counts.ambiguous += 1;
                    continue;
                }

                counts.resolved += 1;
                for &target in targets {
                    if target != caller_qn {
                        out.push((caller_qn, target, *line, tier, targets.len()));
                    }
                }
            }
            (out, counts)
        })
        .collect();

    // Aggregate per-function counters into the pass-level stats.
    let mut stats = CallResolutionStats::default();
    for (_, c) in &per_fn {
        stats.total_calls += c.total;
        stats.excluded_noise += c.excluded;
        stats.no_candidate += c.no_candidate;
        stats.ambiguous_dropped += c.ambiguous;
        stats.resolved_call_sites += c.resolved;
        stats.resolved_via_inheritance += c.inherited;
    }

    // Merge into the final dedupe map sequentially — 200K inserts is ~5ms.
    // One edge aggregates every call site between the pair, so the tier and
    // candidate count must merge too: keep the BEST tier seen (the `Ord` on
    // `Resolution` is the documented precision ranking) and the MINIMUM
    // candidate count. Both are order-independent, which is what keeps the
    // three-build determinism gate green.
    let total: usize = per_fn.iter().map(|(v, _)| v.len()).sum();
    let mut seen: HashMap<(&str, &str), (Vec<u32>, Resolution, usize)> =
        HashMap::with_capacity(total);
    for (edges, _) in per_fn {
        for (caller, callee, line, tier, candidates) in edges {
            let entry = seen
                .entry((caller, callee))
                .or_insert_with(|| (Vec::new(), tier, candidates));
            entry.0.push(line);
            entry.1 = entry.1.min(tier);
            entry.2 = entry.2.min(candidates);
        }
    }

    if verbose {
        eprintln!(
            "[calls]     match loop:   {:.3}s ({} entries)",
            t_match.elapsed().as_secs_f64(),
            seen.len()
        );
    }
    let t_out = std::time::Instant::now();

    // Sort keys for deterministic output (match Python's ordered dict).
    let mut keys: Vec<(&str, &str)> = seen.keys().copied().collect();
    keys.sort_unstable();

    let result: Vec<CallEdge> = keys
        .into_iter()
        .map(|(caller, callee)| {
            let (mut lines, tier, candidates) = seen.remove(&(caller, callee)).unwrap_or((
                Vec::new(),
                Resolution::GlobalFallback,
                0,
            ));
            lines.sort_unstable();
            lines.dedup();
            let count = lines.len() as i64;
            let mut call_lines = String::with_capacity(lines.len() * 4);
            for (i, l) in lines.iter().enumerate() {
                if i > 0 {
                    call_lines.push(',');
                }
                use std::fmt::Write;
                let _ = write!(call_lines, "{}", l);
            }
            // A same-file call needs no import to be structurally backed;
            // otherwise the caller's file must actually import the callee's.
            let caller_file = qname_to_file.get(caller).copied();
            let callee_file = qname_to_file.get(callee).copied();
            let import_backed = match (caller_file, callee_file) {
                (Some(from), Some(to)) => from == to || file_import_pairs.contains(&(from, to)),
                _ => false,
            };
            CallEdge {
                caller: caller.to_string(),
                callee: callee.to_string(),
                call_lines,
                call_count: count,
                raw_targets: None,
                offsets: None,
                via: None,
                address_lines: None,
                resolution: Some(tier.as_str().to_string()),
                candidates: Some(candidates as i64),
                import_backed: Some(import_backed),
            }
        })
        .collect();
    stats.resolved_edges = result.len() as u64;
    if verbose {
        eprintln!(
            "[calls]     output build: {:.3}s ({} edges, {}/{} call sites resolved)",
            t_out.elapsed().as_secs_f64(),
            stats.resolved_edges,
            stats.resolved_call_sites,
            stats.total_calls.saturating_sub(stats.excluded_noise),
        );
    }
    (result, stats)
}

#[cfg(test)]
mod stats_tests {
    use super::*;
    use crate::models::{FileInfo, FunctionInfo, TypeRelationship};

    /// A build with no resolved file imports — `import_backed` then reduces to
    /// "caller and callee share a file".
    fn no_imports() -> HashSet<(&'static str, &'static str)> {
        HashSet::new()
    }

    fn func(qn: &str, file: &str, calls: &[(&str, u32)]) -> FunctionInfo {
        FunctionInfo {
            name: qn.rsplit(['.', ':']).next().unwrap_or(qn).to_string(),
            qualified_name: qn.to_string(),
            file_path: file.to_string(),
            calls: calls.iter().map(|(n, l)| (n.to_string(), *l)).collect(),
            ..Default::default()
        }
    }

    /// `(callee, resolution, candidates, import_backed)` for every edge,
    /// sorted — the property assertions read off this.
    fn meta(edges: &[CallEdge]) -> Vec<(&str, &str, i64, bool)> {
        let mut rows: Vec<_> = edges
            .iter()
            .map(|e| {
                (
                    e.callee.as_str(),
                    e.resolution.as_deref().expect("resolution set"),
                    e.candidates.expect("candidates set"),
                    e.import_backed.expect("import_backed set"),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// D3: mark a function as closure-scoped, the way the TS scope walk does.
    fn nested(mut f: FunctionInfo, depth: u64) -> FunctionInfo {
        f.metadata
            .insert("nesting_depth".into(), serde_json::json!(depth));
        f
    }

    /// D3 — a `nesting_depth > 0` definition never joins the global bare-name
    /// lookup. Naively merging opencode's ~2 270 nested names into it took
    /// multi-candidate names from 664 to 1 562 and flipped 293 unique names
    /// ambiguous (Phase 1 spike), which is exactly the false-positive class
    /// v0.1.5's resolution metadata exists to keep out.
    #[test]
    fn a_nested_definition_is_invisible_to_a_caller_in_another_file() {
        let functions = vec![
            func("b.consume", "b.ts", &[("helper", 3)]),
            nested(func("a.outer.helper", "a.ts", &[]), 1),
        ];
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        assert!(
            edges.is_empty(),
            "leaked cross-file edges: {:?}",
            edges
                .iter()
                .map(|e| (&e.caller, &e.callee))
                .collect::<Vec<_>>()
        );
    }

    /// …but a caller in its own file resolves it, top-level or nested.
    #[test]
    fn a_nested_definition_resolves_for_callers_in_its_own_file() {
        let functions = vec![
            func("a.outer", "a.ts", &[("helper", 3)]),
            nested(func("a.outer.helper", "a.ts", &[("deeper", 5)]), 1),
            nested(func("a.outer.helper.deeper", "a.ts", &[]), 2),
        ];
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        let mut pairs: Vec<_> = edges
            .iter()
            .map(|e| (e.caller.as_str(), e.callee.as_str()))
            .collect();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("a.outer", "a.outer.helper"),
                ("a.outer.helper", "a.outer.helper.deeper"),
            ]
        );
    }

    /// The gating is by *name visibility*, not by dropping the node: a
    /// nested definition in one file must not shadow or suppress an
    /// identically named top-level export in another.
    #[test]
    fn a_nested_name_does_not_disturb_the_global_lookup() {
        let functions = vec![
            func("b.consume", "b.ts", &[("helper", 3)]),
            func("c.helper", "c.ts", &[]),
            nested(func("a.outer.helper", "a.ts", &[]), 1),
        ];
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        assert_eq!(
            meta(&edges),
            vec![("c.helper", "unique_name", 1, false)],
            "the cross-file caller must see exactly the top-level `helper`"
        );
    }

    #[test]
    fn unique_name_tier_is_labelled_and_counted() {
        let functions = vec![
            func("a.foo", "a.py", &[("bar", 1)]),
            func("a.bar", "a.py", &[]),
        ];
        let files = vec![FileInfo {
            path: "a.py".into(),
            ..Default::default()
        }];
        let (edges, _) = build_call_edges(
            &functions,
            &files,
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        // Same file, so import_backed holds without any IMPORTS edge.
        assert_eq!(meta(&edges), vec![("a.bar", "unique_name", 1, true)]);
    }

    #[test]
    fn exact_qualified_tier_is_labelled() {
        let caller = func("m.START", "m.agc", &[("m.P61", 7)]);
        let mut target = func("m.P61", "other.agc", &[]);
        target.name = "P61".into();
        let (edges, _) = build_call_edges(
            &[caller, target],
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        assert_eq!(meta(&edges), vec![("m.P61", "exact_qualified", 1, false)]);
    }

    #[test]
    fn receiver_tier_is_labelled() {
        // `read` exists on two owners; the explicit receiver pins one.
        let functions = vec![
            func("m.Cfg.read", "a.py", &[]),
            func("m.Db.read", "b.py", &[]),
            func("m.caller", "c.py", &[("Cfg.read", 3)]),
        ];
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        assert_eq!(meta(&edges), vec![("m.Cfg.read", "receiver", 1, false)]);
    }

    #[test]
    fn inherited_tier_is_labelled() {
        let files = vec![FileInfo {
            path: "a.py".into(),
            ..Default::default()
        }];
        let functions = vec![
            func("mod.Base.m", "a.py", &[]),
            func("mod.Other.m", "a.py", &[]),
            func("mod.Derived.caller", "a.py", &[("m", 1)]),
        ];
        let rels = vec![TypeRelationship {
            source_type: "mod.Derived".into(),
            target_type: Some("mod.Base".into()),
            relationship: "extends".into(),
            methods: vec![],
        }];
        let (edges, _) = build_call_edges(
            &functions,
            &files,
            &std::collections::HashSet::new(),
            5,
            &rels,
            &no_imports(),
        );
        assert_eq!(meta(&edges), vec![("mod.Base.m", "inherited", 1, true)]);
    }

    #[test]
    fn namespace_import_tier_is_labelled() {
        // Two `True`s; the caller's file imports only one namespace, so the
        // namespace-import tier is what narrows. This tier had no test at all
        // before — it was added for dotnet/runtime and never covered.
        let functions = vec![
            func("xunit.Assert.True", "x.cs", &[]),
            func("fluent.Assert.True", "f.cs", &[]),
            func("app.caller", "app.cs", &[("True", 4)]),
        ];
        let files = vec![FileInfo {
            path: "app.cs".into(),
            imports: vec!["xunit".into()],
            ..Default::default()
        }];
        let (edges, _) = build_call_edges(
            &functions,
            &files,
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        assert_eq!(
            meta(&edges),
            vec![("xunit.Assert.True", "namespace_import", 1, false)]
        );
    }

    #[test]
    fn fan_out_records_the_surviving_candidate_count_on_every_edge() {
        // Two same-named targets no tier can separate: both get an edge, and
        // BOTH must say `candidates: 2` — that is the annotation that makes a
        // fan-out guess distinguishable from a pinned resolution.
        let functions = vec![
            func("one/a.fetch", "one/a.ts", &[]),
            func("two/b.fetch", "two/b.ts", &[]),
            func("three/c.caller", "three/c.ts", &[("fetch", 9)]),
        ];
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        assert_eq!(
            meta(&edges),
            vec![
                ("one/a.fetch", "lang_group", 2, false),
                ("two/b.fetch", "lang_group", 2, false),
            ]
        );
    }

    #[test]
    fn import_backed_follows_the_resolved_file_import_set() {
        let functions = vec![
            func("a.helper", "a.ts", &[]),
            func("b.caller", "b.ts", &[("helper", 1)]),
            func("c.caller", "c.ts", &[("helper", 1)]),
        ];
        // b.ts imports a.ts; c.ts does not.
        let imports: HashSet<(&str, &str)> = HashSet::from([("b.ts", "a.ts")]);
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &imports,
        );
        let backed: Vec<(&str, bool)> = {
            let mut v: Vec<_> = edges
                .iter()
                .map(|e| (e.caller.as_str(), e.import_backed.expect("set")))
                .collect();
            v.sort();
            v
        };
        assert_eq!(backed, vec![("b.caller", true), ("c.caller", false)]);
    }

    #[test]
    fn import_backed_is_one_hop_and_does_not_follow_re_exports() {
        // `c.ts` imports the barrel `b.ts`, which imports `a.ts` where the
        // callee lives. The call is real; `import_backed` is still false.
        // Pinned deliberately: this is the documented limit of the property,
        // and it is exactly why Track C's Phase 5 drop rule was NOT adopted —
        // a one-hop check would have deleted true edges. Do not "fix" this
        // test by making the check transitive without re-measuring the false
        // positives that would come back with it.
        let functions = vec![
            func("a.helper", "a.ts", &[]),
            func("c.caller", "c.ts", &[("helper", 1)]),
        ];
        let imports: HashSet<(&str, &str)> = HashSet::from([("c.ts", "b.ts"), ("b.ts", "a.ts")]);
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &imports,
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].import_backed, Some(false));
    }

    #[test]
    fn aggregation_keeps_the_best_tier_and_the_smallest_candidate_count() {
        // `m.caller` reaches `m.Cfg.read` twice: once by exact qualified name
        // (1 candidate) and once bare, which only the lang-group tier narrows
        // (2 candidates). The single aggregated edge must report the BETTER
        // tier and the SMALLER count — and must do so regardless of the order
        // the sites were seen in.
        let functions = vec![
            func("m.Cfg.read", "a.py", &[]),
            func("m.Db.read", "b.py", &[]),
            func("m.caller", "c.py", &[("m.Cfg.read", 1), ("read", 2)]),
        ];
        let (edges, _) = build_call_edges(
            &functions,
            &[],
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );
        let cfg = edges
            .iter()
            .find(|e| e.callee == "m.Cfg.read")
            .expect("edge to m.Cfg.read");
        assert_eq!(cfg.resolution.as_deref(), Some("exact_qualified"));
        assert_eq!(cfg.candidates, Some(1));
        assert_eq!(cfg.call_lines, "1,2");
    }

    #[test]
    fn stats_classify_every_call_site() {
        // `a.foo` calls `bar` twice (resolvable, single candidate → 1 edge),
        // an external name (no candidate), and a noise name (excluded).
        let functions = vec![
            func(
                "a.foo",
                "a.py",
                &[("bar", 1), ("bar", 2), ("external_thing", 3), ("noisy", 4)],
            ),
            func("a.bar", "a.py", &[]),
        ];
        let files = vec![FileInfo {
            path: "a.py".into(),
            ..Default::default()
        }];
        let mut noise = std::collections::HashSet::new();
        noise.insert("noisy");

        let (edges, stats) = build_call_edges(&functions, &files, &noise, 5, &[], &no_imports());

        assert_eq!(stats.total_calls, 4);
        assert_eq!(stats.excluded_noise, 1);
        assert_eq!(stats.no_candidate, 1);
        assert_eq!(stats.ambiguous_dropped, 0);
        assert_eq!(stats.resolved_call_sites, 2); // two `bar` sites
        assert_eq!(stats.resolved_edges, 1); // collapsed to one a.foo→a.bar edge
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn exact_qualified_name_precedes_receiver_dot_parsing() {
        let caller = func(
            "Comanche055.START",
            "Comanche055/MAIN.agc",
            &[("Comanche055.P61.1", 7)],
        );
        let mut target = func("Comanche055.P61.1", "Comanche055/P61-P67.agc", &[]);
        target.name = "P61.1".into();
        let functions = vec![caller, target];
        let files = vec![FileInfo {
            path: "Comanche055/MAIN.agc".into(),
            language: "agc".into(),
            ..Default::default()
        }];

        let (edges, stats) = build_call_edges(
            &functions,
            &files,
            &std::collections::HashSet::new(),
            5,
            &[],
            &no_imports(),
        );

        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.resolved_call_sites, 1);
        assert_eq!(stats.no_candidate, 0);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].caller, "Comanche055.START");
        assert_eq!(edges[0].callee, "Comanche055.P61.1");
        assert_eq!(edges[0].call_lines, "7");
    }

    #[test]
    fn exact_qualified_name_does_not_bypass_noise_filtering() {
        let caller = func(
            "Comanche055.START",
            "Comanche055/MAIN.agc",
            &[("Comanche055.WAITLIST", 8)],
        );
        let target = func("Comanche055.WAITLIST", "Comanche055/WAITLIST.agc", &[]);
        let functions = vec![caller, target];
        let mut noise = std::collections::HashSet::new();
        noise.insert("WAITLIST");

        let (edges, stats) = build_call_edges(&functions, &[], &noise, 5, &[], &no_imports());

        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.excluded_noise, 1);
        assert_eq!(stats.resolved_call_sites, 0);
        assert!(edges.is_empty());
    }

    #[test]
    fn inheritance_tier_resolves_self_call_to_ancestor_method() {
        // `m` exists on Base and Other. Derived extends Base. A self-call to
        // `m()` from Derived must resolve to the inherited Base.m — not Other.m
        // (which the same-file / global fallbacks could otherwise pick).
        let files = vec![FileInfo {
            path: "a.py".into(),
            ..Default::default()
        }];
        let functions = vec![
            func("mod.Base.m", "a.py", &[]),
            func("mod.Other.m", "a.py", &[]),
            func("mod.Derived.caller", "a.py", &[("m", 1)]),
        ];
        let rels = vec![TypeRelationship {
            source_type: "mod.Derived".into(),
            target_type: Some("mod.Base".into()),
            relationship: "extends".into(),
            methods: vec![],
        }];
        let noise = std::collections::HashSet::new();

        let (edges, stats) = build_call_edges(&functions, &files, &noise, 5, &rels, &no_imports());

        assert_eq!(stats.resolved_via_inheritance, 1);
        let pairs: Vec<(&str, &str)> = edges
            .iter()
            .map(|e| (e.caller.as_str(), e.callee.as_str()))
            .collect();
        assert!(pairs.contains(&("mod.Derived.caller", "mod.Base.m")));
        assert!(!pairs.iter().any(|(_, callee)| *callee == "mod.Other.m"));
    }

    #[test]
    fn no_type_relationships_means_no_inheritance_resolution() {
        // Same shape, but without the EXTENDS relationship the self-call to a
        // multi-owner `m` is left to the ordinary tiers (no inheritance pin).
        let files = vec![FileInfo {
            path: "a.py".into(),
            ..Default::default()
        }];
        let functions = vec![
            func("mod.Base.m", "a.py", &[]),
            func("mod.Other.m", "a.py", &[]),
            func("mod.Derived.caller", "a.py", &[("m", 1)]),
        ];
        let noise = std::collections::HashSet::new();

        let (_edges, stats) = build_call_edges(&functions, &files, &noise, 5, &[], &no_imports());
        assert_eq!(stats.resolved_via_inheritance, 0);
    }

    #[test]
    fn foreign_noise_does_not_hide_python_calls() {
        let functions = vec![
            func("mod.caller", "main.py", &[("find", 3)]),
            func("mod.find", "main.py", &[]),
        ];
        let python_files = vec![FileInfo {
            path: "main.py".into(),
            language: "python".into(),
            ..Default::default()
        }];

        let python_noise = noise_names_for_files(&python_files);
        let (edges, stats) = build_call_edges(
            &functions,
            &python_files,
            &python_noise,
            5,
            &[],
            &no_imports(),
        );

        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.excluded_noise, 0);
        assert_eq!(stats.resolved_call_sites, 1);
        assert_eq!(stats.resolved_edges, 1);
        assert_eq!(edges.len(), 1);

        let mut polyglot_files = python_files;
        polyglot_files.push(FileInfo {
            path: "native.cpp".into(),
            language: "cpp".into(),
            ..Default::default()
        });

        let polyglot_noise = noise_names_for_files(&polyglot_files);
        let (edges, stats) = build_call_edges(
            &functions,
            &polyglot_files,
            &polyglot_noise,
            5,
            &[],
            &no_imports(),
        );

        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.excluded_noise, 1);
        assert_eq!(stats.resolved_call_sites, 0);
        assert_eq!(stats.resolved_edges, 0);
        assert!(edges.is_empty());
    }
}
