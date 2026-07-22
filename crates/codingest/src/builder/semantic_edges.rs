//! Exact-qualified semantic edges emitted by parsers with richer site models.
//!
//! Unlike the generic call/reference resolvers, this module never falls back
//! from a qualified target to a same-named symbol elsewhere. A parser must
//! establish the namespace first; the builder then either finds that exact
//! graph identity or records the site as unresolved.

use crate::builder::call_edges::{CallEdge, CallResolutionStats};
use crate::builder::other_edges::ReferencesEdge;
use crate::models::{
    ConstantInfo, ControlTransferInfo, ControlTransferKind, FunctionInfo, ReferenceAccess,
    ReferenceSiteInfo, SymbolRelationshipInfo, SymbolRelationshipKind, SymbolTargetKind,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlEdge {
    pub caller: String,
    pub target: String,
    pub transfer_lines: String,
    pub transfer_count: i64,
    pub raw_targets: Option<String>,
    pub offsets: Option<String>,
    pub via: Option<String>,
    pub address_lines: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControlResolutionStats {
    pub total_jumps: u64,
    pub resolved_jumps: u64,
    pub total_branches: u64,
    pub resolved_branches: u64,
    pub indirect_transfers: u64,
}

#[derive(Default)]
pub struct ControlEdgeOutput {
    pub calls: Vec<CallEdge>,
    pub jumps: Vec<ControlEdge>,
    pub branches: Vec<ControlEdge>,
    pub call_stats: CallResolutionStats,
    pub control_stats: ControlResolutionStats,
}

#[derive(Default)]
struct SiteAggregate {
    lines: BTreeSet<u32>,
    raw_targets: BTreeSet<String>,
    offsets: BTreeSet<String>,
    via: BTreeSet<String>,
    address_lines: BTreeSet<u32>,
}

impl SiteAggregate {
    fn add(&mut self, site: &ControlTransferInfo) {
        self.lines.insert(site.line);
        if !site.raw_operand.is_empty() {
            self.raw_targets.insert(site.raw_operand.clone());
        }
        if let Some(offset) = &site.offset {
            self.offsets.insert(offset.clone());
        }
        if let Some(via) = &site.via {
            self.via.insert(via.clone());
        }
        if let Some(line) = site.address_line {
            self.address_lines.insert(line);
        }
    }
}

fn join_strings(values: &BTreeSet<String>) -> Option<String> {
    (!values.is_empty()).then(|| values.iter().cloned().collect::<Vec<_>>().join(","))
}

fn join_lines(values: &BTreeSet<u32>) -> String {
    values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn optional_lines(values: &BTreeSet<u32>) -> Option<String> {
    (!values.is_empty()).then(|| join_lines(values))
}

/// Resolve typed transfer sites against exact Function identities.
pub fn build_control_edges(
    sites: &[ControlTransferInfo],
    functions: &[FunctionInfo],
) -> ControlEdgeOutput {
    let known: HashSet<&str> = functions
        .iter()
        .map(|function| function.qualified_name.as_str())
        .collect();
    let mut call_aggregates: BTreeMap<(String, String), SiteAggregate> = BTreeMap::new();
    let mut jump_aggregates: BTreeMap<(String, String), SiteAggregate> = BTreeMap::new();
    let mut branch_aggregates: BTreeMap<(String, String), SiteAggregate> = BTreeMap::new();
    let mut output = ControlEdgeOutput::default();

    let mut ordered = sites.to_vec();
    ordered.sort();
    ordered.dedup();

    for site in &ordered {
        let target = site.target.as_deref();
        let exact = target.is_some_and(|candidate| known.contains(candidate));
        match site.kind {
            ControlTransferKind::Call => {
                output.call_stats.total_calls += 1;
                if exact {
                    output.call_stats.resolved_call_sites += 1;
                    let target = target.expect("exact target exists");
                    if target != site.caller {
                        call_aggregates
                            .entry((site.caller.clone(), target.to_string()))
                            .or_default()
                            .add(site);
                    }
                } else {
                    output.call_stats.no_candidate += 1;
                }
            }
            ControlTransferKind::Jump => {
                output.control_stats.total_jumps += 1;
                if exact {
                    output.control_stats.resolved_jumps += 1;
                    jump_aggregates
                        .entry((
                            site.caller.clone(),
                            target.expect("exact target exists").to_string(),
                        ))
                        .or_default()
                        .add(site);
                }
            }
            ControlTransferKind::Branch => {
                output.control_stats.total_branches += 1;
                if exact {
                    output.control_stats.resolved_branches += 1;
                    branch_aggregates
                        .entry((
                            site.caller.clone(),
                            target.expect("exact target exists").to_string(),
                        ))
                        .or_default()
                        .add(site);
                }
            }
            ControlTransferKind::IndirectCall => {
                output.call_stats.total_calls += 1;
                output.call_stats.no_candidate += 1;
                output.control_stats.indirect_transfers += 1;
            }
            ControlTransferKind::IndirectJump => {
                output.control_stats.total_jumps += 1;
                output.control_stats.indirect_transfers += 1;
            }
        }
    }

    output.calls = call_aggregates
        .into_iter()
        .map(|((caller, callee), aggregate)| CallEdge {
            caller,
            callee,
            call_lines: join_lines(&aggregate.lines),
            call_count: aggregate.lines.len() as i64,
            raw_targets: join_strings(&aggregate.raw_targets),
            offsets: join_strings(&aggregate.offsets),
            via: join_strings(&aggregate.via),
            address_lines: optional_lines(&aggregate.address_lines),
        })
        .collect();
    output.call_stats.resolved_edges = output.calls.len() as u64;
    output.jumps = control_edges(jump_aggregates);
    output.branches = control_edges(branch_aggregates);
    output
}

fn control_edges(aggregates: BTreeMap<(String, String), SiteAggregate>) -> Vec<ControlEdge> {
    aggregates
        .into_iter()
        .map(|((caller, target), aggregate)| ControlEdge {
            caller,
            target,
            transfer_lines: join_lines(&aggregate.lines),
            transfer_count: aggregate.lines.len() as i64,
            raw_targets: join_strings(&aggregate.raw_targets),
            offsets: join_strings(&aggregate.offsets),
            via: join_strings(&aggregate.via),
            address_lines: optional_lines(&aggregate.address_lines),
        })
        .collect()
}

#[derive(Default)]
struct ReferenceAggregate {
    lines: BTreeSet<u32>,
    opcodes: BTreeSet<String>,
    accesses: BTreeSet<&'static str>,
    has_read: bool,
    has_write: bool,
    has_address: bool,
}

fn access_name(access: ReferenceAccess) -> &'static str {
    match access {
        ReferenceAccess::Read => "read",
        ReferenceAccess::Write => "write",
        ReferenceAccess::ReadWrite => "read_write",
        ReferenceAccess::Address => "address",
        ReferenceAccess::Unknown => "unknown",
    }
}

/// Resolve typed reference sites against exact Constant identities and
/// aggregate multiple source sites without duplicating graph edges.
pub fn build_reference_site_edges(
    sites: &[ReferenceSiteInfo],
    functions: &[FunctionInfo],
    constants: &[ConstantInfo],
) -> Vec<ReferencesEdge> {
    let known_functions: HashSet<&str> = functions
        .iter()
        .map(|function| function.qualified_name.as_str())
        .collect();
    let known_constants: HashSet<&str> = constants
        .iter()
        .map(|constant| constant.qualified_name.as_str())
        .collect();
    let mut aggregates: BTreeMap<(String, String), ReferenceAggregate> = BTreeMap::new();

    let mut ordered = sites.to_vec();
    ordered.sort();
    ordered.dedup();
    for site in ordered {
        if !known_functions.contains(site.caller.as_str())
            || !known_constants.contains(site.target.as_str())
        {
            continue;
        }
        let aggregate = aggregates.entry((site.caller, site.target)).or_default();
        aggregate.lines.insert(site.line);
        aggregate.opcodes.insert(site.opcode);
        aggregate.accesses.insert(access_name(site.access));
        aggregate.has_read |= matches!(
            site.access,
            ReferenceAccess::Read | ReferenceAccess::ReadWrite
        );
        aggregate.has_write |= matches!(
            site.access,
            ReferenceAccess::Write | ReferenceAccess::ReadWrite
        );
        aggregate.has_address |= site.access == ReferenceAccess::Address;
    }

    aggregates
        .into_iter()
        .map(|((function, constant), aggregate)| ReferencesEdge {
            function,
            constant,
            line: aggregate.lines.first().copied().unwrap_or_default(),
            reference_lines: Some(join_lines(&aggregate.lines)),
            reference_count: Some(aggregate.lines.len() as i64),
            opcodes: join_strings(&aggregate.opcodes),
            accesses: Some(
                aggregate
                    .accesses
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            has_read: Some(aggregate.has_read),
            has_write: Some(aggregate.has_write),
            has_address: Some(aggregate.has_address),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEdge {
    pub source: String,
    pub target: String,
    pub line: u32,
    pub raw_target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEdgeBatch {
    pub relationship: &'static str,
    pub target_node_type: &'static str,
    pub edges: Vec<SymbolEdge>,
}

fn relationship_name(kind: SymbolRelationshipKind) -> &'static str {
    match kind {
        SymbolRelationshipKind::AliasOf => "ALIAS_OF",
        SymbolRelationshipKind::PointsTo => "POINTS_TO",
    }
}

fn target_node_type(kind: SymbolTargetKind) -> &'static str {
    match kind {
        SymbolTargetKind::Constant => "Constant",
        SymbolTargetKind::Function => "Function",
    }
}

/// Validate exact symbol links and group them by relationship/endpoint schema.
pub fn build_symbol_edges(
    links: &[SymbolRelationshipInfo],
    functions: &[FunctionInfo],
    constants: &[ConstantInfo],
) -> Vec<SymbolEdgeBatch> {
    let known_functions: HashSet<&str> = functions
        .iter()
        .map(|function| function.qualified_name.as_str())
        .collect();
    let known_constants: HashSet<&str> = constants
        .iter()
        .map(|constant| constant.qualified_name.as_str())
        .collect();
    let mut grouped: BTreeMap<(SymbolRelationshipKind, SymbolTargetKind), Vec<SymbolEdge>> =
        BTreeMap::new();

    let mut ordered = links.to_vec();
    ordered.sort();
    ordered.dedup();
    for link in ordered {
        if !known_constants.contains(link.source.as_str()) {
            continue;
        }
        let target_exists = match link.target_kind {
            SymbolTargetKind::Constant => known_constants.contains(link.target.as_str()),
            SymbolTargetKind::Function => known_functions.contains(link.target.as_str()),
        };
        if !target_exists {
            continue;
        }
        grouped
            .entry((link.relationship, link.target_kind))
            .or_default()
            .push(SymbolEdge {
                source: link.source,
                target: link.target,
                line: link.line,
                raw_target: link.raw_target,
            });
    }

    grouped
        .into_iter()
        .map(|((relationship, target_kind), edges)| SymbolEdgeBatch {
            relationship: relationship_name(relationship),
            target_node_type: target_node_type(target_kind),
            edges,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function(name: &str) -> FunctionInfo {
        FunctionInfo {
            name: name.rsplit('.').next().unwrap_or(name).to_string(),
            qualified_name: name.to_string(),
            ..Default::default()
        }
    }

    fn constant(name: &str) -> ConstantInfo {
        ConstantInfo {
            name: name.rsplit('.').next().unwrap_or(name).to_string(),
            qualified_name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn control_edges_require_exact_qualified_targets_and_keep_self_jumps() {
        let functions = vec![function("P.A"), function("P.B"), function("Q.B")];
        let sites = vec![
            ControlTransferInfo {
                caller: "P.A".into(),
                target: Some("P.B".into()),
                kind: ControlTransferKind::Call,
                line: 1,
                raw_operand: "B".into(),
                offset: None,
                via: None,
                address_line: None,
            },
            ControlTransferInfo {
                caller: "P.A".into(),
                target: Some("P.A".into()),
                kind: ControlTransferKind::Jump,
                line: 2,
                raw_operand: "A".into(),
                offset: None,
                via: None,
                address_line: None,
            },
            ControlTransferInfo {
                caller: "P.A".into(),
                target: Some("P/MISSING".into()),
                kind: ControlTransferKind::Call,
                line: 3,
                raw_operand: "MISSING".into(),
                offset: None,
                via: None,
                address_line: None,
            },
        ];
        let output = build_control_edges(&sites, &functions);
        assert_eq!(output.calls.len(), 1);
        assert_eq!(output.calls[0].callee, "P.B");
        assert_eq!(output.jumps.len(), 1);
        assert_eq!(output.jumps[0].target, "P.A");
        assert_eq!(output.call_stats.total_calls, 2);
        assert_eq!(output.call_stats.resolved_call_sites, 1);
        assert_eq!(output.call_stats.no_candidate, 1);
    }

    #[test]
    fn reference_sites_aggregate_access_without_cross_namespace_fallback() {
        let functions = vec![function("P.A")];
        let constants = vec![constant("P.X"), constant("Q.X")];
        let sites = vec![
            ReferenceSiteInfo {
                caller: "P.A".into(),
                target: "P.X".into(),
                line: 4,
                opcode: "CA".into(),
                access: ReferenceAccess::Read,
            },
            ReferenceSiteInfo {
                caller: "P.A".into(),
                target: "P.X".into(),
                line: 7,
                opcode: "TS".into(),
                access: ReferenceAccess::Write,
            },
        ];
        let edges = build_reference_site_edges(&sites, &functions, &constants);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].constant, "P.X");
        assert_eq!(edges[0].reference_lines.as_deref(), Some("4,7"));
        assert_eq!(edges[0].accesses.as_deref(), Some("read,write"));
        assert_eq!(edges[0].has_read, Some(true));
        assert_eq!(edges[0].has_write, Some(true));
    }

    #[test]
    fn symbol_links_are_grouped_by_exact_target_type() {
        let functions = vec![function("P.START")];
        let constants = vec![constant("P.ALIAS"), constant("P.VALUE")];
        let links = vec![
            SymbolRelationshipInfo {
                source: "P.ALIAS".into(),
                target: "P.VALUE".into(),
                target_kind: SymbolTargetKind::Constant,
                relationship: SymbolRelationshipKind::AliasOf,
                line: 1,
                raw_target: "VALUE".into(),
            },
            SymbolRelationshipInfo {
                source: "P.VALUE".into(),
                target: "P.START".into(),
                target_kind: SymbolTargetKind::Function,
                relationship: SymbolRelationshipKind::PointsTo,
                line: 2,
                raw_target: "START".into(),
            },
        ];
        let batches = build_symbol_edges(&links, &functions, &constants);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].relationship, "ALIAS_OF");
        assert_eq!(batches[0].target_node_type, "Constant");
        assert_eq!(batches[1].relationship, "POINTS_TO");
        assert_eq!(batches[1].target_node_type, "Function");
    }
}
