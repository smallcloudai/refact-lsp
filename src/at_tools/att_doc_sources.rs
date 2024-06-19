use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::mem::swap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use hashbrown::HashSet;
use itertools::Itertools;
use log::{info, warn};
use select::predicate::Name;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task;
use tokio::sync::RwLock as ARwLock;
use url::{Position, Url};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::AtTool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::files_in_workspace::Document;
use crate::global_context::GlobalContext;

pub struct AttDocSources;

#[derive(Serialize, Deserialize)]
struct DocOrigin {
    url: String,
    max_depth: usize,
    max_pages: usize,
    pages: HashMap<String, String>,
}

fn get_directory_and_file_from_url(url: &str) -> Option<(&str, String)> {
    let url_without_http = url.split("://").nth(1).unwrap();
    let (site_name, mut site_path) = url_without_http.split_once("/")?;
    if site_path == "" {
        site_path = "index";
    }
    if site_path.ends_with("/") {
        site_path = &site_path[..site_path.len() - 1];
    }
    let file_name = format!("{}.md", site_path.replace("/", "_"));
    Some((site_name, file_name))
}

async fn add_url_to_documentation(gcx: Arc<ARwLock<GlobalContext>>, url_str: String, depth: usize, max_pages: usize) -> Result<Vec<String>, String> {
    let mut visited_pages = HashSet::new();
    let mut queue = vec![];
    let mut sources = vec![];
    let mut pages = HashMap::default();
    let url = Url::parse(&url_str).map_err(|_| format!("Invalid url '{url_str}'"))?;
    let base_url = Url::parse(&url[..Position::BeforePath])
        .map_err(|_| format!("Unable to find base url of '{url_str}'"))?;

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
            let Some((dir_name, file_name)) = get_directory_and_file_from_url(&url) else {
                continue; // skip this url
            };
            let file_path = format!("./.refact/docs/{dir_name}/{file_name}");
            let directory = Path::new(&file_path).parent().unwrap();
            fs::create_dir_all(directory)
                .map_err(|e| format!("Unable to create directory {:?} {directory:?}: {e:?}", env::current_dir()))?;
            let mut file = File::create(&file_path)
                .map_err(|_| format!("Unable to create file {file_path}"))?;
            file.write_all(text.as_bytes())
                .map_err(|_| format!("Unable to write to file {file_path}"))?;
            pages.insert(url, file_path.clone());

            // vectorize file
            let gcx = gcx.write().await;
            let mut files = gcx.documents_state.documentation_files.lock().await;
            let vec_db_module = {
                *gcx.documents_state.cache_dirty.lock().await = true;
                gcx.vec_db.clone()
            };
            sources.push(file_path.clone());
            if !files.contains(&file_path) {
                files.push(file_path.clone());
            }
            let document = Document::new(&PathBuf::from(file_path.as_str()));
            match *vec_db_module.lock().await {
                Some(ref mut db) => db.vectorizer_enqueue_files(&vec![document], false).await,
                None => {}
            };

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
                    if !link.starts_with(&base_url.to_string()) {
                        return;
                    }
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

    let (directory, _) = get_directory_and_file_from_url(&url_str).unwrap();

    let origin = DocOrigin {
        url: url_str.clone(),
        max_depth: depth,
        max_pages,
        pages,
    };

    if let Ok(origin_json) = serde_json::to_string(&origin) {
        let file_path = format!("./.refact/docs/{directory}/origin.json");
        let mut file = File::create(&file_path)
            .map_err(|_| format!("Unable to create file {file_path}"))?;
        file.write_all(origin_json.as_bytes())
            .map_err(|_| format!("Unable to write to file {file_path}"))?;
    } else {
        warn!("Unable to convert DocOrigin to json");
    }

    Ok(sources)
}

async fn doc_sources_list(ccx: &mut AtCommandsContext, tool_call_id: &String) -> Result<Vec<ContextEnum>, String> {
    let sources = ccx
        .global_context
        .read()
        .await
        .documents_state
        .documentation_files
        .lock()
        .await
        .join(",");

    let results = vec![ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: format!("[{sources}]"),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
    })];
    Ok(results)
}

async fn doc_sources_add(ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
    let source = match args.get("source") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => return Err(format!("argument `source` is not a string: {:?}", v)),
        None => return Err("Missing source argument for doc_sources_add".to_string()),
    };

    // if the source is an url, download the page and convert it to markdown
    if source.starts_with("http://") || source.starts_with("https://") {
        task::spawn(add_url_to_documentation(ccx.global_context.clone(), source, 2, 3));
        let results = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "Started background task to add website to documentation, this may take a few minutes...".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })];
        Ok(results)
    } else {
        let abs_source_path = PathBuf::from(source.as_str());
        if fs::canonicalize(abs_source_path).is_err() {
            return Err(format!("File or directory '{}' doesn't exist", source));
        }

        let gcx = ccx.global_context.write().await;
        let mut files = gcx.documents_state.documentation_files.lock().await;
        let vec_db_module = {
            *gcx.documents_state.cache_dirty.lock().await = true;
            gcx.vec_db.clone()
        };

        if !files.contains(&source) {
            files.push(source.clone());
        }
        let document = Document::new(&PathBuf::from(source.as_str()));
        match *vec_db_module.lock().await {
            Some(ref mut db) => db.vectorizer_enqueue_files(&vec![document], false).await,
            None => {}
        };

        let results = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "Successfully added source to documentation list.".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        })];
        Ok(results)
    }
}
async fn doc_sources_remove(ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
    let source = match args.get("source") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => return Err(format!("argument `source` is not a string: {:?}", v)),
        None => return Err("Missing source argument for doc_sources_remove".to_string()),
    };

    let gc = ccx.global_context
        .write()
        .await;

    let mut files = gc
        .documents_state
        .documentation_files
        .lock()
        .await;

    let Some(i) = files.iter().position(|x| *x == source) else {
        return Err(format!("Unable to find '{}' in the documentation list", source));
    };
    files.remove(i);

    let results = vec![ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: "Succesfully removed source from the documentation list.".to_string(),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
    })];
    Ok(results)
}

#[async_trait]
impl AtTool for AttDocSources {
    async fn execute(
        &self,
        ccx: &mut AtCommandsContext,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<Vec<ContextEnum>, String> {
        let action = match args.get("action") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `action` is not a string: {:?}", v)),
            None => return Err("Missing `action` argument for doc_sources".to_string()),
        };
        match action.as_str() {
            "list" => doc_sources_list(ccx, tool_call_id).await,
            "add" => doc_sources_add(ccx, tool_call_id, args).await,
            "remove" => doc_sources_remove(ccx, tool_call_id, args).await,
            _ => Err(format!("Unknown action `{}`", action))
        }
    }
}
