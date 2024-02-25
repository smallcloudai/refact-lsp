use std::sync::Arc;
use std::collections::HashMap;

use async_trait::async_trait;
use itertools::Itertools;
use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tracing::info;

use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::at_params::AtParamFilePath;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::call_validation::{ChatMessage, ContextFile};

pub struct AtFile {
    pub name: String,
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtFile {
    pub fn new() -> Self {
        AtFile {
            name: "@file".to_string(),
            params: vec![
                Arc::new(AMutex::new(AtParamFilePath::new()))
            ],
        }
    }
}

#[async_trait]
impl AtCommand for AtFile {
    fn name(&self) -> &String {
        &self.name
    }
    fn params(&self) -> &Vec<Arc<AMutex<dyn AtParam>>> {
        &self.params
    }
    async fn execute(&self, _query: &String, args: &mut Vec<String>, _top_n: usize, context: &AtCommandsContext) -> Result<ChatMessage, String> {
        let (can_execute, parsed_args_mb) = self.can_execute(args, context).await;
        if !can_execute {
            return Err("incorrect arguments".to_string());
        }
        let file_path = match args.get(0) {
            Some(x) => x,
            None => return Err("no file path".to_string()),
        };

        let mut file_text = get_file_text_from_memory_or_disk(context.global_context.clone(), file_path).await?;
        let lines_cnt = file_text.lines().count() as i32;

        let parsed_args = parsed_args_mb.unwrap_or(HashMap::new());
        info!("parsed_args: {:?}", parsed_args);
        let line1 = match parsed_args.get("file_start_line") {
            Some(value) => value.parse::<i32>().map(|x|x-1).unwrap_or(0).max(0).min(lines_cnt),
            None => 0,
        };
        let mut line2 = match parsed_args.get("file_end_line") {
            Some(value) => value.parse::<i32>().unwrap_or(lines_cnt).max(0).min(lines_cnt),
            None => lines_cnt,
        };

        if parsed_args.get("file_start_line").is_some() && parsed_args.get("file_end_line").is_none() {
            line2 = line1 + 1;
        }

        if line2 < line1 {
            return Err("line2 must be greater than line1".to_string());
        }

        let lines: Vec<&str> = file_text.lines().collect();
        file_text = lines[line1 as usize..line2 as usize].join("\n");

        let mut vector_of_context_file: Vec<ContextFile> = vec![];
        vector_of_context_file.push(ContextFile {
            file_name: file_path.clone(),
            file_content: file_text,
            line1,
            line2,
            usefullness: 100.0,
        });
        Ok(ChatMessage {
            role: "context_file".to_string(),
            content: json!(vector_of_context_file).to_string(),
        })
    }
}
