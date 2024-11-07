use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::warn;

use crate::ast::ast_structs::AstDefinition;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::tools::tool_patch_aux::postprocessing_utils::does_doc_have_symbol;

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PatchAction {
    RewriteSymbol,
    #[default]
    PartialEdit,
    RewriteWholeFile,
    Other,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub enum PatchLocateAs {
    BEFORE,
    AFTER,
    SYMBOLNAME,
}

impl PatchLocateAs {
    pub fn from_string(s: &str) -> Result<PatchLocateAs, String> {
        match s {
            "BEFORE" => Ok(PatchLocateAs::BEFORE),
            "AFTER" => Ok(PatchLocateAs::AFTER),
            "SYMBOL_NAME" => Ok(PatchLocateAs::SYMBOLNAME),
            _ => Err(format!("invalid locate_as: {}", s)),
        }
    }
}

impl PatchAction {
    pub fn from_string(action: &str) -> Result<PatchAction, String> {
        match action {
            "📍REWRITE_ONE_SYMBOL" => Ok(PatchAction::RewriteSymbol),
            "📍REWRITE_WHOLE_FILE" => Ok(PatchAction::RewriteWholeFile),
            "📍PARTIAL_EDIT" => Ok(PatchAction::PartialEdit),
            "📍OTHER" => Ok(PatchAction::Other),
            _ => Err(format!("invalid action: {}", action)),
        }
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct TicketToApply {
    pub action: PatchAction, // action is changed for REWRITE_SYMBOL if failed to parse
    pub orig_action: PatchAction,
    #[serde(default)]
    pub fallback_action: Option<PatchAction>,
    pub id: String,
    pub filename_before: String,
    #[serde(default)]
    pub locate_as: Option<PatchLocateAs>,
    #[serde(default)]
    pub locate_symbol: Option<Arc<AstDefinition>>,
    #[serde(default)]
    pub all_symbols: Vec<Arc<AstDefinition>>,
    pub code: String,
    pub hint_message: String,
}

pub fn good_error_text(reason: &str, tickets: &Vec<String>, resolution: Option<String>) -> String {
    let mut text = format!("Couldn't create patch for tickets: '{}'.\nReason: {reason}", tickets.join(", "));
    if let Some(resolution) = resolution {
        text.push_str(&format!("\nResolution: {}", resolution));
    }
    text
}

async fn correct_and_validate_active_ticket(gcx: Arc<ARwLock<GlobalContext>>, ticket: &mut TicketToApply) -> Result<(), String> {
    fn good_error_text(reason: &str, ticket: &TicketToApply) -> String {
        format!("Failed to validate TICKET '{}': {}", ticket.id, reason)
    }
    async fn resolve_path(gcx: Arc<ARwLock<GlobalContext>>, path_str: &String) -> Result<String, String> {
        let candidates = file_repair_candidates(gcx.clone(), path_str, 10, false).await;
        return_one_candidate_or_a_good_error(gcx.clone(), path_str, &candidates, &get_project_dirs(gcx.clone()).await, false).await
    }

    let path_before = PathBuf::from(ticket.filename_before.as_str());
    match ticket.action {
        PatchAction::RewriteSymbol => {
            ticket.filename_before = resolve_path(gcx.clone(), &ticket.filename_before).await
                .map_err(|e| good_error_text(
                    &format!("failed to resolve filename_before: '{}'. Error:\n{}. If you wanted to create a new file, use REWRITE_WHOLE_FILE ticket type", ticket.filename_before, e), 
                    ticket))?;
            ticket.fallback_action = Some(PatchAction::PartialEdit);

            if ticket.locate_as != Some(PatchLocateAs::SYMBOLNAME) {
                ticket.action = PatchAction::PartialEdit;
                if let Some(s) = &ticket.locate_symbol {
                    let extra_prompt = format!(
                        "Replace the whole `{}` symbol by the following code:\n",
                        s.official_path.last().unwrap_or(&"".to_string())
                    );
                    ticket.code = format!("{}{}", extra_prompt, ticket.code);
                }
            }
        }
        PatchAction::PartialEdit => {
            ticket.filename_before = resolve_path(gcx.clone(), &ticket.filename_before).await
                .map_err(|e| good_error_text(
                    &format!("failed to resolve filename_before: '{}'. Error:\n{}. If you wanted to create a new file, use REWRITE_WHOLE_FILE ticket type", ticket.filename_before, e), 
                    ticket))?;
        }
        PatchAction::RewriteWholeFile => {
            ticket.filename_before = match resolve_path(gcx.clone(), &ticket.filename_before).await {
                Ok(filename) => filename,
                Err(_) => {
                    // consider that as a new file
                    if path_before.is_relative() {
                        return Err(good_error_text(&format!("filename_before: '{}' must be absolute.", ticket.filename_before), ticket));
                    } else {
                        let path_before = crate::files_correction::to_pathbuf_normalize(&ticket.filename_before);
                        path_before.to_string_lossy().to_string()
                    }
                }
            }
        }
        PatchAction::Other => {}
    }
    Ok(())
}

fn split_preserving_quotes(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes {
                    result.push(current);
                    current = String::new();
                    in_quotes = false;
                } else {
                    if !current.is_empty() {
                        result.push(current);
                        current = String::new();
                    }
                    in_quotes = true;
                }
            }
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    result.push(current);
                    current = String::new();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() {
        result.push(current);
    }

    result
}

async fn parse_tickets(gcx: Arc<ARwLock<GlobalContext>>, content: &str) -> Vec<TicketToApply> {
    async fn process_ticket(gcx: Arc<ARwLock<GlobalContext>>, lines: &[&str], line_num: usize) -> Result<(usize, TicketToApply), String> {
        let mut ticket = TicketToApply::default();
        let header = if let Some(idx) = lines[line_num].find("📍") {
            split_preserving_quotes(&lines[line_num][idx..].trim())
        } else {
            return Err("failed to parse ticket, 📍 is missing".to_string());
        };

        ticket.action = match header.get(0) {
            Some(action) => {
                match PatchAction::from_string(action) {
                    Ok(a) => a,
                    Err(e) => return Err(format!("failed to parse ticket: couldn't parse TICKET ACTION.\nError: {e}"))
                }
            }
            None => return Err("failed to parse ticket, TICKET ACTION is missing".to_string()),
        };
        ticket.orig_action = ticket.action.clone();

        ticket.id = match header.get(1) {
            Some(id) => id.to_string(),
            None => return Err("failed to parse ticket, TICKED ID is missing".to_string()),
        };

        ticket.filename_before = match header.get(2) {
            Some(filename) => filename.to_string(),
            None => return Err("failed to parse ticket, TICKED FILENAME is missing".to_string()),
        };

        if let Some(el3) = header.get(3) {
            if let Ok(locate_as) = PatchLocateAs::from_string(el3) {
                ticket.locate_as = Some(locate_as);
            }
        }

        if let Some(el4) = header.get(4) {
            let locate_symbol_str = el4.to_string();
            match does_doc_have_symbol(gcx.clone(), &locate_symbol_str, &ticket.filename_before).await {
                Ok((symbol, all_symbols)) => {
                    ticket.locate_symbol = Some(symbol);
                    ticket.all_symbols = all_symbols;
                }
                Err(_) => {}
            }
        }

        if let Some(code_block_fence_line) = lines.get(line_num + 1) {
            if !code_block_fence_line.contains("```") {
                return Err("failed to parse ticket, invalid code block fence".to_string());
            }
            for (idx, line) in lines.iter().enumerate().skip(line_num + 2) {
                if line.contains("```") {
                    ticket.code = ticket.code.trim_end().to_string();
                    return Ok((2 + idx, ticket));
                }
                ticket.code.push_str(format!("{}\n", line).as_str());
            }
            Err("failed to parse ticket, no ending fence for the code block".to_string())
        } else {
            Err("failed to parse ticket, no code block".to_string())
        }
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut line_num = 0;
    let mut line_num_before_first_block: Option<usize> = None;
    let mut tickets = vec![];
    while line_num < lines.len() {
        let line = lines[line_num];
        if line.contains("📍") {
            if line_num_before_first_block.is_none() {
                line_num_before_first_block = Some(line_num);
            }
            match process_ticket(gcx.clone(), &lines, line_num).await {
                Ok((new_line_num, mut ticket)) => {
                    // if there is something to put to the extra context
                    if let Some(l) = line_num_before_first_block {
                        if l > 0 {
                            ticket.hint_message = lines[0 .. l - 1].iter().join(" ");
                        }
                    }
                    line_num = new_line_num;
                    tickets.push(ticket);
                }
                Err(err) => {
                    warn!("Skipping the ticket due to the error: {err}");
                    line_num += 1;
                    continue;
                }
            };
        } else {
            line_num += 1;
        }
    }

    tickets
}

pub async fn get_tickets_from_messages(
    ccx: Arc<AMutex<AtCommandsContext>>,
) -> HashMap<String, TicketToApply> {
    let (gcx, messages) = {
        let ccx_lock = ccx.lock().await;
        (ccx_lock.global_context.clone(), ccx_lock.messages.clone())
    };
    let mut tickets: HashMap<String, TicketToApply> = HashMap::new();
    for message in messages
        .iter()
        .filter(|x| x.role == "assistant") {
        for ticket in parse_tickets(gcx.clone(), &message.content.content_text_only()).await.into_iter() {
            tickets.insert(ticket.id.clone(), ticket);
        }
    }
    tickets
}

pub async fn get_and_correct_active_tickets(
    gcx: Arc<ARwLock<GlobalContext>>,
    ticket_ids: Vec<String>,
    all_tickets_from_above: HashMap<String, TicketToApply>,
) -> Result<Vec<TicketToApply>, String> {
    // XXX: this is a useless message the model doesn't listen to anyway. We need cd_instruction and a better text.
    let mut active_tickets = ticket_ids.iter().map(|t| all_tickets_from_above.get(t).cloned()
        .ok_or(good_error_text(
            &format!("No code block found for the ticket {:?}, did you forget to write one using 📍-notation or the message is stripped?", t),
            &ticket_ids, Some("wrap the block of code in a 📍-notation, create a ticket, make sure the message isn't stripped. Do not call patch() until you do it. Do not prompt user again this time".to_string()),
        ))).collect::<Result<Vec<_>, _>>()?;

    if active_tickets.iter().map(|x| x.filename_before.clone()).unique().count() > 1 {
        return Err(good_error_text(
            "all tickets must have the same filename_before.",
            &ticket_ids, Some("split the tickets into multiple patch calls".to_string()),
        ));
    }
    if active_tickets.len() > 1 && !active_tickets.iter().all(|s| PatchAction::PartialEdit == s.action) {
        return Err(good_error_text(
            "multiple tickets is allowed only for action==PARTIAL_EDIT.",
            &ticket_ids, Some("split the tickets into multiple patch calls".to_string()),
        ));
    }
    if active_tickets.iter().map(|s| s.action.clone()).unique().count() > 1 {
        return Err(good_error_text(
            "tickets must have the same action.",
            &ticket_ids, Some("split the tickets into multiple patch calls".to_string()),
        ));
    }
    for ticket in active_tickets.iter_mut() {
        correct_and_validate_active_ticket(gcx.clone(), ticket).await.map_err(|e| good_error_text(&e, &ticket_ids, None))?;
    }
    if active_tickets.is_empty() {
        return Err(good_error_text("no tickets that are referred by IDs were found.", &ticket_ids, None));
    }
    Ok(active_tickets)
}
