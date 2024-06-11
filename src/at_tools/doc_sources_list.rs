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
        _: &mut AtCommandsContext,
        tool_call_id: &String,
        _: &HashMap<String, Value>,
    ) -> Result<Vec<ContextEnum>, String> {
        let results = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "[]".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })];
        Ok(results)
    }
}
