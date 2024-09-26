use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tracing::info;
use crate::ast::ast_structs::AstDefinition;
use crate::call_validation::DiffChunk;
use crate::global_context::GlobalContext;
use crate::tools::patch::chat_interaction::read_file;
use crate::tools::patch::patch_utils::most_common_value_in_vec;
use crate::tools::patch::tickets::{PatchLocateAs, TicketToApply};
use crate::tools::patch::unified_diff_format::{diff_blocks_to_diff_chunks, DiffBlock, DiffLine, LineType};


pub async fn full_rewrite_diff(
    gcx: Arc<ARwLock<GlobalContext>>,
    ticket: &TicketToApply,
) -> Result<Vec<DiffChunk>, String> {
    let context_file = read_file(gcx.clone(), ticket.filename_before.clone()).await
        .map_err(|e|format!("cannot read file to modify: {}.\nError: {e}", ticket.filename_before))?;
    let file_path = PathBuf::from(&context_file.file_name);

    let diffs = diff::lines(&context_file.file_content, &ticket.code);
    chunks_from_diffs(file_path, diffs)
}

fn minimal_common_indent(symbol_lines: &[&str]) -> (usize, usize) {
    let mut common_spaces = vec![];
    let mut common_tabs = vec![];
    for line in symbol_lines.iter().filter(|l|!l.is_empty()) {
        let spaces = line.chars().take_while(|c| *c == ' ').count();
        common_spaces.push(spaces);
        let tabs = line.chars().take_while(|c| *c == '\t').count();
        common_tabs.push(tabs);
    }
    (
        common_spaces.iter().min().cloned().unwrap_or(0), 
        common_tabs.iter().min().cloned().unwrap_or(0)
    )
}

fn place_indent(code_lines: &[&str], indent_spaces: usize, indent_tabs: usize) -> Vec<String> {
    info!("CODE:\n{}\n", code_lines.join("\n"));
    let (min_spaces, min_tabs) = minimal_common_indent(code_lines);
    info!("MIN INDENT: {:?} and {:?}", min_spaces, min_tabs);

    code_lines.iter().map(|line| {
        let trimmed_line = line
            .chars()
            .skip(min_spaces + min_tabs)
            .collect::<String>();

        let new_indent = if line.is_empty() {"".to_string()} else {" ".repeat(indent_spaces) + &"\t".repeat(indent_tabs)};
        format!("{}{}", new_indent, trimmed_line)
    }).collect()
}

fn same_parent_symbols(ticket: &TicketToApply, locate_symbol: &Arc<AstDefinition>) -> Vec<Arc<AstDefinition>> {
    fn symbol_parent_elements(symbol: &Arc<AstDefinition>) -> Vec<String> {
        let mut elements = symbol.official_path.clone();
        elements.pop();
        elements
    }
    let mut grouped_symbols = HashMap::new();
    for symbol in &ticket.all_symbols {
        grouped_symbols.entry(symbol_parent_elements(symbol)).or_insert_with(Vec::new).push(symbol.clone());
    }
    let mut same_parents_syms = grouped_symbols.get(&symbol_parent_elements(locate_symbol)).cloned().unwrap_or(Vec::new());
    if same_parents_syms.len() > 1 {
        same_parents_syms.sort_by_key(|s| s.full_range.start_point.row);
    }
    same_parents_syms
}

fn most_common_spacing(same_parent_symbols: &Vec<Arc<AstDefinition>>) -> usize {
    return if same_parent_symbols.len() > 1 {
        let spacings: Vec<isize> = same_parent_symbols.windows(2)
            .map(|pair| { 
                // info!("pair names: {:?} AND {:?}", pair[1].official_path, pair[0].official_path);
                // info!("diff: {}", pair[1].full_range.start_point.row as isize - pair[0].full_range.end_point.row as isize);
                (pair[1].full_range.start_point.row as isize - pair[0].full_range.end_point.row as isize).saturating_sub(1)
            })
            .collect();
        most_common_value_in_vec(spacings).unwrap_or(1) as usize
    } else {
        1
    }
}

pub async fn add_to_file_diff(
    gcx: Arc<ARwLock<GlobalContext>>,
    ticket: &TicketToApply,
) -> Result<Vec<DiffChunk>, String> {
    let context_file = read_file(gcx.clone(), ticket.filename_before.clone()).await
        .map_err(|e|format!("cannot read file to modify: {}.\nError: {e}", ticket.filename_before))?;
    let context_file_path = PathBuf::from(&context_file.file_name);

    let symbol = ticket.locate_symbol.clone().expect("symbol not found");
    let file_text = context_file.file_content.clone();
    let line_ending = if file_text.contains("\r\n") { "\r\n" } else { "\n" };
    let file_lines = file_text.split(line_ending).collect::<Vec<&str>>();
    let symbol_lines = file_lines[symbol.full_range.start_point.row..symbol.full_range.end_point.row].to_vec();
    let (indent_spaces, indent_tabs) = minimal_common_indent(&symbol_lines);

    let locate_as = ticket.locate_as.clone().expect("locate_as not found");
    let same_parent_symbols = same_parent_symbols(ticket, &symbol);
    let pos_locate_symbol = same_parent_symbols.iter().position(|s| s.official_path == symbol.official_path).expect("symbol not found");
    
    let ticket_code = ticket.code.clone();
    let ticket_line_ending = if ticket_code.contains("\r\n") { "\r\n" } else { "\n" };
    if line_ending != ticket_line_ending {
        return Err(format!("line endings do not match: {line_ending} != {ticket_line_ending}; {line_ending} is expected"));
    }
    
    let ticket_code_lines = ticket_code.split(ticket_line_ending).collect::<Vec<&str>>();
    let ticket_code_lines = place_indent(&ticket_code_lines, indent_spaces, indent_tabs);
    let file_lines = file_lines.into_iter().map(|s| s.to_string()).collect::<Vec<_>>();
    
    let spacing = most_common_spacing(&same_parent_symbols);
    
    let new_code_lines = if locate_as == PatchLocateAs::BEFORE {
        let sym_before = if pos_locate_symbol == 0 { None } else { Some(same_parent_symbols[pos_locate_symbol - 1].clone()) };
        let sym_after = symbol;
        if let Some(sym_before) = sym_before {
            file_lines[..sym_before.full_range.end_point.row + 1].iter()
                .chain(vec!["".to_string(); spacing].iter())
                .chain(ticket_code_lines.iter())
                .chain(vec!["".to_string(); spacing].iter())
                .chain(file_lines[sym_after.full_range.start_point.row..].iter())
                .cloned().collect::<Vec<_>>()
        } else {
            file_lines[..sym_after.full_range.start_point.row].iter()
                .chain(ticket_code_lines.iter())
                .chain(vec!["".to_string(); spacing].iter())
                .chain(file_lines[sym_after.full_range.start_point.row..].iter())
               .cloned().collect::<Vec<_>>()
        }
        
    } else {
        let sym_before = symbol;
        let sym_after = same_parent_symbols.get(pos_locate_symbol + 1).cloned();
        if let Some(sym_after) = sym_after {
            file_lines[..sym_before.full_range.end_point.row + 1].iter()
                .chain(vec!["".to_string(); spacing].iter())
                .chain(ticket_code_lines.iter())
                .chain(vec!["".to_string(); spacing].iter())
                .chain(file_lines[sym_after.full_range.start_point.row..].iter())
                .cloned().collect::<Vec<_>>()
        } else {
            file_lines[..sym_before.full_range.end_point.row + 1].iter()
                .chain(vec!["".to_string(); spacing].iter())
                .chain(ticket_code_lines.iter())
                .chain(file_lines[sym_before.full_range.end_point.row + 1..].iter())
                .cloned().collect::<Vec<_>>()
        }
    };
    let new_code = new_code_lines.join(ticket_line_ending);
    
    let diffs = diff::lines(&context_file.file_content, &new_code);

    chunks_from_diffs(context_file_path, diffs)
}

pub async fn rewrite_symbol_diff(
    gcx: Arc<ARwLock<GlobalContext>>,
    ticket: &TicketToApply,
) -> Result<Vec<DiffChunk>, String> {
    let context_file = read_file(gcx.clone(), ticket.filename_before.clone()).await
        .map_err(|e|format!("cannot read file to modify: {}.\nError: {e}", ticket.filename_before))?;
    let context_file_path = PathBuf::from(&context_file.file_name);
    let symbol = ticket.locate_symbol.clone().expect("symbol not found");
    
    let file_text = context_file.file_content.clone();
    let line_ending = if file_text.contains("\r\n") { "\r\n" } else { "\n" };
    let file_lines = file_text.split(line_ending).collect::<Vec<&str>>();
    let symbol_lines = file_lines[symbol.full_range.start_point.row..symbol.full_range.end_point.row].to_vec();
    let (indent_spaces, indent_tabs) = minimal_common_indent(&symbol_lines);
    
    let ticket_code = ticket.code.clone();
    let ticket_line_ending = if ticket_code.contains("\r\n") { "\r\n" } else { "\n" };
    if line_ending != ticket_line_ending {
        return Err(format!("line endings do not match: {line_ending} != {ticket_line_ending}; {line_ending} is expected"));
    }
    let ticket_code_lines = ticket_code.split(ticket_line_ending).collect::<Vec<&str>>();
    let ticket_code_lines = place_indent(&ticket_code_lines, indent_spaces, indent_tabs);

    let new_code_lines = file_lines[..symbol.full_range.start_point.row].iter()
        .map(|s| s.to_string())
        .chain(ticket_code_lines.iter().cloned())
        .chain(file_lines[symbol.full_range.end_point.row + 1..].iter().map(|s| s.to_string()))
        .collect::<Vec<_>>();
    let new_code = new_code_lines.join(ticket_line_ending);

    let diffs = diff::lines(&context_file.file_content, &new_code);

    chunks_from_diffs(context_file_path, diffs)
}

pub fn new_file_diff(
    ticket: &TicketToApply,
) -> Vec<DiffChunk> {
    vec![
        DiffChunk {
            file_name: ticket.filename_before.clone(),
            file_name_rename: None,
            file_action: "add".to_string(),
            line1: 1,
            line2: 1,
            lines_remove: "".to_string(),
            lines_add: ticket.code.clone(),
            ..Default::default()
        }
    ]
}

fn chunks_from_diffs(file_path: PathBuf, diffs: Vec<diff::Result<&str>>) -> Result<Vec<DiffChunk>, String> {
    let mut line_num: usize = 0;
    let mut blocks = vec![];
    let mut diff_lines = vec![];
    for diff in diffs {
        match diff {
            diff::Result::Left(l) => {
                diff_lines.push(DiffLine {
                    line: l.to_string(),
                    line_type: LineType::Minus,
                    file_line_num_idx: Some(line_num),
                    correct_spaces_offset: Some(0),
                });
                line_num += 1;
            }
            diff::Result::Right(r) => {
                diff_lines.push(DiffLine {
                    line: r.to_string(),
                    line_type: LineType::Plus,
                    file_line_num_idx: Some(line_num),
                    correct_spaces_offset: Some(0),
                });
            }
            diff::Result::Both(_, _) => {
                line_num += 1;
                if !diff_lines.is_empty() {
                    blocks.push(DiffBlock {
                        file_name_before: file_path.clone(),
                        file_name_after: file_path.clone(),
                        action: "edit".to_string(),
                        file_lines: Arc::new(vec![]),
                        hunk_idx: 0,
                        diff_lines: diff_lines.clone(),
                    });
                    diff_lines.clear();
                }
            }
        }
    }
    if !diff_lines.is_empty() {
        blocks.push(DiffBlock {
            file_name_before: file_path.clone(),
            file_name_after: file_path.clone(),
            action: "edit".to_string(),
            file_lines: Arc::new(vec![]),
            hunk_idx: 0,
            diff_lines: diff_lines.clone(),
        });
        diff_lines.clear();
    }

    Ok(diff_blocks_to_diff_chunks(&blocks))
}
