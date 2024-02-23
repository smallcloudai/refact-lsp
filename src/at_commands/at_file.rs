use std::sync::Arc;
use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Mutex as AMutex;

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
    async fn are_args_valid(&self, args: &Vec<String>, context: &AtCommandsContext) -> Vec<bool> {
        let mut results = Vec::new();
        for (arg, param) in args.iter().zip(self.params.iter()) {
            let param = param.lock().await;
            results.push(param.is_value_valid(arg, context).await);
        }
        results
    }

    async fn can_execute(&self, args: &Vec<String>, context: &AtCommandsContext) -> bool {
        if self.are_args_valid(args, context).await.iter().any(|&x| x == false) || args.len() != self.params.len() {
            return false;
        }
        return true;
    }

    async fn execute(&self, _query: &String, args: &Vec<String>, _top_n: usize, context: &AtCommandsContext, parsed_args: &HashMap<String, String>) -> Result<ChatMessage, String> {
        let can_execute = self.can_execute(args, context).await;
        if !can_execute {
            return Err("incorrect arguments".to_string());
        }
        let file_path = match args.get(0) {
            Some(x) => x,
            None => return Err("no file path".to_string()),
        };

        let mut file_text = get_file_text_from_memory_or_disk(context.global_context.clone(), file_path).await?;
        let lines_cnt = file_text.lines().count() as i32;

        let line1 = match parsed_args.get("file_start_line") {
            Some(value) => value.parse::<i32>().map(|x|x-1).unwrap_or(0).max(0).min(lines_cnt),
            None => 0,
        };
        let mut line2 = match parsed_args.get("file_end_line") {
            Some(value) => value.parse::<i32>().map(|x|x-1).unwrap_or(lines_cnt).max(0).min(lines_cnt),
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
