use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use tokio::task::JoinHandle;
use tracing::{error, info};
use tree_sitter::Point;

use crate::background_tasks::BackgroundTasksHolder;
use crate::global_context::{CommandLine, GlobalContext};
use crate::lsp::ast_index::AstIndex;
use crate::lsp::ast_index_service::AstIndexService;
use crate::lsp::ast_search_engine::AstSearchEngine;
use crate::lsp::structs::SearchResult;

pub struct AstModule {
    ast_index_service: Arc<AMutex<AstIndexService>>,
    ast_index: Arc<AMutex<AstIndex>>,
    ast_search_engine: Arc<AMutex<AstSearchEngine>>,
    cmdline: CommandLine,
}

#[derive(Debug, Serialize)]
pub struct VecDbCaps {
    functions: Vec<String>,
}


impl AstModule {
    pub async fn init(
        cmdline: CommandLine,
    ) -> Result<AstModule, String> {
        let ast_index = match AstIndex::init().await {
            Ok(res) => Arc::new(AMutex::new(res)),
            Err(err) => { return Err(err); }
        };
        let ast_search_engine = match AstSearchEngine::init(ast_index.clone()).await {
            Ok(res) => Arc::new(AMutex::new(res)),
            Err(err) => { return Err(err); }
        };
        let ast_index_service = match AstIndexService::init(ast_index.clone()).await {
            Ok(res) => Arc::new(AMutex::new(res)),
            Err(err) => { return Err(err); }
        };

        Ok(AstModule {
            ast_index_service,
            ast_index,
            ast_search_engine,
            cmdline,
        })
    }

    pub async fn start_background_tasks(&self) -> Vec<JoinHandle<()>> {
        info!("vecdb: start_background_tasks");
        return self.ast_index_service.lock().await.start_background_tasks().await;
    }

    pub async fn add_or_update_file(&self, file_path: PathBuf, force: bool) {
        self.ast_index_service.lock().await.process_file(file_path, force).await;
    }

    pub async fn add_or_update_files(&self, file_paths: Vec<PathBuf>, force: bool) {
        self.ast_index_service.lock().await.process_files(file_paths, force).await;
    }

    pub async fn remove_file(&self, file_path: &PathBuf) {
        self.ast_index.lock().await.remove(file_path).await;
    }

    async fn search(
        &mut self,
        filename: &PathBuf,
        cursor: Point,
        top_n: usize
    ) -> Result<SearchResult, String> {
        let t0 = std::time::Instant::now();

        let mut handler_locked = self.ast_search_engine.lock().await;
        let results = match handler_locked.search(query, filename, cursor).await {
            Ok(res) => res,
            Err(_) => { return Err("error during search occurred".to_string()); }
        };
        for rec in results.iter() {
            let last_30_chars: String = rec.file_path.display().to_string().chars().rev().take(30).collect::<String>().chars().rev().collect();
            info!("distance {:.3}, found ...{}:{}-{}, ", rec.distance, last_30_chars, rec.start_line, rec.end_line);
        }
        info!("ast search query {:?}, took {:.3}s", query, t0.elapsed().as_secs_f64());

        Ok(
            SearchResult {
                query_text: query.to_string(),
                filename: filename.clone(),
                cursor: cursor,
                search_results: results,
            }
        )
    }
}
