use std::collections::HashMap;
use async_trait::async_trait;
use serde_json::Value;
use crate::at_commands::at_commands::{AtCommandsContext, vec_context_file_to_context_tools};
use crate::at_commands::at_workspace::{execute_at_workspace, text_on_clip};
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum, RChatMessage};


pub struct AttWorkspace;

#[async_trait]
impl Tool for AttWorkspace {
    async fn tool_execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, (String, Option<ChatUsage>)> {
        let query = match args.get("query") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err((format!("argument `query` is not a string: {:?}", v), None)),
            None => return Err(("Missing argument `query` for att_workspace".to_string(), None))
        };
        let vector_of_context_file = execute_at_workspace(ccx, &query, None).await.map_err(|e|(e, None))?;
        let text = text_on_clip(&query, true);

        let mut results = vec_context_file_to_context_tools(vector_of_context_file);
        results.push(ContextEnum::RChatMessage(RChatMessage::new(ChatMessage {
            role: "tool".to_string(),
            content: text,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })));
        Ok(results)
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec!["vecdb".to_string()]
    }
}
