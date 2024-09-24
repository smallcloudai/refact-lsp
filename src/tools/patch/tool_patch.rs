use async_trait::async_trait;
use itertools::Itertools;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tracing::warn;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::patch::chat_interaction::execute_chat_model;
use crate::tools::patch::diff_formats::postprocess_diff_chunks_from_message;
use crate::tools::patch::tickets::{TicketToApply, correct_and_validate_ticket, get_tickets_from_messages, PatchAction};
use crate::tools::patch::unified_diff_format::UnifiedDiffFormat;
use crate::tools::patch::whole_file_diff::{full_rewrite_diff, new_file_diff};
use crate::tools::tools_execute::unwrap_subchat_params;
use crate::tools::tools_description::Tool;
use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum, SubchatParameters};


pub const N_CHOICES: usize = 16;
pub type DefaultToolPatch = UnifiedDiffFormat;

pub struct ToolPatch {
    pub usage: Option<ChatUsage>,
}

impl ToolPatch {
    pub fn new() -> Self {
        ToolPatch {
            usage: None
        }
    }
}

fn choose_correct_chunk(chunks: Vec<Result<String, String>>) -> Result<String, String> {
    let errors = chunks
        .iter()
        .filter(|res| res.is_err())
        .map(|res| res.clone().unwrap_err())
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        warn!("There is a list of errors for some generated diffs");
        for err in errors.iter() {
            warn!("{err}");
        }
    }
    if chunks.iter().all(|res| res.is_err()) {
        let mut err_message = "No valid chunks were generated, reasons are:\n".to_string();
        for err in errors.iter().unique() {
            err_message.push_str(format!("- {err}\n").as_str());
        }
        err_message.push_str("Try to call `patch` one more time to generate a correct diff");
        return Err(err_message);
    }

    let non_error_chunks = chunks
        .iter()
        .filter_map(|res| res.as_ref().ok())
        .cloned()
        .collect::<Vec<_>>();
    warn!("{} diff were parsed successfully", non_error_chunks.len());

    // return the most common chunk
    let mut chunks_freq = HashMap::new();
    for chunk in non_error_chunks.iter() {
        *chunks_freq.entry(chunk.as_str()).or_insert(0) += 1;
    }
    let max_repeats = chunks_freq.iter().max_by_key(|(_, k)| *k).unwrap().1.clone();
    let chunks_max_repeats = chunks_freq
        .iter()
        .filter(|(_, v)| **v == max_repeats)
        .map(|x| *x.0)
        .collect::<Vec<_>>();
    Ok(chunks_max_repeats
        .iter()
        .max()
        .expect("There is no max repeats")
        .to_string())
}

async fn partial_edit_tickets_to_diffs(
    ccx_subchat: Arc<AMutex<AtCommandsContext>>,
    tickets: Vec<TicketToApply>,
    params: &SubchatParameters,
    tool_call_id: &String,
    usage: &mut ChatUsage,
) -> Result<String, (String, Option<String>)>{
    let mut all_chunks = execute_chat_model(
        ccx_subchat.clone(),
        tickets,
        &params.subchat_model,
        params.subchat_n_ctx,
        params.subchat_temperature,
        params.subchat_max_new_tokens,
        tool_call_id,
        usage,
    ).await?;

    let mut chunks_for_answers = vec![];
    for chunks in all_chunks.iter_mut() {
        let diffs = postprocess_diff_chunks_from_message(ccx_subchat.clone(), chunks).await;
        chunks_for_answers.push(diffs);
    }
    choose_correct_chunk(chunks_for_answers).map_err(|e|(e, None))
}

async fn tickets_to_diffs(
    ccx_subchat: Arc<AMutex<AtCommandsContext>>,
    path_from_call: String,
    all_tickets_from_above: HashMap<String, TicketToApply>,
    tickets: Vec<String>,
    params: &SubchatParameters,
    tool_call_id: &String,
    usage: &mut ChatUsage,
) -> Result<String, String> {
    let gcx = ccx_subchat.lock().await.global_context.clone();

    fn good_error_text(reason: &str, tickets: &Vec<String>, resolution: Option<String>) -> String {
        let mut text = format!("Couldn't create patch for tickets: '{}'.\nReason: {reason}", tickets.join(", "));
        if let Some(resolution) = resolution {
            text.push_str(&format!("\nResolution: {}", resolution));
        }
        text
    }
    let mut active_tickets = tickets.iter().map(|t|all_tickets_from_above.get(t).cloned()
        .ok_or(good_error_text(
            &format!("No code block found for the ticket {:?} did you forget to write one using 📍-notation?", t),
            &tickets, Some("wrap the block of code in a 📍-notation, creating a ticket, do not call patch() until you do it. Do not prompt user again this time".to_string())
        ))).collect::<Result<Vec<_>, _>>()?;
    drop(all_tickets_from_above);

    if active_tickets.iter().map(|x|x.filename_before.clone()).unique().count() > 1 {
        return Err(good_error_text(
            "all tickets must have the same filename_before.",
            &tickets, Some("split the tickets into multiple patch calls".to_string())
        ));
    }
    if active_tickets[0].filename_before != path_from_call {
        return Err(good_error_text(
            &format!("ticket(s) have different filename from what you provided: '{}'!='{}'.", active_tickets[0].filename_before, path_from_call),
            &tickets, Some("recreate the ticket with correct filename in 📍-notation or change path argument".to_string())
        ));
    }
    if active_tickets.is_empty() {
        return Err(good_error_text("no tickets that are referred by IDs were found.", &tickets, None));
    }
    if active_tickets.len() > 1 && !active_tickets.iter().all(|s|PatchAction::PartialEdit == s.action) {
        return Err(good_error_text(
            "multiple tickets is allowed only for action==PARTIAL_EDIT.",
            &tickets, Some("split the tickets into multiple patch calls".to_string())
        ));
    }
    if active_tickets.iter().map(|s|s.action.clone()).unique().count() > 1 {
        return Err(good_error_text(
            "tickets must have the same action.",
            &tickets, Some("split the tickets into multiple patch calls".to_string())
        ));
    }

    for ticket in active_tickets.iter_mut() {
        correct_and_validate_ticket(gcx.clone(), ticket).await.map_err(|e|good_error_text(&e, &tickets, None))?;
    }

    let action = active_tickets[0].action.clone();
    let result = match action {
        PatchAction::PartialEdit => {
            partial_edit_tickets_to_diffs(
                ccx_subchat.clone(), active_tickets.clone(), params, tool_call_id, usage
            ).await.map_err(|(e, r)| good_error_text(e.as_str(), &tickets, r))
        },
            PatchAction::FullRewrite => {
            let mut chunks = full_rewrite_diff(ccx_subchat.clone(), &active_tickets[0]).await?;
            postprocess_diff_chunks_from_message(ccx_subchat.clone(), &mut chunks).await
        },
        PatchAction::NewFile => {
            let mut chunks = new_file_diff(&active_tickets[0]);
            postprocess_diff_chunks_from_message(ccx_subchat.clone(), &mut chunks).await
        },
        _ => Err(good_error_text(&format!("unknown action provided: '{:?}'.", action), &tickets, None))
    }?;

    Ok(result)
}

#[async_trait]
impl Tool for ToolPatch {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let tickets = match args.get("tickets") {
            Some(Value::String(s)) => s.split(",").map(|s|s.trim().to_string()).collect::<Vec<_>>(),
            Some(v) => { return Err(format!("argument 'ticket' should be a string: {:?}", v)) }
            None => { vec![] }
        };
        let path = match args.get("path") {
            Some(Value::String(s)) => s.trim().to_string(),
            Some(v) => { return Err(format!("argument 'path' should be a string: {:?}", v)) }
            None => { return Err("argument 'path' is required".to_string()) }
        };
        if tickets.is_empty() {
            return Err("`tickets` shouldn't be empty".to_string());
        }

        let mut usage = ChatUsage { ..Default::default() };
        let params = unwrap_subchat_params(ccx.clone(), "patch").await?;
        let ccx_subchat = {
            let ccx_lock = ccx.lock().await;
            Arc::new(AMutex::new(AtCommandsContext::new(
                ccx_lock.global_context.clone(),
                params.subchat_n_ctx,
                ccx_lock.top_n,
                false,
                ccx_lock.messages.clone(),
            ).await))
        };

        let all_tickets_from_above = get_tickets_from_messages(ccx.clone()).await;
        let diff = tickets_to_diffs(
            ccx_subchat,
            path,
            all_tickets_from_above,
            tickets.clone(),
            &params,
            tool_call_id,
            &mut usage,
        ).await?;

        let mut results = vec![];
        results.push(ChatMessage {
            role: "diff".to_string(),
            content: diff,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            usage: Some(usage),
        });
        results.push(ChatMessage::new(
            "disk-system".to_string(),
            format!("💿 Patch generated successfully for TICKETS: {}. User will decide to apply it manually. All you can do now -- summarize patch in one brief paragraph to help user decide", tickets.join("\n")),
        ));

        let results = results.into_iter().map(|x|ContextEnum::ChatMessage(x)).collect::<Vec<_>>();
        Ok((false, results))
    }

    fn usage(&mut self) -> &mut Option<ChatUsage> {
        &mut self.usage
    }
}
