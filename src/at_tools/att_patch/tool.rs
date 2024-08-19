use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use itertools::Itertools;
use tokio::sync::Mutex as AMutex;
use tracing::warn;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::att_patch::args_parser::parse_arguments;
use crate::at_tools::att_patch::chat_interaction::execute_chat_model;
use crate::at_tools::att_patch::diff_formats::parse_diff_chunks_from_message;
use crate::at_tools::att_patch::unified_diff_format::UnifiedDiffFormat;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum};

pub const DEFAULT_MODEL_NAME: &str = "gpt-4o-mini";
pub const MAX_NEW_TOKENS: usize = 8192;
pub const TEMPERATURE: f32 = 0.7;
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

fn choose_correct_chunk(chunks: Vec<Result<String, String>>) -> Result<Vec<(String, i32)>, String> {
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

    // count chunks
    let mut chunks_counter: HashMap<&str, i32> = HashMap::new();
    for chunk in non_error_chunks.iter() {
        *chunks_counter.entry(chunk.as_str()).or_insert(0) += 1;
    }
    Ok(chunks_counter
      .into_iter()
      .map(|(k, v)| (k.to_string(), v))
      .sorted_by_key(|x| -x.1)
      .collect())
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
        let mut usage = ChatUsage{..Default::default()};
        let answers = match execute_chat_model(
            ccx.clone(),
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
            let parsed_chunks = parse_diff_chunks_from_message(ccx.clone(), &answer).await;
            chunks_for_answers.push(parsed_chunks);
        }
        let chunks = choose_correct_chunk(chunks_for_answers)?;

        let mut messages = vec![];
        for (chunk, count) in chunks {
            let mut chunk_usage = ChatUsage{..Default::default()};
            if messages.is_empty() {
                chunk_usage = usage.clone();
            }
            messages.push(ContextEnum::ChatMessage(ChatMessage {
                role: "diff".to_string(),
                content: chunk,
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: Some(chunk_usage),
                count: count,
            }));
        }

        Ok(messages)
    }

    fn usage(&mut self) -> &mut Option<ChatUsage> {
        &mut self.usage
    }
}
