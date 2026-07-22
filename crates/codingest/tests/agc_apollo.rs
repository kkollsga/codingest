use codingest::models::{ControlTransferKind, SymbolRelationshipKind};
use codingest::parsers::agc::AgcParser;
use codingest::parsers::LanguageParser;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use walkdir::WalkDir;

#[test]
#[ignore = "requires CODINGEST_APOLLO11_ROOT pinned to the Apollo-11 validation checkout"]
fn apollo_semantic_anchors() {
    let root = PathBuf::from(
        std::env::var("CODINGEST_APOLLO11_ROOT")
            .expect("set CODINGEST_APOLLO11_ROOT to the pinned Apollo-11 checkout"),
    );
    let files: Vec<_> = WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("agc"))
        .collect();
    let parsed = AgcParser::new().parse_files(&files, &root);
    let known: HashSet<_> = parsed
        .functions
        .iter()
        .map(|function| function.qualified_name.as_str())
        .collect();
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    let mut addressed: BTreeMap<&str, usize> = BTreeMap::new();
    let mut resolved: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unresolved = Vec::new();
    for site in &parsed.control_transfers {
        let Some(via) = site.via.as_deref() else {
            continue;
        };
        *totals.entry(via).or_default() += 1;
        if site.address_line.is_some() {
            *addressed.entry(via).or_default() += 1;
        }
        if site
            .target
            .as_deref()
            .is_some_and(|target| known.contains(target))
        {
            *resolved.entry(via).or_default() += 1;
        } else if site.address_line.is_some() {
            unresolved.push((
                site.caller.clone(),
                site.target.clone(),
                via.to_string(),
                site.line,
                site.address_line,
            ));
        }
    }
    eprintln!("totals={totals:?}");
    eprintln!("addressed={addressed:?}");
    eprintln!("resolved={resolved:?}");
    eprintln!(
        "addressed-but-unresolved={} sample={:?}",
        unresolved.len(),
        unresolved.iter().take(5).collect::<Vec<_>>()
    );

    assert_eq!(totals.get("BANKCALL"), Some(&560));
    assert_eq!(totals.get("IBNKCALL"), Some(&103));
    assert_eq!(totals.get("POSTJUMP"), Some(&121));
    assert_eq!(totals.get("BANKJUMP"), Some(&34));
    assert_eq!(totals.get("SWCALL"), Some(&10));
    assert_eq!(resolved.get("BANKCALL"), Some(&520));
    assert_eq!(resolved.get("IBNKCALL"), Some(&102));
    assert_eq!(resolved.get("POSTJUMP"), Some(&115));
    assert_eq!(
        resolved.values().sum::<usize>(),
        737,
        "direct trampoline recovery anchor moved"
    );
    assert!(parsed.control_transfers.iter().all(|site| {
        site.target.as_deref().is_none_or(|target| {
            !["BANKCALL", "IBNKCALL", "POSTJUMP", "BANKJUMP", "SWCALL"]
                .iter()
                .any(|trampoline| target.ends_with(trampoline))
        })
    }));
    assert!(parsed.control_transfers.iter().any(|site| {
        site.via.as_deref() == Some("SWCALL") && site.kind == ControlTransferKind::IndirectCall
    }));
    assert!(parsed.reference_sites.iter().all(|site| {
        site.caller.split_once('.').map(|(program, _)| program)
            == site.target.split_once('.').map(|(program, _)| program)
    }));
    assert_eq!(
        parsed
            .constants
            .iter()
            .filter(|constant| constant.kind == "agc_erase")
            .count(),
        812
    );
    assert_eq!(
        parsed
            .symbol_relationships
            .iter()
            .filter(|link| link.relationship == SymbolRelationshipKind::AliasOf)
            .count(),
        3_280
    );
    assert_eq!(
        parsed
            .symbol_relationships
            .iter()
            .filter(|link| link.relationship == SymbolRelationshipKind::PointsTo)
            .count(),
        204
    );
}
