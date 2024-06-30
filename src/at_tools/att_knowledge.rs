use std::collections::HashMap;
use serde_json::Value;
use tracing::info;

use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};


pub struct AttKnowledge;

#[async_trait]
impl Tool for AttKnowledge {
    async fn execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        info!("run @knowledge {:?}", args);
        let mut im_going_to_do = match args.get("im_going_to_do") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => { return Err(format!("argument `im_going_to_do` is not a string: {:?}", v)) },
            None => { return Err("argument `im_going_to_do` is missing".to_string()) }
        };

        let mut memories: Vec<String> = vec![];
        memories.push("memory 5f4he83\nThe Frog class represents a frog in a 2D environment, with position and velocity attributes. It is defined at /Users/kot/code/refact-lsp/tests/emergency_frog_situation/frog.py:5".to_string());

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: serde_json::to_string(&memories).unwrap(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        }));

        Ok(results)
    }

    fn depends_on(&self) -> Vec<String> {
        vec!["ast".to_string()]
    }
}
