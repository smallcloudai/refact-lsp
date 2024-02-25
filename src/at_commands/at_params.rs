use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use async_trait::async_trait;
use itertools::Itertools;
use regex::Regex;
use strsim::{normalized_damerau_levenshtein, jaro_winkler};
use tokio::sync::RwLock as ARwLock;
use url::Url;

use crate::at_commands::at_commands::{AtCommandsContext, AtParam};
use crate::global_context::GlobalContext;

#[derive(Debug)]
pub struct AtParamFilePath {
    pub name: String,
}

impl AtParamFilePath {
    pub fn new() -> Self {
        Self {
            name: "file_path".to_string()
        }
    }
}

async fn get_vecdb_file_paths(global_context: Arc<ARwLock<GlobalContext>>) -> Vec<String> {
    let file_paths_from_memory = global_context.read().await.documents_state.document_map.read().await.keys().cloned().collect::<Vec<Url>>();

    let file_paths_from_vecdb = match *global_context.read().await.vec_db.lock().await {
        Some(ref db) => {
            let index_file_paths = db.get_indexed_file_paths().await;
            let index_file_paths = index_file_paths.lock().await.deref().clone();
            index_file_paths.iter().map(|f| f.to_str().unwrap().to_string()).collect()
        },
        None => vec![]
    };

    let index_file_paths: Vec<_> = file_paths_from_memory.into_iter()
        .filter_map(|f|f.to_file_path().map(|x|Some(x)).unwrap_or(None))
        .filter_map(|x|x.to_str().map(|x|Some(x.to_string())).unwrap_or(None))
        .chain(file_paths_from_vecdb.into_iter())
        .collect::<HashSet<_>>() // dedup
        .into_iter()
        .collect();

    index_file_paths
}

async fn get_ast_file_paths(global_context: Arc<ARwLock<GlobalContext>>) -> Vec<String> {
     match *global_context.read().await.ast_module.lock().await {
        Some(ref ast) => {
            let index_file_paths = ast.get_indexed_file_paths().await;
            index_file_paths.iter().map(|f| f.to_str().unwrap().to_string()).collect()
        },
        None => vec![]
    }
}

fn put_parsed_args_back_to_arg(value: &mut String, parsed_args: &HashMap<String, String>) {
    if let Some(start_line) = parsed_args.get("file_start_line") {
        let end_line_mb = parsed_args.get("file_end_line");
        *value = if end_line_mb.is_some() {
            format!("{}:{}-{}", value, start_line, end_line_mb.unwrap())
        } else {
            format!("{}:{}", value, start_line)
        };
    }
}

#[async_trait]
impl AtParam for AtParamFilePath {
    fn name(&self) -> &String {
        &self.name
    }
    async fn is_value_valid(&self, value: &mut String, context: &AtCommandsContext) -> (bool, Option<HashMap<String, String>>) {
        let parsed_args = self.parse_args_from_arg(value);
        let is_valid = get_vecdb_file_paths(context.global_context.clone()).await.contains(value);
        (is_valid, parsed_args)
    }
    async fn complete(&self, value: &String, context: &AtCommandsContext, top_n: usize) -> Vec<String> {
        let mut value = value.clone();
        let parsed_args_mb = self.parse_args_from_arg(&mut value);
        let index_file_paths = get_vecdb_file_paths(context.global_context.clone()).await;

        let mapped_paths = index_file_paths.iter().map(|f| {
            let path = PathBuf::from(f);
            (
                f,
                // TODO: paths are too long, so calculations for refact-lsp take ~150ms. jaro-winkler is 25ms but is not good enough
                normalized_damerau_levenshtein(
                    if value.starts_with("/") {
                        f
                    } else {
                        path.file_name().unwrap().to_str().unwrap()
                    },
                    &value.to_string(),
                )
            )
        });

        let mut sorted_paths = mapped_paths
            .sorted_by(|(_, dist1), (_, dist2)| dist1.partial_cmp(dist2).unwrap())
            .rev()
            .map(|(path, _)| path.clone())
            .take(top_n)
            .collect::<Vec<String>>();

        if let Some(parsed_args) = parsed_args_mb {
            sorted_paths = sorted_paths.iter_mut().map(|x| { put_parsed_args_back_to_arg(x, &parsed_args); x.clone() }).collect::<Vec<String>>();
        }
        sorted_paths
    }
    fn complete_if_valid(&self) -> bool {
        false
    }
    fn parse_args_from_arg(&self, value: &mut String) -> Option<HashMap<String, String>> {
        let re = Regex::new(r":(\d+)(?:-(\d+))?$").unwrap();
        if let Some(captures) = re.captures(value) {
            let mut results = HashMap::new();
            if let Some(start) = captures.get(1) {
                results.insert("file_start_line".to_string(), start.as_str().to_string());
            }
            if let Some(end) = captures.get(2) {
                results.insert("file_end_line".to_string(), end.as_str().to_string());
            }
            *value = re.replace(value, "").to_string();
            Some(results)
        } else {
            None
        }
    }
}


#[derive(Debug)]
pub struct AtParamFilePathWithRow {
    pub name: String,
}

impl AtParamFilePathWithRow {
    pub fn new() -> Self {
        Self {
            name: "file_path".to_string()
        }
    }
}

#[async_trait]
impl AtParam for AtParamFilePathWithRow {
    fn name(&self) -> &String {
        &self.name
    }
    async fn is_value_valid(&self, value: &mut String, context: &AtCommandsContext) -> (bool, Option<HashMap<String, String>>) {
        let mut parts = value.split(":");
        let file_path = match parts.next().ok_or("_") {
            Ok(res) => res,
            Err(_) => {
                return (false, None)
            }
        };
        let row_idx_str = match parts.next().ok_or("_") {
            Ok(res) => res,
            Err(_) => {
                return (false, None)
            }
        };
        // TODO: check if the row exists
        let row_idx: usize = match row_idx_str.parse() {
            Ok(res) => res,
            Err(_) => {
                return (false, None)
            }
        };
        let paths = get_ast_file_paths(context.global_context.clone()).await;
        (paths.contains(&file_path.to_string()), None)
    }

    async fn complete(&self, value: &String, context: &AtCommandsContext, top_n: usize) -> Vec<String> {
        let index_file_paths = get_ast_file_paths(context.global_context.clone()).await;

        let mapped_paths = index_file_paths.iter().map(|f| {
            let path = PathBuf::from(f);
            (
                f,
                normalized_damerau_levenshtein(
                    if value.starts_with("/") {
                        f
                    } else {
                        path.file_name().unwrap().to_str().unwrap()
                    },
                    &value.to_string(),
                )
            )
        });

        let sorted_paths = mapped_paths
            .sorted_by(|(_, dist1), (_, dist2)| dist1.partial_cmp(dist2).unwrap())
            .rev()
            .map(|(path, _)| path.clone())
            .take(top_n)
            .collect::<Vec<String>>();
        sorted_paths
    }
    fn complete_if_valid(&self) -> bool {
        false
    }
}


#[derive(Debug)]
pub struct AtParamSymbolPathQuery {
    pub name: String,
}

impl AtParamSymbolPathQuery {
    pub fn new() -> Self {
        Self {
            name: "context_file".to_string()
        }
    }
}

#[async_trait]
impl AtParam for AtParamSymbolPathQuery {
    fn name(&self) -> &String {
        &self.name
    }
    async fn is_value_valid(&self, _: &mut String, _: &AtCommandsContext) -> (bool, Option<HashMap<String, String>>) {
        (true, None)
    }
    async fn complete(&self, value: &String, context: &AtCommandsContext, top_n: usize) -> Vec<String> {
        let ast_module_ptr = context.global_context.read().await.ast_module.clone();
        let index_paths = match *ast_module_ptr.lock().await {
            Some(ref ast) => ast.get_indexed_symbol_paths().await,
            None => vec![]
        };

        let value_lower = value.to_lowercase();
        let mapped_paths = index_paths
            .iter()
            .filter(|x| x.to_lowercase().contains(&value_lower))
            .map(|f| {
                let filename = f.split("::").dropping(1).into_iter().join("::");
                (
                    f,
                    jaro_winkler(
                        if value.starts_with("/") {
                            f
                        } else {
                            &filename
                        },
                        &value.to_string(),
                    )
                )
            });
        let sorted_paths = mapped_paths
            .sorted_by(|(_, dist1), (_, dist2)| dist1.partial_cmp(dist2).unwrap())
            .rev()
            .map(|(path, _)| path.clone())
            .take(top_n)
            .collect::<Vec<String>>();
        return sorted_paths;
    }
    fn complete_if_valid(&self) -> bool {
        true
    }
}


#[derive(Debug)]
pub struct AtParamSymbolReferencePathQuery {
    pub name: String,
}

impl AtParamSymbolReferencePathQuery {
    pub fn new() -> Self {
        Self {
            name: "context_file".to_string()
        }
    }
}

#[async_trait]
impl AtParam for AtParamSymbolReferencePathQuery {
    fn name(&self) -> &String {
        &self.name
    }
    async fn is_value_valid(&self, _: &mut String, _: &AtCommandsContext) -> (bool, Option<HashMap<String, String>>) {
        (true, None)
    }
    async fn complete(&self, value: &String, context: &AtCommandsContext, top_n: usize) -> Vec<String> {
        let ast_module_ptr = context.global_context.read().await.ast_module.clone();
        let index_paths = match *ast_module_ptr.lock().await {
            Some(ref ast) => ast.get_indexed_references().await,
            None => vec![]
        };
        let value_lower = value.to_lowercase();
        let mapped_paths = index_paths
            .iter()
            .filter(|x| x.to_lowercase().contains(&value_lower))
            .map(|f| {
                let filename = f.split("::").dropping(1).into_iter().join("::");
                (
                    f,
                    jaro_winkler(
                        if value.starts_with("/") {
                            f
                        } else {
                            &filename
                        },
                        &value.to_string(),
                    )
                )
            });
        let sorted_paths = mapped_paths
            .sorted_by(|(_, dist1), (_, dist2)| dist1.partial_cmp(dist2).unwrap())
            .rev()
            .map(|(path, _)| path.clone())
            .take(top_n)
            .collect::<Vec<String>>();
        return sorted_paths;
    }
    fn complete_if_valid(&self) -> bool {
        true
    }
}
