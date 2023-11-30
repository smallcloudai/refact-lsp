use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::vecdb::hadler::{VecDBHandler, VecDBHandlerRef};
use crate::vecdb::req_client::get_embedding;
use crate::vecdb::retriever_service::RetrieverService;
use crate::vecdb::structs::{SearchResult, VecDbStatus};

#[derive(Debug)]
pub struct VecDb {
    vecdb_handler: VecDBHandlerRef,
    retriever_service: Arc<RwLock<RetrieverService>>,
    embedding_model_name: String,
    top_n: usize,
}


impl VecDb {
    pub async fn new(
        cache_dir: PathBuf,
        embedding_size: i32,
        cooldown_secs: u64,
        splitter_window_size: usize,
        embedding_model_name: String,
        top_n: usize,
    ) -> Self {
        let vecdb_handler = Arc::new(RwLock::new(VecDBHandler::init(
            cache_dir, embedding_size,
        ).await));
        let retriever_service = Arc::new(RwLock::new(RetrieverService::new(
            vecdb_handler.clone(), cooldown_secs, splitter_window_size, embedding_model_name.clone(),
        ).await));

        VecDb {
            vecdb_handler,
            retriever_service,
            embedding_model_name,
            top_n,
        }
    }

    pub async fn add_or_update_file(&mut self, file_path: PathBuf, force: bool) {
        self.retriever_service.write().unwrap().process_file(file_path, force).await;
    }

    pub async fn add_or_update_files(&self, file_paths: Vec<PathBuf>, force: bool) {
        self.retriever_service.write().unwrap().process_files(file_paths, force).await;
    }

    pub async fn remove_file(&self, file_path: &PathBuf) {
        self.vecdb_handler.write().unwrap().remove(file_path).await;
    }

    pub async fn search(&self, query: String) -> Result<SearchResult, String> {
        let embedding = get_embedding(query.clone(), &self.embedding_model_name).await.unwrap();
        match embedding {
            Ok(vector) => {
                let results = self.vecdb_handler.read().unwrap().search(vector, self.top_n).await.unwrap();
                Ok(
                    SearchResult {
                        query_text: query,
                        results: results,
                        db_status: self.get_status().await,
                    }
                )
            }
            Err(_) => {
                return Err("Failed to get embedding".to_string());
            }
        }
    }

    pub async fn get_status(&self) -> VecDbStatus {
        self.retriever_service.read().unwrap().status().await
    }
}
