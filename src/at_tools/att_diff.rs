use std::collections::HashMap;
use std::path::PathBuf;
use serde_json::Value;

use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_diff::execute_diff_for_vcs;
use crate::at_commands::at_file::{at_file_repair_candidates, get_project_paths};
use crate::at_tools::att_file::real_file_path_candidate;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::files_correction::correct_to_nearest_dir_path;


pub struct AttDiff;

#[async_trait]
impl Tool for AttDiff {
    async fn tool_execute(&mut self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        let p = match args.get("path") {
            Some(Value::String(s)) => s,
            Some(v) => { return Err(format!("argument `path` is not a string: {:?}", v)) },
            None => { return Err("argument `path` is missing".to_string()) }
        };
        
        let diff_chunks = {
            let p_path = PathBuf::from(p);
            if p_path.extension().is_some() {
                let candidates = at_file_repair_candidates(p, ccx, false).await;
                let candidate = real_file_path_candidate(ccx, p, &candidates, &get_project_paths(ccx).await, false).await?;
                let parent_dir = PathBuf::from(&candidate).parent().ok_or(format!("Couldn't get parent directory of file: {:?}", candidate))?.to_string_lossy().to_string();
                execute_diff_for_vcs(&parent_dir, &[&candidate]).await.map_err(|e| format!("Couldn't execute git diff {}.\nError: {}", candidate, e))?
            } else {
                let candidates = correct_to_nearest_dir_path(ccx.global_context.clone(), p, false, 10).await;
                let candidate = real_file_path_candidate(ccx, p, &candidates, &get_project_paths(ccx).await, true).await?;
                execute_diff_for_vcs(&candidate, &[]).await.map_err(|e| format!("Couldn't execute git diff.\nError: {}", e))?
            }
        };
        let mut results = vec![];
        
        for chunk in diff_chunks {
            results.push(ContextEnum::ChatMessage(ChatMessage::new(
                "plain_text".to_string(), chunk
            )));
        }
        
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "executed diff function".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));

        Ok(results)
    }
}
