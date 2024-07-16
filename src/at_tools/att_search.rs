use std::collections::HashMap;
use async_trait::async_trait;
use serde_json::Value;
use crate::at_commands::at_commands::{AtCommandsContext, vec_context_file_to_context_tools};
use crate::at_commands::at_file::{at_file_repair_candidates, get_project_paths};
use crate::at_commands::at_search::{execute_at_search, text_on_clip};
use crate::at_tools::att_file::real_file_path_candidate;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum, ContextFile};


pub struct AttSearch;


async fn validate_and_correct_args(ccx: &mut AtCommandsContext, args: &HashMap<String, Value>) -> Result<(String, String, Option<String>, Option<String>), String> {
    let query = match args.get("query") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => return Err(format!("argument `query` is not a string: {:?}", v)),
        None => return Err("Missing argument `query` for att_search".to_string())
    };
    let scope = match args.get("scope") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => return Err(format!("argument `scope` is not a string: {:?}", v)),
        None => return Err("Missing argument `scope` for att_search".to_string())
    };
    let project_name = match args.get("project_name") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(v) => return Err(format!("argument `project_name` is not a string: {:?}", v)),
        None => None
    };
    let file_name = match args.get("file_name") {
        Some(Value::String(s)) => {
            let candidates = at_file_repair_candidates(s, ccx, false).await;
            Some(real_file_path_candidate(ccx, s, &candidates, &get_project_paths(ccx).await).await?)
        },
        Some(v) => return Err(format!("argument `file_name` is not a string: {:?}", v)),
        None => None
    };

    if project_name.is_some() && file_name.is_some() {
        return Err("`project_name` and `file_name` can't be specified together. Choose one.".to_string());
    }
    if (project_name.is_some() || file_name.is_some()) && &scope != "fs" {
        return Err("`project_name` and `file_name` can be specified for `scope` == `fs`".to_string());
    }
    Ok((query, scope, project_name, file_name))
}

async fn execute_att_search(ccx: &mut AtCommandsContext, query: &String, scope: &String, project_name_mb: Option<String>, file_name_mb: Option<String>) -> Result<Vec<ContextFile>, String> {
    return match scope.as_str() {
        "fs" => {
            let mut filter = "".to_string();
            if let Some(project_name) = project_name_mb {
                filter.push_str(&format!("(file_path LIKE '{}%')", project_name));
            }
            if let Some(file_name) = file_name_mb {
                if !filter.is_empty() {
                    filter = format!("{} AND ", filter);
                }
                filter.push_str(&format!("(file_path = \"{}\")", file_name));
            }
            let filter = if filter.is_empty() { None } else { Some(filter) };
            println!("att-search: filter={:?}", filter);
            Ok(execute_at_search(ccx, &query, filter).await?)
        },
        _ => Err(format!("Scope {} is not implemented", scope))
    };
}

#[async_trait]
impl Tool for AttSearch {
    async fn execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        let (query, scope, project_name, file_name) = validate_and_correct_args(ccx, args).await?;
        println!("att-search: query={:?}, scope={:?}, project_name={:?}, file_name={:?}", query, scope, project_name, file_name);
        let vector_of_context_file = execute_att_search(ccx, &query, &scope, project_name, file_name).await?;
        println!("att-search: vector_of_context_file={:?}", vector_of_context_file);
        let text = text_on_clip(&query, true);

        let mut results = vec_context_file_to_context_tools(vector_of_context_file);
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: text,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        }));
        Ok(results)
    }
    fn depends_on(&self) -> Vec<String> {
        vec!["vecdb".to_string()]
    }
}
