use std::sync::Arc;
use std::collections::HashMap;
use std::path::PathBuf;
use serde_json::Value;
use tracing::warn;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{get_project_paths, real_file_path_candidate};
use crate::at_commands::at_tree::{construct_tree_out_of_flat_list_of_paths, print_files_tree_with_budget};
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::files_correction::{correct_to_nearest_dir_path, paths_from_anywhere};


pub struct AttTree;

fn preformat_path(path: &String) -> String {
    path.trim_end_matches(&['/', '\\'][..]).to_string()
}

#[async_trait]
impl Tool for AttTree {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();
        let paths_from_anywhere = paths_from_anywhere(gcx.clone()).await;

        let path_mb = match args.get("path") {
            Some(Value::String(s)) => {
                let s = preformat_path(s);
                let p = PathBuf::from(&s);
                if p.extension().is_some() {
                    return Err(format!("`path` is a file, not a directory: {:?}", s));
                }
                Some(s)
            },
            Some(v) => return Err(format!("argument `path` is not a string: {:?}", v)),
            None => None,
        };
        let use_ast = match args.get("use_ast") {
            Some(Value::Bool(b)) => *b,
            Some(v) => return Err(format!("argument `use_ast` is not a boolean: {:?}", v)),
            None => false,
        };

        let tree = match path_mb {
            Some(path) => {
                let candidates = correct_to_nearest_dir_path(gcx.clone(), &path, false, 10).await;
                let candidate = real_file_path_candidate(gcx.clone(), &path, &candidates, &get_project_paths(gcx.clone()).await, true).await?;
                let true_path = PathBuf::from(candidate);
                let filtered_paths_from_anywhere = paths_from_anywhere.iter().filter(|f|f.starts_with(&true_path)).cloned().collect();
                construct_tree_out_of_flat_list_of_paths(&filtered_paths_from_anywhere)
            },
            None => construct_tree_out_of_flat_list_of_paths(&paths_from_anywhere)
        };

        let content = print_files_tree_with_budget(ccx.clone(), tree, use_ast).await.map_err(|err| {
            warn!("print_files_tree_with_budget err: {}", err);
            err
        })?;

        Ok((false, vec![
            ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content,
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })
        ]))
    }
}
