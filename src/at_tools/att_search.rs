use std::sync::Arc;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use serde_json::Value;
use tracing::info;

use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::{AtCommandsContext, vec_context_file_to_context_tools};
use crate::at_commands::at_file::{file_repair_candidates, get_project_paths, real_file_path_candidate};
use crate::at_commands::at_search::execute_at_search;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum, ContextFile};


pub struct AttSearch;

async fn execute_att_search(
    ccx: Arc<AMutex<AtCommandsContext>>,
    query: &String,
    scope: &String
) -> Result<Vec<ContextFile>, String> {
    fn is_scope_a_file(scope: &String) -> bool {
        PathBuf::from(scope).extension().is_some()
    }
    fn is_scope_a_dir(scope: &String) -> bool {
        let path = PathBuf::from(scope);
        match fs::metadata(&path) {
            Ok(metadata) => metadata.is_dir(),
            Err(_) => false,
        }
    }
    let gcx = ccx.lock().await.global_context.clone();

    return match scope.as_str() {
        "workspace" => {
            Ok(execute_at_search(ccx.clone(), &query, None).await?)
        },
        _ if is_scope_a_file(scope) => {
            let candidates = file_repair_candidates(gcx.clone(), scope, 10, false).await;
            let file_path = real_file_path_candidate(
                gcx.clone(),
                scope,
                &candidates,
                &get_project_paths(gcx.clone()
            ).await, false).await?;
            let filter = Some(format!("(file_path = \"{}\")", file_path));
            Ok(execute_at_search(ccx.clone(), &query, filter).await?)
        },
        _ if is_scope_a_dir(scope) => {
            // TODO: complete path
            let filter = format!("(file_path LIKE '{}%')", scope);
            Ok(execute_at_search(ccx.clone(), &query, Some(filter)).await?)
        },
        _ => Err(format!("scope {} is not supported", scope))
    };
}

#[async_trait]
impl Tool for AttSearch {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<Vec<ContextEnum>, String> {
        let query = match args.get("query") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `query` is not a string: {:?}", v)),
            None => return Err("Missing argument `query` in the search() call.".to_string())
        };
        let scope = match args.get("scope") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `scope` is not a string: {:?}", v)),
            None => return Err("Missing argument `scope` in the search() call.".to_string())
        };
        
        let vector_of_context_file = execute_att_search(ccx.clone(), &query, &scope).await?;
        info!("att-search: vector_of_context_file={:?}", vector_of_context_file);
        
        if vector_of_context_file.is_empty() {
            return Err("search has given no results. Adjust a query or try a different scope".to_string());
        }
        
        let file_results = vector_of_context_file.iter().map(|c|c.file_name.clone()).collect::<Vec<_>>();
        let mut file_results_counters = HashMap::new();
        for f in file_results.iter() {
            *file_results_counters.entry(f.clone()).or_insert(0usize) += 1;
        }
        let mut content = "Records found:\n".to_string();
        for (file_name, count) in file_results_counters.iter() {
            content.push_str(&format!("{}: {}\n", file_name, count));
        }
        
        let mut results = vec_context_file_to_context_tools(vector_of_context_file.clone());
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));
        Ok(results)
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec!["vecdb".to_string()]
    }
}
