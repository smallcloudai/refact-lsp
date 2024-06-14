use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::mem::swap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use hashbrown::HashSet;
use select::predicate::Name;
use serde_json::Value;
use url::{Position, Url};

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

async fn walk_links(url_str: &str, depth: usize, max_pages: usize) -> Result<Vec<String>, String> {
    let mut visited_pages = HashSet::new();
    let mut queue = vec![];
    let url = Url::parse(url_str).map_err(|_| format!("Invalid url '{url_str}'"))?;
    let base_url = Url::parse(&url[..Position::BeforePath])
        .map_err(|_| format!("Unable to find base url of '{url_str}'"))?;
    let mut files = vec![];

    queue.push(url_str.to_string());
    visited_pages.insert(url_str.to_string());

    for iteration in 0..=depth {
        let mut new_urls = vec![];
        let is_last_iteration = iteration == depth;

        for url in queue {
            let html = reqwest::get(url.clone())
                .await
                .map_err(|_| format!("Unable to connect to '{url}'"))?
                .text()
                .await
                .map_err(|_| "Unable to convert page to text".to_string())?;

            let text = html2text::config::plain()
                .string_from_read(&html.as_bytes()[..], 200)
                .map_err(|_| "Unable to convert html to text".to_string())?;

            // create file
            let file_path = format!("./.refact/{}/parsed.md", url.split("://").nth(1).unwrap());
            let directory = Path::new(&file_path).parent().unwrap();
            fs::create_dir_all(directory)
                .map_err(|_| format!("Unable to create directory {directory:?}"))?;
            let mut file = File::create(&file_path)
                .map_err(|_| format!("Unable to create file {file_path}"))?;
            file.write_all(text.as_bytes())
                .map_err(|_| format!("Unable to write to file {file_path}"))?;
            files.push(file_path);

            if is_last_iteration {
                continue;
            }

            // find links
            let base_parser = Url::options().base_url(Some(&base_url));
            select::document::Document::from(html.as_str())
                .find(Name("a"))
                .filter_map(|n| n.attr("href"))
                .for_each(|link| {
                    let Ok(link) = base_parser.parse(link) else {
                        return;
                    };
                    let link = link.as_str();
                    if visited_pages.len() >= max_pages || visited_pages.contains(link) {
                        return;
                    }
                    new_urls.push(link.to_string());
                    visited_pages.insert(link.to_string());
                });
        }

        queue = vec![];
        swap(&mut queue, &mut new_urls);
    }

    Ok(files)
}

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

        // if the source is an url, download the page and convert it to markdown
        let mut sources = vec![];
        if source.starts_with("http://") || source.starts_with("https://") {
            sources = walk_links(source.as_str(), 2, 40).await?;
        } else {
            sources.push(source);
        }

        for source in &sources {
            let abs_source_path = PathBuf::from(source.as_str());
            if fs::canonicalize(abs_source_path).is_err() {
                return Err(format!("File or directory '{}' doesn't exist", source));
            }
        }

        let gcx = ccx.global_context.write().await;
        let mut files = gcx.documents_state.documentation_files.lock().await;
        let vec_db_module = {
            *gcx.documents_state.cache_dirty.lock().await = true;
            gcx.vec_db.clone()
        };

        for source in sources {
            if !files.contains(&source) {
                files.push(source.clone());
            }
            let document = Document::new(&PathBuf::from(source.as_str()));
            match *vec_db_module.lock().await {
                Some(ref mut db) => db.vectorizer_enqueue_files(&vec![document], false).await,
                None => {}
            };
        }

        let results = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "Successfully added source to documentation list.".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })];
        Ok(results)
    }
}
