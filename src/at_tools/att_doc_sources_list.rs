use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::at_tools::AtTool;
use crate::call_validation::{ChatMessage, ContextEnum};

pub struct AttDocSourcesList;

#[async_trait]
impl AtTool for AttDocSourcesList {
    async fn execute(
        &self,
        ccx: &mut AtCommandsContext,
        tool_call_id: &String,
        _: &HashMap<String, Value>,
    ) -> Result<Vec<ContextEnum>, String> {
        let sources = ccx
            .global_context
            .read()
            .await
            .documents_state
            .documentation_files
            .lock()
            .await
            .join(",");

        let results = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: format!("[{sources}]"),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })];
        Ok(results)
    }
}
