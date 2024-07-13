use std::collections::HashMap;
use serde_json::Value;
use tracing::info;

use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::vecdb::vdb_highlev::memories_search;
use crate::vecdb::vdb_structs::MemoRecord;

pub struct AttGetKnowledge;


#[async_trait]
impl Tool for AttGetKnowledge {
    async fn tool_execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        info!("run @get-knowledge {:?}", args);
        let im_going_to_do = match args.get("im_going_to_do") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => { return Err(format!("argument `im_going_to_do` is not a string: {:?}", v)) },
            None => { return Err("argument `im_going_to_do` is missing".to_string()) }
        };

        let vec_db = ccx.global_context.read().await.vec_db.clone();
        let mut memories: crate::vecdb::vdb_structs::MemoSearchResult = memories_search(vec_db.clone(), &im_going_to_do, ccx.top_n).await?;

        // 💿 There is no ongoing session on the topic of "{im_going_to_do}". A new session was created, it's a good idea to formulate a plan.
        // 💿 An ongoing session on the topic of "{im_going_to_do}" is activated. Continue with your plan.
        if false {
            memories.results.push(
                MemoRecord {
                    memid: "ongoing".to_string(),
                    thevec: None,
                    distance: 2.0,
                    m_type: "ongoing".to_string(),
                    m_goal: im_going_to_do.clone(),
                    m_project: "project1".to_string(),
                    m_payload: format!("💿 An ongoing session on the topic \"{}\" is activated. Continue with your plan.", im_going_to_do),
                    ..Default::default()
                }
            );
        }

        let memories_json = memories.results.iter().map(|m| {
            serde_json::json!({
                "memid": m.memid.clone(),
                "content": m.m_payload.clone()
            })
        }).collect::<Vec<Value>>();
        let mut memories_str = serde_json::to_string(&memories_json).unwrap();
        if false {
            // works with sonnet
            memories_str.push_str("\n\n💿 There is no ongoing session with this goal. A new empty ongoing session is created, formulate your plan now.");
        }

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: memories_str,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        }));
        // better for gpt-3.5
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "user".to_string(),
            content: "💿 There is no ongoing session with this goal. A new empty ongoing session is created, formulate your plan now.".to_string(),
            tool_calls: None,
            tool_call_id: "".to_string(),
        }));
        Ok(results)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["vecdb".to_string()]
    }
}

// pub struct AttSaveKnowledge;
// #[async_trait]
// impl Tool for AttSaveKnowledge {
//     async fn execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
//         info!("run @save-knowledge {:?}", args);
//         let memory_topic = match args.get("memory_topic") {
//             Some(Value::String(s)) => s,
//             _ => return Err("argument `memory_topic` is missing or not a string".to_string()),
//         };
//         let memory_text = match args.get("memory_text") {
//             Some(Value::String(s)) => s,
//             _ => return Err("argument `memory_text` is missing or not a string".to_string()),
//         };
//         let memory_type = match args.get("memory_type") {
//             Some(Value::String(s)) => s,
//             _ => return Err("argument `memory_type` is missing or not a string".to_string()),
//         };
//         if !["consequence", "reflection", "familiarity", "relationship"].contains(&memory_type.as_str()) {
//             return Err(format!("Invalid memory_type: {}. Must be one of: consequence, reflection, familiarity, relationship", memory_type));
//         }
//         let memdb = {
//             let vec_db = ccx.global_context.read().await.vec_db.clone();
//             let vec_db_guard = vec_db.lock().await;
//             let vec_db_ref = vec_db_guard.as_ref().ok_or("vecdb is not available".to_string())?;
//             vec_db_ref.memdb.clone()
//         };
//         let _memid = memdb.lock().await.permdb_add(memory_type, memory_topic, "current_project", memory_text)?;
//         let mut results = vec![];
//         results.push(ContextEnum::ChatMessage(ChatMessage {
//             role: "tool".to_string(),
//             content: format!("Model will remember it:\n{memory_text}"),
//             tool_calls: None,
//             tool_call_id: tool_call_id.clone(),
//         }));
//         Ok(results)
//     }
//     fn depends_on(&self) -> Vec<String> {
//         vec!["vecdb".to_string()]
//     }
// }
