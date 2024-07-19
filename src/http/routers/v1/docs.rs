use crate::custom_error::ScratchError;
use crate::documentation_files::get_docs_dir;
use crate::files_in_workspace::Document;
use crate::global_context::GlobalContext;
use axum::http::{Response, StatusCode};
use axum::Extension;
use hashbrown::HashSet;
use html2text::render::text_renderer::{TaggedLine, TextDecorator};
use hyper::Body;
use itertools::Itertools;
use log::warn;
use select::predicate::{Attr, Name};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::mem::swap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{env, fs};
use tokio::sync::RwLock as ARwLock;
use tokio::task;
use url::{Position, Url};

#[derive(Serialize, Deserialize)]
pub struct DocOrigin {
    pub url: String,
    pub max_depth: usize,
    pub max_pages: usize,
    pub pages: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct DocsProps {
    source: String,
}

fn get_directory_and_file_from_url(url: &str) -> Option<(PathBuf, String)> {
    let url_without_http = url.split("://").nth(1).unwrap();
    let (site_name, mut site_path) = url_without_http.split_once("/")?;
    if site_path == "" {
        site_path = "index";
    }
    if site_path.ends_with("/") {
        site_path = &site_path[..site_path.len() - 1];
    }
    let file_name = format!("{}.md", site_path.replace("/", "_"));
    let mut site_dir = get_docs_dir();
    site_dir.push(site_name);
    Some((site_dir, file_name))
}

#[derive(Clone, Copy)]
struct CustomTextConversion;

impl TextDecorator for CustomTextConversion {
    type Annotation = ();

    fn decorate_link_start(&mut self, _url: &str) -> (String, Self::Annotation) {
        ("[".to_string(), ())
    }

    fn decorate_link_end(&mut self) -> String {
        "]".to_string()
    }

    fn decorate_em_start(&self) -> (String, Self::Annotation) {
        ("*".to_string(), ())
    }

    fn decorate_em_end(&self) -> String {
        "*".to_string()
    }

    fn decorate_strong_start(&self) -> (String, Self::Annotation) {
        ("**".to_string(), ())
    }

    fn decorate_strong_end(&self) -> String {
        "**".to_string()
    }

    fn decorate_strikeout_start(&self) -> (String, Self::Annotation) {
        ("".to_string(), ())
    }

    fn decorate_strikeout_end(&self) -> String {
        "".to_string()
    }

    fn decorate_code_start(&self) -> (String, Self::Annotation) {
        ("`".to_string(), ())
    }

    fn decorate_code_end(&self) -> String {
        "`".to_string()
    }

    fn decorate_preformat_first(&self) -> Self::Annotation {}
    fn decorate_preformat_cont(&self) -> Self::Annotation {}

    fn decorate_image(&mut self, _src: &str, title: &str) -> (String, Self::Annotation) {
        (format!("[{}]", title), ())
    }

    fn header_prefix(&self, level: usize) -> String {
        "#".repeat(level) + " "
    }

    fn quote_prefix(&self) -> String {
        "> ".to_string()
    }

    fn unordered_item_prefix(&self) -> String {
        "* ".to_string()
    }

    fn ordered_item_prefix(&self, i: i64) -> String {
        format!("{}. ", i)
    }

    fn make_subblock_decorator(&self) -> Self {
        *self
    }

    fn finalise(&mut self, _: Vec<String>) -> Vec<TaggedLine<()>> {
        vec![]
    }
}

fn find_content(html: String) -> String {
    let document = select::document::Document::from(html.as_str());

    let content_ids = vec![
        "content",
        "I_content",
        "main-content",
        "main_content",
        "CONTENT",
    ];
    for id in content_ids {
        if let Some(node) = document.find(Attr("id", id)).next() {
            return node.html();
        }
    }

    if let Some(node) = document.find(Name("article")).next() {
        return node.html();
    }

    if let Some(node) = document.find(Name("main")).next() {
        return node.html();
    }

    html
}

// returns a pair of html and markdown
pub async fn fetch_and_convert_to_md(url: &str) -> Result<(String, String), String> {
    let response = reqwest::get(url)
        .await
        .map_err(|_| format!("Unable to connect to '{url}'"))?;

    if !response.status().is_success() {
        return Err(format!("Unable to connect to '{url}'"));
    }

    let html = response
        .text()
        .await
        .map_err(|_| "Unable to convert page to text".to_string())?;

    let html = find_content(html);

    let md = html2text::config::with_decorator(CustomTextConversion)
        .string_from_read(&html.as_bytes()[..], 200)
        .map_err(|_| "Unable to convert html to text".to_string())?;

    Ok((html, md))
}

async fn add_url_to_documentation(
    gcx: Arc<ARwLock<GlobalContext>>,
    url_str: String,
    depth: usize,
    max_pages: usize,
) -> Result<DocOrigin, String> {
    let mut visited_pages = HashSet::new();
    let mut queue = vec![];
    let mut pages = HashMap::default();
    let url = Url::parse(&url_str).map_err(|_| format!("Invalid url '{url_str}'"))?;
    let base_url = Url::parse(&url[..Position::BeforePath])
        .map_err(|_| format!("Unable to find base url of '{url_str}'"))?;

    queue.push(url_str.to_string());
    visited_pages.insert(url_str.to_string());

    {
        let gcx = gcx.write().await;
        let mut doc_sources = gcx.documents_state.documentation_sources.lock().await;
        if !doc_sources.contains(&url_str) {
            doc_sources.push(url_str.clone());
        }
    }

    for iteration in 0..=depth {
        let mut new_urls = vec![];
        let is_last_iteration = iteration == depth;

        for url in queue {
            let Ok((html, text)) = fetch_and_convert_to_md(&url).await else {
                continue;
            };

            // create file
            let Some((dir_path, file_name)) = get_directory_and_file_from_url(&url) else {
                continue; // skip this url
            };
            let mut file_path = dir_path.clone();
            file_path.push(file_name);
            let directory = Path::new(&file_path).parent().unwrap();
            fs::create_dir_all(directory).map_err(|e| {
                format!(
                    "Unable to create directory {:?} {directory:?}: {e:?}",
                    env::current_dir()
                )
            })?;
            let mut file = File::create(&file_path)
                .map_err(|_| format!("Unable to create file {}", file_path.display()))?;
            file.write_all(text.as_bytes())
                .map_err(|_| format!("Unable to write to file {}", file_path.display()))?;
            pages.insert(url.clone(), format!("{}", file_path.display()));

            // vectorize file
            let gcx = gcx.write().await;
            let vec_db_module = {
                *gcx.documents_state.cache_dirty.lock().await = true;
                gcx.vec_db.clone()
            };
            let document = Document::new(&file_path);
            match *vec_db_module.lock().await {
                Some(ref mut db) => db.vectorizer_enqueue_files(&vec![document], false).await,
                None => {}
            };

            if is_last_iteration {
                continue;
            }

            // find links
            let url = Url::parse(url.as_str()).ok();
            let base_parser = Url::options().base_url(url.as_ref());
            select::document::Document::from(html.as_str())
                .find(Name("a"))
                .filter_map(|n| n.attr("href"))
                .for_each(|link| {
                    let Ok(mut link) = base_parser.parse(link) else {
                        return;
                    };
                    link.set_fragment(None);
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

    if let Ok(origin_json) = serde_json::to_string_pretty(&origin) {
        let mut file_path = directory.clone();
        file_path.push("origin.json");
        let mut file = File::create(&file_path)
            .map_err(|_| format!("Unable to create file {}", file_path.display()))?;
        file.write_all(origin_json.as_bytes())
            .map_err(|_| format!("Unable to write to file {}", file_path.display()))?;
    } else {
        warn!("Unable to convert DocOrigin to json");
    }

    Ok(origin)
}

pub async fn handle_v1_list_docs(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let sources = gcx
        .read()
        .await
        .documents_state
        .documentation_sources
        .lock()
        .await
        .clone();

    let body = serde_json::to_string_pretty(&sources).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("json problem: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

pub async fn handle_v1_add_docs(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let DocsProps { source } = serde_json::from_slice::<DocsProps>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    if source.starts_with("http://") || source.starts_with("https://") {
        task::spawn(add_url_to_documentation(gcx.clone(), source, 2, 40));
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("Started background task to add website to documentation, this may take a few minutes..."))
            .unwrap())
    } else {
        let abs_source_path = PathBuf::from(source.as_str());
        if fs::canonicalize(abs_source_path).is_err() {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Unable to find source file"))
                .unwrap());
        }

        let gcx = gcx.write().await;
        let mut files = gcx.documents_state.documentation_sources.lock().await;
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

        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(
                "Successfully added source to documentation list.",
            ))
            .unwrap())
    }
}

pub async fn handle_v1_remove_docs(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let DocsProps { source } = serde_json::from_slice::<DocsProps>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let gc = gcx.write().await;

    let mut files = gc.documents_state.documentation_sources.lock().await;

    let Some(i) = files.iter().position(|x| *x == source) else {
        return Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from(format!(
                "Unable to find '{}' in the documentation list",
                source
            )))
            .unwrap());
    };
    files.remove(i);

    if source.starts_with("http://") || source.starts_with("https://") {
        if let Some((dir, _)) = get_directory_and_file_from_url(&source) {
            if let Err(err) = fs::remove_dir_all(&dir)
                .map_err(|err| format!("Error while deleting directory '{}': {err}", dir.display()))
            {
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from(err))
                    .unwrap());
            }
        }
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(
            "Successfully removed source from the documentation list.",
        ))
        .unwrap())
}
