use std::collections::HashMap;
use async_trait::async_trait;
use serde_json::Value;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_local_cmdline::execute_cmd;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum, RChatMessage};


pub struct AttExecuteCommand {
    pub command: String,
    pub timeout: usize,
    #[allow(dead_code)]
    pub output_postprocess: String,
}

#[async_trait]
impl Tool for AttExecuteCommand {
    async fn tool_execute(&self, _ccx: &mut AtCommandsContext, tool_call_id: &String, _args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, (String, Option<ChatUsage>)> {
        let (stdout, stderr) = execute_cmd(&self.command, self.timeout).await.map_err(|e|(e, None))?;

        let mut results = vec![];
        results.push(ContextEnum::RChatMessage(RChatMessage::new(ChatMessage {
            role: "tool".to_string(),
            content: format!("Running compile:\n```{}{}```", stdout, stderr),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })));
        Ok(results)
    }
}
