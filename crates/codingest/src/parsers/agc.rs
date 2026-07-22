//! Hand-written scanner for yaYUL Apollo Guidance Computer assembly.
//!
//! AGC source is column-sensitive: a token beginning in column one is a label,
//! while indented lines begin with an opcode. This scanner intentionally models
//! labels as coarse Function nodes and data pseudo-ops as Constant nodes; it
//! does not attempt to decode the interpretive arithmetic language.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::LanguageParser;
use crate::models::{
    ConstantInfo, ControlTransferInfo, ControlTransferKind, FileInfo, FunctionInfo, ParseResult,
};
use rayon::prelude::*;

pub const AGC_NOISE_NAMES: &[&str] = &[
    "Q", "A", "L", "Z", "BANKCALL", "POSTJUMP", "ISWCALL", "INTPRET", "PHASCHNG", "TASKOVER",
    "FINDVAC", "NOVAC", "WAITLIST",
];

const DATA_OPS: &[&str] = &[
    "EQUALS", "=", "ERASE", "OCT", "DEC", "2OCT", "2DEC", "ADRES", "CADR", "ECADR", "GENADR", "VN",
    "BBCON",
];

const TRANSFER_OPS: &[&str] = &["TC", "TCF", "CALL", "GOTO", "BZF", "BZMF"];
const DIRECT_TRAMPOLINES: &[&str] = &["BANKCALL", "IBNKCALL", "POSTJUMP"];
const INDIRECT_TRAMPOLINES: &[&str] = &["BANKJUMP", "SWCALL"];

pub struct AgcParser;

impl Default for AgcParser {
    fn default() -> Self {
        Self::new()
    }
}

impl AgcParser {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
struct Line<'a> {
    number: u32,
    label: Option<&'a str>,
    opcode: Option<&'a str>,
    operand: String,
}

fn scan_line(number: u32, raw: &str) -> Option<Line<'_>> {
    let code = raw.split('#').next().unwrap_or("");
    if code.trim().is_empty() {
        return None;
    }
    let indented = code.chars().next().is_some_and(char::is_whitespace);
    let mut tokens = code.split_whitespace();
    if indented {
        let opcode = tokens.next();
        let operand = tokens.collect::<Vec<_>>().join(" ");
        Some(Line {
            number,
            label: None,
            opcode,
            operand,
        })
    } else {
        let label = tokens.next();
        let opcode = tokens.next();
        let operand = tokens.collect::<Vec<_>>().join(" ");
        Some(Line {
            number,
            label,
            opcode,
            operand,
        })
    }
}

fn upper(token: &str) -> String {
    token.trim_end_matches(',').to_ascii_uppercase()
}

fn is_directive(opcode: &str) -> bool {
    matches!(opcode, "BLOCK" | "BANK" | "SETLOC" | "EBANK=") || opcode.starts_with("COUNT")
}

fn constant_kind(opcode: &str) -> String {
    if opcode == "=" {
        "agc_equals_alias".to_string()
    } else {
        format!("agc_{}", opcode.to_ascii_lowercase())
    }
}

fn truncate_100(value: &str) -> String {
    let end = value
        .char_indices()
        .nth(100)
        .map(|(index, _)| index)
        .unwrap_or(value.len());
    value[..end].trim().to_string()
}

fn is_numeric_or_relative(token: &str) -> bool {
    let unsigned = token
        .strip_prefix('+')
        .or_else(|| token.strip_prefix('-'))
        .unwrap_or(token);
    !unsigned.is_empty() && unsigned.chars().all(|character| character.is_ascii_digit())
}

fn normalize_operand(operand: &str) -> Option<String> {
    let mut token = operand
        .split_whitespace()
        .next()?
        .trim_end_matches(',')
        .to_string();
    if token.is_empty() || is_numeric_or_relative(&token) {
        return None;
    }

    if let Some(offset_at) = token
        .char_indices()
        .rev()
        .find(|(index, character)| {
            *index > 0
                && matches!(character, '+' | '-')
                && token[index + character.len_utf8()..]
                    .chars()
                    .all(|digit| digit.is_ascii_digit())
        })
        .map(|(index, _)| index)
    {
        token.truncate(offset_at);
    }
    if token.is_empty() || is_numeric_or_relative(&token) {
        return None;
    }
    Some(token)
}

#[derive(Debug, PartialEq, Eq)]
struct TransferOperand {
    raw: String,
    target: Option<String>,
    offset: Option<String>,
}

fn transfer_operand(operand: &str) -> TransferOperand {
    let raw = operand.trim().to_string();
    let mut fields = raw.split_whitespace();
    let Some(first) = fields.next() else {
        return TransferOperand {
            raw,
            target: None,
            offset: None,
        };
    };
    let token = first.trim_end_matches(',');
    if is_numeric_or_relative(token) {
        let offset = (token.starts_with('+') || token.starts_with('-')).then(|| token.to_string());
        return TransferOperand {
            raw,
            target: None,
            offset,
        };
    }

    let suffix_at = token.char_indices().rev().find_map(|(index, character)| {
        (index > 0
            && matches!(character, '+' | '-')
            && token[index + character.len_utf8()..]
                .chars()
                .all(|digit| digit.is_ascii_digit()))
        .then_some(index)
    });
    let (target, suffix_offset) = if let Some(index) = suffix_at {
        (token[..index].to_string(), Some(token[index..].to_string()))
    } else {
        (token.to_string(), None)
    };
    let separate_offset = fields
        .next()
        .filter(|field| {
            (field.starts_with('+') || field.starts_with('-')) && is_numeric_or_relative(field)
        })
        .map(str::to_string);
    TransferOperand {
        raw,
        target: (!target.is_empty()).then_some(target),
        offset: suffix_offset.or(separate_offset),
    }
}

fn is_register(target: &str) -> bool {
    matches!(target.to_ascii_uppercase().as_str(), "Q" | "A" | "L" | "Z")
}

fn module_path(filepath: &Path, src_root: &Path) -> String {
    let rel = filepath.strip_prefix(src_root).unwrap_or(filepath);
    let mut parts: Vec<String> = rel
        .components()
        .filter_map(|component| component.as_os_str().to_str().map(str::to_string))
        .collect();
    if let Some(last) = parts.last_mut() {
        if let Some(stem) = last.strip_suffix(".agc") {
            *last = stem.to_string();
        }
    }
    parts.join("/")
}

fn program_name(filepath: &Path, src_root: &Path) -> String {
    let rel = filepath.strip_prefix(src_root).unwrap_or(filepath);
    let mut components = rel
        .components()
        .filter_map(|component| component.as_os_str().to_str());
    let first = components.next().unwrap_or("");
    if components.next().is_some() {
        first.to_string()
    } else {
        filepath
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_string()
    }
}

fn include_target(trimmed: &str, program: &str) -> Option<String> {
    let include = trimmed.strip_prefix('$')?.split_whitespace().next()?;
    let name = include
        .strip_suffix(".agc")
        .or_else(|| include.strip_suffix(".AGC"))
        .unwrap_or(include)
        .replace('\\', "/");
    (!name.is_empty()).then(|| format!("{program}/{name}"))
}

fn next_line_operand(lines: &[&str], current: usize) -> Option<(TransferOperand, u32)> {
    lines
        .iter()
        .enumerate()
        .skip(current + 1)
        .find_map(|(index, raw)| {
            let code = raw.split('#').next().unwrap_or("").trim();
            (!code.is_empty()).then(|| (transfer_operand(code), index as u32 + 1))
        })
}

fn next_address_operand(lines: &[&str], current: usize) -> Option<(TransferOperand, u32)> {
    lines
        .iter()
        .enumerate()
        .skip(current + 1)
        .find_map(|(index, raw)| scan_line(index as u32 + 1, raw))
        .and_then(|line| {
            let opcode = line.opcode.map(upper)?;
            matches!(opcode.as_str(), "CADR" | "FCADR")
                .then(|| (transfer_operand(&line.operand), line.number))
        })
}

fn transfer_site(
    caller: &str,
    program: &str,
    opcode: &str,
    operand: &str,
    lines: &[&str],
    current: usize,
    line: u32,
) -> Option<ControlTransferInfo> {
    if matches!(opcode, "TC" | "TCF") {
        let trampoline = transfer_operand(operand)
            .target
            .map(|target| target.to_ascii_uppercase());
        if let Some(trampoline) = trampoline {
            if DIRECT_TRAMPOLINES.contains(&trampoline.as_str()) {
                let (kind, address) = if trampoline == "POSTJUMP" {
                    (
                        ControlTransferKind::Jump,
                        next_address_operand(lines, current),
                    )
                } else {
                    (
                        ControlTransferKind::Call,
                        next_address_operand(lines, current),
                    )
                };
                let (parsed, address_line) = address.unwrap_or_else(|| {
                    (
                        TransferOperand {
                            raw: operand.trim().to_string(),
                            target: None,
                            offset: None,
                        },
                        0,
                    )
                });
                return Some(ControlTransferInfo {
                    caller: caller.to_string(),
                    target: parsed.target.map(|target| format!("{program}.{target}")),
                    kind,
                    line,
                    raw_operand: parsed.raw,
                    offset: parsed.offset,
                    via: Some(trampoline),
                    address_line: (address_line != 0).then_some(address_line),
                });
            }
            if INDIRECT_TRAMPOLINES.contains(&trampoline.as_str()) {
                let kind = if trampoline == "SWCALL" {
                    ControlTransferKind::IndirectCall
                } else {
                    ControlTransferKind::IndirectJump
                };
                return Some(ControlTransferInfo {
                    caller: caller.to_string(),
                    target: None,
                    kind,
                    line,
                    raw_operand: "A".to_string(),
                    offset: None,
                    via: Some(trampoline),
                    address_line: None,
                });
            }
        }
    }

    let fields: Vec<&str> = operand.split_whitespace().collect();
    let embedded = fields
        .iter()
        .position(|field| matches!(upper(field).as_str(), "CALL" | "GOTO"));
    let transfer_opcode = embedded
        .map(|index| upper(fields[index]))
        .unwrap_or_else(|| opcode.to_string());
    if !TRANSFER_OPS.contains(&transfer_opcode.as_str()) {
        return None;
    }

    let inline = if let Some(index) = embedded {
        fields.get(index + 1..).unwrap_or_default().join(" ")
    } else {
        operand.trim().to_string()
    };
    let (parsed, address_line) =
        if inline.is_empty() && matches!(transfer_opcode.as_str(), "CALL" | "GOTO") {
            next_line_operand(lines, current)?
        } else {
            (transfer_operand(&inline), 0)
        };

    let mut kind = match transfer_opcode.as_str() {
        "TC" | "CALL" => ControlTransferKind::Call,
        "TCF" | "GOTO" => ControlTransferKind::Jump,
        "BZF" | "BZMF" => ControlTransferKind::Branch,
        _ => return None,
    };
    let target = parsed.target.and_then(|target| {
        if is_register(&target) {
            kind = if transfer_opcode == "TC" && target.eq_ignore_ascii_case("Q") {
                ControlTransferKind::IndirectJump
            } else if kind == ControlTransferKind::Call {
                ControlTransferKind::IndirectCall
            } else {
                ControlTransferKind::IndirectJump
            };
            None
        } else {
            Some(format!("{program}.{target}"))
        }
    });

    Some(ControlTransferInfo {
        caller: caller.to_string(),
        target,
        kind,
        line,
        raw_operand: parsed.raw,
        offset: parsed.offset,
        via: None,
        address_line: (address_line != 0).then_some(address_line),
    })
}

fn emit_function(
    result: &mut ParseResult,
    label: &str,
    program: &str,
    rel_path: &str,
    line: u32,
) -> usize {
    result.functions.push(FunctionInfo {
        name: label.to_string(),
        qualified_name: format!("{program}.{label}"),
        visibility: "public".to_string(),
        is_async: false,
        is_method: false,
        signature: format!("{label} (agc)"),
        file_path: rel_path.to_string(),
        line_number: line,
        docstring: None,
        return_type: None,
        decorators: Vec::new(),
        calls: Vec::new(),
        references: Vec::new(),
        function_refs: Vec::new(),
        type_parameters: None,
        end_line: None,
        parameters: Vec::new(),
        branch_count: None,
        param_count: Some(0),
        max_nesting: None,
        is_recursive: None,
        procedure_names: Vec::new(),
        metadata: HashMap::from([(
            "symbol_kind".to_string(),
            serde_json::Value::String("agc_label".to_string()),
        )]),
    });
    result.functions.len() - 1
}

impl LanguageParser for AgcParser {
    fn language_name(&self) -> &'static str {
        "agc"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["agc"]
    }

    fn parse_files(&self, files: &[PathBuf], src_root: &Path) -> ParseResult {
        let mut result = files
            .par_iter()
            .map(|filepath| self.parse_file(filepath, src_root))
            .reduce(ParseResult::new, |mut accumulated, parsed| {
                accumulated.merge(parsed);
                accumulated
            });

        let known_functions: HashSet<&str> = result
            .functions
            .iter()
            .map(|function| function.qualified_name.as_str())
            .collect();
        let called_targets: HashSet<&str> = result
            .control_transfers
            .iter()
            .filter(|site| site.kind == ControlTransferKind::Call)
            .filter_map(|site| site.target.as_deref())
            .filter(|target| known_functions.contains(target))
            .collect();
        for function in &mut result.functions {
            if called_targets.contains(function.qualified_name.as_str()) {
                function.metadata.insert(
                    "role_hint".to_string(),
                    serde_json::Value::String("routine".to_string()),
                );
            }
        }
        result
    }

    fn parse_file(&self, filepath: &Path, src_root: &Path) -> ParseResult {
        let Ok(source) = std::fs::read_to_string(filepath) else {
            return ParseResult::new();
        };
        let rel_path = filepath
            .strip_prefix(src_root)
            .unwrap_or(filepath)
            .to_string_lossy()
            .replace('\\', "/");
        let filename = filepath
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let module_path = module_path(filepath, src_root);
        let program = program_name(filepath, src_root);
        let lines: Vec<&str> = source.lines().collect();
        let loc = lines.len() as u32;
        let mut result = ParseResult::new();
        let mut file_info = FileInfo {
            path: rel_path.clone(),
            filename,
            loc,
            module_path,
            language: "agc".to_string(),
            submodule_declarations: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            annotations: None,
            is_test: false,
            skip_reason: None,
        };
        let mut current_function: Option<usize> = None;

        for (index, raw) in lines.iter().enumerate() {
            let code = raw.split('#').next().unwrap_or("");
            if let Some(target) = include_target(code.trim(), &program) {
                file_info.imports.push(target);
                continue;
            }
            let Some(line) = scan_line(index as u32 + 1, raw) else {
                continue;
            };
            let opcode = line.opcode.map(upper);

            if let (Some(label), Some(opcode)) = (line.label, opcode.as_deref()) {
                if DATA_OPS.contains(&opcode) {
                    result.constants.push(ConstantInfo {
                        name: label.to_string(),
                        qualified_name: format!("{program}.{label}"),
                        kind: constant_kind(opcode),
                        type_annotation: None,
                        value_preview: Some(truncate_100(&line.operand)),
                        visibility: "public".to_string(),
                        file_path: rel_path.clone(),
                        line_number: line.number,
                    });
                    continue;
                }
                if is_directive(opcode) {
                    continue;
                }
                if let Some(function_index) = current_function.take() {
                    result.functions[function_index].end_line = Some(line.number.saturating_sub(1));
                }
                current_function = Some(emit_function(
                    &mut result,
                    label,
                    &program,
                    &rel_path,
                    line.number,
                ));
            }

            let Some(opcode) = opcode.as_deref() else {
                continue;
            };
            if is_directive(opcode) || DATA_OPS.contains(&opcode) {
                continue;
            }
            let Some(function_index) = current_function else {
                continue;
            };

            let caller = result.functions[function_index].qualified_name.clone();
            if let Some(site) = transfer_site(
                &caller,
                &program,
                opcode,
                &line.operand,
                &lines,
                index,
                line.number,
            ) {
                result.control_transfers.push(site);
            } else if let Some(reference) = normalize_operand(&line.operand) {
                result.functions[function_index]
                    .references
                    .push((reference, line.number));
            }
        }

        if let Some(function_index) = current_function {
            result.functions[function_index].end_line = Some(loc);
        }
        result.files.push(file_info);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite::api::GraphRead;
    use kglite::datatypes::Value;
    use std::fs;

    fn parse(source: &str) -> ParseResult {
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("Comanche055");
        fs::create_dir_all(&program).unwrap();
        let file = program.join("MAIN.agc");
        fs::write(&file, source).unwrap();
        AgcParser::new().parse_file(&file, temp.path())
    }

    #[test]
    fn labels_form_program_scoped_functions_with_fallthrough_spans() {
        let parsed = parse("FIRST\tTC SECOND\n\tCA VALUE\nSECOND\tTC Q\n");
        assert_eq!(parsed.files[0].module_path, "Comanche055/MAIN");
        assert_eq!(parsed.functions.len(), 2);
        assert_eq!(parsed.functions[0].qualified_name, "Comanche055.FIRST");
        assert_eq!(parsed.functions[0].end_line, Some(2));
        assert_eq!(parsed.functions[1].end_line, Some(3));
        assert!(parsed.functions[0].calls.is_empty());
        assert_eq!(parsed.control_transfers.len(), 2);
        assert_eq!(parsed.control_transfers[0].caller, "Comanche055.FIRST");
        assert_eq!(
            parsed.control_transfers[0].target.as_deref(),
            Some("Comanche055.SECOND")
        );
        assert_eq!(parsed.control_transfers[0].kind, ControlTransferKind::Call);
        assert_eq!(
            parsed.control_transfers[1].kind,
            ControlTransferKind::IndirectJump
        );
        assert_eq!(parsed.functions[0].references, vec![("VALUE".into(), 2)]);
    }

    #[test]
    fn every_data_pseudo_op_emits_a_constant() {
        let source = "A EQUALS\nB = 1\nC ERASE 2\nD OCT 7\nE DEC 8\nF 2OCT 10\nG 2DEC 11\nH ADRES A\nI CADR A\nJ ECADR A\nK GENADR A\nL VN 1\nM BBCON A\n";
        let parsed = parse(source);
        assert_eq!(parsed.constants.len(), DATA_OPS.len());
        assert_eq!(parsed.constants[0].kind, "agc_equals");
        assert_eq!(parsed.constants[0].value_preview.as_deref(), Some(""));
        assert_eq!(parsed.constants[1].kind, "agc_equals_alias");
        assert_eq!(parsed.constants[12].kind, "agc_bbcon");
    }

    #[test]
    fn transfer_targets_support_lookahead_offsets_and_register_exclusion() {
        let parsed = parse(
            "START\tTC SAME\n\tTCF OFFSET +2\n\tCALL\n\tNEXTCALL\n\tGOTO\n\tNEXTGOTO\n\tDLOAD CALL\n\tPAIREDCALL\n\tBZE GOTO\n\tPAIREDGOTO\n\tTC Q\nSAME\tTC Q\nOFFSET\tTC Q\nNEXTCALL\tTC Q\nNEXTGOTO\tTC Q\nPAIREDCALL\tTC Q\nPAIREDGOTO\tTC Q\n",
        );
        let sites = &parsed.control_transfers[..6];
        assert_eq!(sites[0].target.as_deref(), Some("Comanche055.SAME"));
        assert_eq!(sites[0].kind, ControlTransferKind::Call);
        assert_eq!(sites[1].target.as_deref(), Some("Comanche055.OFFSET"));
        assert_eq!(sites[1].offset.as_deref(), Some("+2"));
        assert_eq!(sites[1].kind, ControlTransferKind::Jump);
        assert_eq!(sites[2].target.as_deref(), Some("Comanche055.NEXTCALL"));
        assert_eq!(sites[2].address_line, Some(4));
        assert_eq!(sites[3].target.as_deref(), Some("Comanche055.NEXTGOTO"));
        assert_eq!(sites[3].kind, ControlTransferKind::Jump);
        assert_eq!(sites[4].target.as_deref(), Some("Comanche055.PAIREDCALL"));
        assert_eq!(sites[5].target.as_deref(), Some("Comanche055.PAIREDGOTO"));
        assert_eq!(sites[5].kind, ControlTransferKind::Jump);
    }

    #[test]
    fn include_uses_program_prefixed_module_shape() {
        let parsed = parse("$SUB.agc\nSTART\tTC Q\n");
        assert_eq!(parsed.files[0].imports, vec!["Comanche055/SUB"]);
    }

    #[test]
    fn directives_and_prelabel_instructions_emit_no_entities() {
        let parsed = parse("\tTC LOST\n\tBANK 2\nPAGE BLOCK 1\nSTART\tTC Q\n");
        assert_eq!(parsed.functions.len(), 1);
        assert_eq!(parsed.functions[0].name, "START");
        assert!(parsed.functions[0].calls.is_empty());
        assert_eq!(parsed.control_transfers.len(), 1);
        assert_eq!(
            parsed.control_transfers[0].kind,
            ControlTransferKind::IndirectJump
        );
        assert!(parsed.constants.is_empty());
    }

    #[test]
    fn parse_files_blocks_unique_cross_program_targets() {
        let temp = tempfile::tempdir().unwrap();
        let comanche = temp.path().join("Comanche055");
        let luminary = temp.path().join("Luminary099");
        fs::create_dir_all(&comanche).unwrap();
        fs::create_dir_all(&luminary).unwrap();
        let caller = comanche.join("MAIN.agc");
        let foreign = luminary.join("MAIN.agc");
        fs::write(
            &caller,
            "START TC FOREIGN\n\tTC LOCAL\n\tTC FOREIGN.1\n\tTC LOCAL.1\nLOCAL TC Q\nLOCAL.1 TC Q\n",
        )
        .unwrap();
        fs::write(&foreign, "FOREIGN TC Q\nFOREIGN.1 TC Q\n").unwrap();

        let parsed = AgcParser::new().parse_files(&[caller, foreign], temp.path());
        let start_sites: Vec<_> = parsed
            .control_transfers
            .iter()
            .filter(|site| site.caller == "Comanche055.START")
            .collect();
        assert_eq!(
            start_sites
                .iter()
                .map(|site| site.target.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("Comanche055.FOREIGN"),
                Some("Comanche055.LOCAL"),
                Some("Comanche055.FOREIGN.1"),
                Some("Comanche055.LOCAL.1"),
            ]
        );
        let local = parsed
            .functions
            .iter()
            .find(|function| function.qualified_name == "Comanche055.LOCAL")
            .unwrap();
        assert!(local.calls.is_empty());
        assert_eq!(local.metadata["role_hint"], "routine");
    }

    #[test]
    fn branches_relative_transfers_and_ccs_are_modeled_honestly() {
        let parsed = parse(
            "START\tBZF ZERO\n\tBZMF NEGATIVE-1\n\tTCF +2\n\tTC A\n\tCCS COUNTER\nZERO\tTC Q\nNEGATIVE\tTC Q\n",
        );
        assert_eq!(parsed.control_transfers.len(), 6);
        assert_eq!(
            parsed.control_transfers[0].kind,
            ControlTransferKind::Branch
        );
        assert_eq!(
            parsed.control_transfers[0].target.as_deref(),
            Some("Comanche055.ZERO")
        );
        assert_eq!(parsed.control_transfers[1].offset.as_deref(), Some("-1"));
        assert_eq!(parsed.control_transfers[2].target, None);
        assert_eq!(parsed.control_transfers[2].offset.as_deref(), Some("+2"));
        assert_eq!(
            parsed.control_transfers[3].kind,
            ControlTransferKind::IndirectCall
        );
        assert!(!parsed
            .control_transfers
            .iter()
            .any(|site| site.raw_operand == "COUNTER"));
        assert!(parsed.functions[0]
            .references
            .contains(&("COUNTER".to_string(), 5)));
    }

    #[test]
    fn transfer_operand_preserves_raw_target_and_offsets() {
        assert_eq!(
            transfer_operand("TARGET +2"),
            TransferOperand {
                raw: "TARGET +2".into(),
                target: Some("TARGET".into()),
                offset: Some("+2".into()),
            }
        );
        assert_eq!(
            transfer_operand("TARGET-3").target.as_deref(),
            Some("TARGET")
        );
        assert_eq!(transfer_operand("TARGET-3").offset.as_deref(), Some("-3"));
        assert_eq!(transfer_operand("+4").target, None);
    }

    #[test]
    fn loaded_graph_separates_calls_jumps_and_branches() {
        let parsed = parse(
            "START\tTC ROUTINE\n\tTCF EXIT\n\tBZF ZERO\nROUTINE\tTC Q\nEXIT\tTC Q\nZERO\tTC Q\n",
        );
        let (graph, _) = crate::builder::load::load_into_graph(&parsed, None).unwrap();
        let mut relationships = graph
            .graph
            .edge_indices()
            .filter_map(|edge_index| {
                let edge = graph.graph.edge_weight(edge_index)?;
                let (source, target) = graph.graph.edge_endpoints(edge_index)?;
                let source = graph.graph.node_weight(source)?;
                let target = graph.graph.node_weight(target)?;
                let source_id = match source.id().as_ref() {
                    Value::String(value) => value.clone(),
                    _ => return None,
                };
                let target_id = match target.id().as_ref() {
                    Value::String(value) => value.clone(),
                    _ => return None,
                };
                matches!(
                    edge.connection_type_str(&graph.interner),
                    "CALLS" | "JUMPS_TO" | "BRANCHES_TO"
                )
                .then(|| {
                    (
                        edge.connection_type_str(&graph.interner).to_string(),
                        source_id,
                        target_id,
                    )
                })
            })
            .collect::<Vec<_>>();
        relationships.sort();
        assert_eq!(
            relationships,
            vec![
                (
                    "BRANCHES_TO".into(),
                    "Comanche055.START".into(),
                    "Comanche055.ZERO".into(),
                ),
                (
                    "CALLS".into(),
                    "Comanche055.START".into(),
                    "Comanche055.ROUTINE".into(),
                ),
                (
                    "JUMPS_TO".into(),
                    "Comanche055.START".into(),
                    "Comanche055.EXIT".into(),
                ),
            ]
        );
    }

    #[test]
    fn inter_bank_trampolines_resolve_only_direct_address_forms() {
        let parsed = parse(
            "START\tTC BANKCALL\n\tCADR CALLEE\n\tTC IBNKCALL\n\tFCADR CALLEE\n\tTC POSTJUMP\n\tCADR EXIT\n\tTC BANKJUMP\n\tCA CALLEE\n\tTC SWCALL\nCALLEE\tTC Q\nEXIT\tTC Q\n",
        );
        let sites: Vec<_> = parsed
            .control_transfers
            .iter()
            .filter(|site| site.caller == "Comanche055.START")
            .collect();
        assert_eq!(sites.len(), 5);
        assert_eq!(sites[0].target.as_deref(), Some("Comanche055.CALLEE"));
        assert_eq!(sites[0].via.as_deref(), Some("BANKCALL"));
        assert_eq!(sites[0].address_line, Some(2));
        assert_eq!(sites[1].target.as_deref(), Some("Comanche055.CALLEE"));
        assert_eq!(sites[1].via.as_deref(), Some("IBNKCALL"));
        assert_eq!(sites[1].address_line, Some(4));
        assert_eq!(sites[2].target.as_deref(), Some("Comanche055.EXIT"));
        assert_eq!(sites[2].kind, ControlTransferKind::Jump);
        assert_eq!(sites[2].via.as_deref(), Some("POSTJUMP"));
        assert_eq!(sites[3].target, None);
        assert_eq!(sites[3].kind, ControlTransferKind::IndirectJump);
        assert_eq!(sites[3].via.as_deref(), Some("BANKJUMP"));
        assert_eq!(sites[3].address_line, None);
        assert_eq!(sites[4].target, None);
        assert_eq!(sites[4].kind, ControlTransferKind::IndirectCall);
        assert_eq!(sites[4].via.as_deref(), Some("SWCALL"));
        assert!(!parsed.control_transfers.iter().any(|site| {
            site.target.as_deref().is_some_and(|target| {
                ["BANKCALL", "IBNKCALL", "POSTJUMP", "BANKJUMP", "SWCALL"]
                    .iter()
                    .any(|name| target.ends_with(name))
            })
        }));
    }

    #[test]
    fn direct_trampoline_does_not_search_past_the_next_source_line() {
        let parsed = parse("START\tTC BANKCALL\n\tCA VALUE\n\tCADR CALLEE\nCALLEE\tTC Q\n");
        let site = &parsed.control_transfers[0];
        assert_eq!(site.via.as_deref(), Some("BANKCALL"));
        assert_eq!(site.target, None);
        assert_eq!(site.address_line, None);
    }
}
