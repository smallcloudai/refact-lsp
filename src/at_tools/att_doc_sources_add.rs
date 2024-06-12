use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::at_tools::AtTool;
use crate::call_validation::{ChatMessage, ContextEnum};

pub struct AttDocSourcesAdd;

#[async_trait]
impl AtTool for AttDocSourcesAdd {
    async fn execute(
        &self,
        ccx: &mut AtCommandsContext,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<Vec<ContextEnum>, String> {
        let source = match args.get("source") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `source` is not a string: {:?}", v)),
            None => return Err("Missing source argument for doc_sources_add".to_string()),
        };

        let gc = ccx.global_context
            .write()
            .await;

        let mut files = gc
            .documents_state
            .documentation_files
            .lock()
            .await;

        if !files.contains(&source) {
            files.push(source);
        }

        let results = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "Succesfully added source to documentation list.".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })];
        Ok(results)
    }
}
