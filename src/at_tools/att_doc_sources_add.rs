use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::at_tools::AtTool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::files_in_workspace::Document;

pub struct AttDocSourcesAdd;

async fn download_and_convert_page(url: &str) -> Result<String, String> {
    let html = reqwest::get(url)
        .await
        .map_err(|_| format!("Unable to connect to '{url}'"))?
        .text()
        .await
        .map_err(|_| "Unable to convert page to text".to_string())?;

    let text = html2text::config::plain()
        .string_from_read(&html.as_bytes()[..], 200)
        .map_err(|_| "Unable to convert html to text".to_string())?;

    Ok(text)
}

#[async_trait]
impl AtTool for AttDocSourcesAdd {
    async fn execute(
        &self,
        ccx: &mut AtCommandsContext,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<Vec<ContextEnum>, String> {
        let mut source = match args.get("source") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `source` is not a string: {:?}", v)),
            None => return Err("Missing source argument for doc_sources_add".to_string()),
        };

        // if the source is a url, download the page and convert it to markdown
        if source.starts_with("http://") || source.starts_with("https://") {
            let page = download_and_convert_page(source.as_str()).await?;

            let file_path = format!("./.refact/{}", source.split("://").nth(1).unwrap());
            let directory = Path::new(&file_path).parent().unwrap();
            fs::create_dir_all(directory)
                .map_err(|_| format!("Unable to create directory {directory:?}"))?;

            let mut file = File::create(&file_path)
                .map_err(|_| format!("Unable to create file {file_path}"))?;

            file.write_all(page.as_bytes())
                .map_err(|_| format!("Unable to write to file {file_path}"))?;
            source = file_path;
        }

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
