use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::at_tools::AtTool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::files_in_workspace::Document;

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

        let abs_source_path = PathBuf::from(source.as_str());
        if fs::canonicalize(abs_source_path).is_err() {
            return Err(format!("File or directory '{}' doesn't exist", source));
        }

        let gcx = ccx.global_context.write().await;

        let mut files = gcx.documents_state.documentation_files.lock().await;

        if !files.contains(&source) {
            files.push(source.clone());
        }

        let vec_db_module = {
            *gcx.documents_state.cache_dirty.lock().await = true;
            gcx.vec_db.clone()
        };
        let document = Document::new(&PathBuf::from(source.as_str()));
        match *vec_db_module.lock().await {
            Some(ref mut db) => db.vectorizer_enqueue_files(&vec![document], false).await,
            None => {}
        };

        let results = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "Succesfully added source to documentation list.".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })];
        Ok(results)
    }
}
