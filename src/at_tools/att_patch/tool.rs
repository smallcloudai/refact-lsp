use async_trait::async_trait;
use itertools::Itertools;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tracing::warn;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::att_locate::locate_utils::unwrap_subchat_params;
use crate::at_tools::att_patch::chat_interaction::execute_chat_model;
use crate::at_tools::att_patch::diff_formats::parse_diff_chunks_from_message;
use crate::at_tools::att_patch::unified_diff_format::UnifiedDiffFormat;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum};

pub const N_CHOICES: usize = 16;
pub type DefaultToolPatch = UnifiedDiffFormat;


pub struct PatchArguments {
    pub paths: Vec<String>,
    pub todo: String,
    pub use_locate_for_context: bool,
}

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

pub async fn parse_arguments(
    args: &HashMap<String, Value>,
) -> Result<PatchArguments, String> {
    let paths = match args.get("paths") {
        Some(Value::String(s)) => s.split(",").map(|x| x.to_string()).collect::<Vec<String>>(),
        Some(v) => { return Err(format!("argument `paths` is not a string: {:?}", v)) }
        None => { return Err("argument `path` is not a string".to_string()) }
    };
    let use_locate_for_context = if let Some(p) = paths.get(0) {
        p == "pick_locate_json_above"
    } else {
        false
    };
    let todo = match args.get("todo") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => { return Err(format!("argument `todo` is not a string: {:?}", v)) }
        None => { "".to_string() }
    };
    Ok(PatchArguments {
        paths,
        todo,
        use_locate_for_context,
    })
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
        .filter(|(k, v)| **v == max_repeats)
        .map(|x| x.0.clone())
        .collect::<Vec<_>>();
    Ok(chunks_max_repeats
        .iter()
        .max()
        .expect("There is no max repeats")
        .to_string())
}

#[async_trait]
impl Tool for ToolPatch {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<Vec<ContextEnum>, String> {
        let args = match parse_arguments(args).await {
            Ok(res) => res,
            Err(err) => {
                return Err(format!("Cannot parse input arguments: {err}. Try to call `patch` one more time with valid arguments"));
            }
        };
        let mut usage = ChatUsage { ..Default::default() };

        let params = unwrap_subchat_params(ccx.clone(), "patch").await?;
        let ccx_subchat = {
            let ccx_lock = ccx.lock().await;
            Arc::new(AMutex::new(AtCommandsContext::new(
                ccx_lock.global_context.clone(),
                params.n_ctx,
                ccx_lock.top_n,
                false,
                ccx_lock.messages.clone(),
            ).await))
        };

        let answers = match execute_chat_model(
            ccx_subchat.clone(),
            &params.model,
            params.n_ctx,
            params.temperature,
            params.max_new_tokens,
            tool_call_id,
            &args,
            &mut usage,
        ).await {
            Ok(res) => res,
            Err(err) => {
                return Err(format!("Patch model execution problem: {err}. Try to call `patch` one more time"));
            }
        };

        let mut chunks_for_answers = vec![];
        for answer in answers.iter() {
            warn!("Patch model answer:\n{}", &answer);
            let parsed_chunks = parse_diff_chunks_from_message(ccx_subchat.clone(), &answer).await;
            chunks_for_answers.push(parsed_chunks);
        }
        let chunks = choose_correct_chunk(chunks_for_answers)?;

        Ok(vec![
            ContextEnum::ChatMessage(ChatMessage {
                role: "diff".to_string(),
                content: chunks,
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: Some(usage),
            })
        ])
    }

    fn usage(&mut self) -> &mut Option<ChatUsage> {
        &mut self.usage
    }
}
