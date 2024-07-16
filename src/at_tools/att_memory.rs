use std::collections::HashMap;
use serde_json::Value;
use tracing::info;

use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};


pub struct AttSaveMemory;

fn validate_args(args: &HashMap<String, Value>) -> Result<(String, String, String), String> {
    let memory_topic = match args.get("memory_topic") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err("argument `memory_topic` is missing or not a string".to_string()),
    };

    let memory_text = match args.get("memory_text") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err("argument `memory_text` is missing or not a string".to_string()),
    };

    let memory_type = match args.get("memory_type") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err("argument `memory_type` is missing or not a string".to_string()),
    };

    if !["consequence", "reflection", "familiarity", "relationship"].contains(&memory_type.as_str()) {
        return Err(format!("Invalid memory_type: {}. Must be one of: consequence, reflection, familiarity, relationship", memory_type));
    }

    Ok((memory_topic, memory_text, memory_type))
}

#[async_trait]
impl Tool for AttSaveMemory {
    async fn tool_execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        info!("run @save-memory {:?}", args);
        let (memory_topic, memory_text, memory_type) = validate_args(args)?;
        
        let memdb = {
            let vec_db = ccx.global_context.read().await.vec_db.clone();
            let vec_db_guard = vec_db.lock().await;
            let vec_db_ref = vec_db_guard.as_ref().ok_or("Unable to execute save-memory: vecdb is not available".to_string())?;
            vec_db_ref.memdb.clone()
        };
        let _memid = memdb.lock().await.permdb_add(&memory_type, &memory_topic, "", &memory_text)?;
        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: memory_text,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        }));
        Ok(results)
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec!["vecdb".to_string()]
    }
    fn prompt(&self) -> Option<&str> {
        Some(r#"
TOOL: save-memory
you must call it AUTOMATICALLY without user reminding you or telling you to use it.
you must call it even if you are expected to respond. Call the tool, continue to respond.
CASES WHEN save-memory MUST BE CALLED:
1. user mentions: personal information, preferences, information about his persona, information about his acquintances;
2. user mentions tech he uses, any technical details about tech he uses;
3. user mentions technical details about his projects;
4. user complains, criticises, teaches, guides you, when he gives advices or instructions.

you must AUTOMATICALLY locate mentioned cases in user message even if user mentions it briefly and implicitly.
you can use save-memory in parallel if needed.

if knowledge you want to save is complex, consider split it into several memories.
        "#)
    }
}
