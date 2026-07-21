//! Hand-written scanner for yaYUL Apollo Guidance Computer assembly.
//!
//! AGC source is column-sensitive: a token beginning in column one is a label,
//! while indented lines begin with an opcode. This scanner intentionally models
//! labels as coarse Function nodes and data pseudo-ops as Constant nodes; it
//! does not attempt to decode the interpretive arithmetic language.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::LanguageParser;
use crate::models::{ConstantInfo, FileInfo, FunctionInfo, ParseResult};
use rayon::prelude::*;

pub const AGC_NOISE_NAMES: &[&str] = &[
    "Q", "A", "L", "Z", "BANKCALL", "POSTJUMP", "ISWCALL", "INTPRET", "PHASCHNG", "TASKOVER",
    "FINDVAC", "NOVAC", "WAITLIST",
];

const DATA_OPS: &[&str] = &[
    "EQUALS", "=", "ERASE", "OCT", "DEC", "2OCT", "2DEC", "ADRES", "CADR", "ECADR", "GENADR", "VN",
    "BBCON",
];

const TRANSFER_OPS: &[&str] = &["TC", "TCF", "CALL", "GOTO"];

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

fn next_line_target(lines: &[&str], current: usize) -> Option<String> {
    lines
        .iter()
        .skip(current + 1)
        .filter_map(|raw| {
            let code = raw.split('#').next().unwrap_or("").trim();
            (!code.is_empty()).then_some(code)
        })
        .find_map(normalize_operand)
}

fn transfer_target(opcode: &str, operand: &str, lines: &[&str], current: usize) -> Option<String> {
    if matches!(opcode, "TC" | "TCF") {
        return normalize_operand(operand);
    }
    if matches!(opcode, "CALL" | "GOTO") {
        return normalize_operand(operand).or_else(|| next_line_target(lines, current));
    }

    let fields: Vec<&str> = operand.split_whitespace().collect();
    let transfer_at = fields
        .iter()
        .position(|field| matches!(upper(field).as_str(), "CALL" | "GOTO"))?;
    fields
        .get(transfer_at + 1)
        .and_then(|target| normalize_operand(target))
        .or_else(|| next_line_target(lines, current))
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
        metadata: Default::default(),
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

        let mut programs_by_label: HashMap<String, HashSet<String>> = HashMap::new();
        for function in &result.functions {
            let program = function
                .qualified_name
                .split_once('.')
                .map(|(program, _)| program)
                .unwrap_or("");
            programs_by_label
                .entry(function.name.clone())
                .or_default()
                .insert(program.to_string());
        }

        for function in &mut result.functions {
            let program = function
                .qualified_name
                .split_once('.')
                .map(|(program, _)| program)
                .unwrap_or("");
            for (target, _) in &mut function.calls {
                if let Some(programs) = programs_by_label.get(target) {
                    if programs.contains(program) && !target.contains('.') {
                        *target = format!("{program}.{target}");
                    } else if !programs.contains(program) {
                        *target = format!("{program}/{target}");
                    }
                }
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

            if line.label.is_some() {
                if let Some(function_index) = current_function.take() {
                    result.functions[function_index].end_line = Some(line.number.saturating_sub(1));
                }
            }

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

            let has_interpretive_transfer = line
                .operand
                .split_whitespace()
                .any(|field| matches!(upper(field).as_str(), "CALL" | "GOTO"));
            if TRANSFER_OPS.contains(&opcode) || has_interpretive_transfer {
                let target = transfer_target(opcode, &line.operand, &lines, index);
                if let Some(target) = target.filter(|target| !is_register(target)) {
                    result.functions[function_index]
                        .calls
                        .push((target, line.number));
                }
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
        assert_eq!(parsed.functions[0].calls, vec![("SECOND".into(), 1)]);
        assert_eq!(parsed.functions[0].references, vec![("VALUE".into(), 2)]);
        assert!(parsed.functions[1].calls.is_empty());
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
        assert_eq!(
            parsed.functions[0].calls,
            vec![
                ("SAME".into(), 1),
                ("OFFSET".into(), 2),
                ("NEXTCALL".into(), 3),
                ("NEXTGOTO".into(), 5),
                ("PAIREDCALL".into(), 7),
                ("PAIREDGOTO".into(), 9),
            ]
        );
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
        fs::write(&caller, "START TC FOREIGN\n\tTC LOCAL\nLOCAL TC Q\n").unwrap();
        fs::write(&foreign, "FOREIGN TC Q\n").unwrap();

        let parsed = AgcParser::new().parse_files(&[caller, foreign], temp.path());
        let start = parsed
            .functions
            .iter()
            .find(|function| function.qualified_name == "Comanche055.START")
            .unwrap();
        assert_eq!(
            start.calls,
            vec![
                ("Comanche055/FOREIGN".into(), 1),
                ("Comanche055.LOCAL".into(), 2),
            ]
        );
        let local = parsed
            .functions
            .iter()
            .find(|function| function.qualified_name == "Comanche055.LOCAL")
            .unwrap();
        assert!(local.calls.is_empty());
    }
}
