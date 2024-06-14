use std::collections::HashMap;
use serde_json::Value;

use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_diff::{execute_diff_for_vcs, text_on_clip, last_accessed_project};
use crate::at_commands::execute_at::AtCommandMember;
use crate::at_tools::tools::AtTool;
use crate::call_validation::{ChatMessage, ContextEnum};


pub struct AttDiff;

#[async_trait]
impl AtTool for AttDiff {
    async fn execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        let project_path = last_accessed_project(ccx).await?;
        let diff_chunks = match args.len() {
            0 => {
                // No arguments: git diff for all tracked files
                execute_diff_for_vcs(&project_path, &[]).await.map_err(|e| format!("Couldn't execute git diff.\nError: {}", e))
            },
            1 => {
                // 1 argument: git diff for a specific file
                let file_path = args.get("file_path").and_then(|v| v.as_str()).ok_or("Missing argument `file_path` for att_diff")?;
                execute_diff_for_vcs(&project_path, &[file_path]).await.map_err(|e| format!("Couldn't execute git diff {}.\nError: {}", file_path, e))
            },
            _ => {
                return Err("Invalid number of arguments".to_string());
            },
        }?;

        let text = text_on_clip(&args.iter().map(|(_k, v)| AtCommandMember { text: v.to_string(), ..Default::default() }).collect());
        let mut results = diff_chunks.into_iter().map(ContextEnum::DiffChunk).collect::<Vec<_>>();

        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: text,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        }));

        Ok(results)
    }
}
