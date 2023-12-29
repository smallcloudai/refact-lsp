use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::info;
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;
use crate::global_context::GlobalContext;
use tokio::sync::RwLock as ARwLock;
use tracing::error;

use crate::vecdb::handler::{VecDBHandler, VecDBHandlerRef};
use crate::fetch_embedding::get_embedding;
use crate::vecdb::vectorizer_service::FileVectorizerService;
use crate::vecdb::structs::{SearchResult, VecdbSearch, VecDbStatus};

#[derive(Debug)]
pub struct VecDb {
    vecdb_handler: VecDBHandlerRef,
    retriever_service: Arc<AMutex<FileVectorizerService>>,
    cmdline: crate::global_context::CommandLine,

    model_name: String,
    api_key: String,
    cloud_name: String,
    endpoint_template: String,
}


pub async fn create_vecdb_if_caps_present(gcx: Arc<ARwLock<GlobalContext>>) -> Option<VecDb> {
    let gcx_locked = gcx.read().await;
    let cache_dir = gcx_locked.cache_dir.clone();
    let cmdline = gcx_locked.cmdline.clone();

    let caps_mb = gcx_locked.caps.clone();
    if caps_mb.is_none() {
        return None;
    }
    let caps = caps_mb.unwrap();
    let caps_locked = caps.read().unwrap();
    // info!("caps {:?}", caps_locked);

    if !cmdline.vecdb {
        info!("VecDB is disabled by cmdline");
        return None;
    }
    if caps_locked.default_embeddings_model.is_empty() {
        info!("no default_embeddings_model in caps");
        return None;
    }
    if caps_locked.endpoint_embeddings_template.is_empty() {
        info!("endpoint_embeddings_template is empty");
        return None
    }

    let vec_db = match VecDb::init(
        cache_dir, cmdline.clone(),
        384, 60, 512, 1024,
        caps_locked.default_embeddings_model.clone(),
        cmdline.api_key.clone(),
        caps_locked.cloud_name.clone(),
        caps_locked.endpoint_template.clone(),
    ).await {
        Ok(res) => Some(res),
        Err(err) => {
            error!("Ooops database is broken!
                Last error message: {}
                You can report this issue here:
                https://github.com/smallcloudai/refact-lsp/issues
                Also, you can run this to erase your db:
                `rm -rf ~/.cache/refact/refact_vecdb_cache`
                After that restart this LSP server or your IDE.", err);
            None
        }
    };
    vec_db
}


impl VecDb {
    pub async fn init(
        cache_dir: PathBuf,
        cmdline: crate::global_context::CommandLine,
        embedding_size: i32,
        cooldown_secs: u64,
        splitter_window_size: usize,
        splitter_soft_limit: usize,

        model_name: String,
        api_key: String,
        cloud_name: String,
        endpoint_template: String,
    ) -> Result<VecDb, String> {
        let handler = match VecDBHandler::init(cache_dir, embedding_size).await {
            Ok(res) => res,
            Err(err) => { return Err(err) }
        };
        let vecdb_handler = Arc::new(AMutex::new(handler));
        let retriever_service = Arc::new(AMutex::new(FileVectorizerService::new(
            vecdb_handler.clone(),
            cooldown_secs,
            splitter_window_size,
            splitter_soft_limit,

            model_name.clone(),
            api_key.clone(),
            cloud_name.clone(),
            endpoint_template.clone(),
        ).await));

        Ok(VecDb {
            vecdb_handler,
            retriever_service,
            cmdline,

            model_name,
            api_key,
            cloud_name,
            endpoint_template,
        })
    }

    pub async fn start_background_tasks(&self) -> Vec<JoinHandle<()>> {
        return self.retriever_service.lock().await.start_background_tasks().await;
    }

    pub async fn add_or_update_file(&mut self, file_path: PathBuf, force: bool) {
        self.retriever_service.lock().await.process_file(file_path, force).await;
    }

    pub async fn add_or_update_files(&self, file_paths: Vec<PathBuf>, force: bool) {
        self.retriever_service.lock().await.process_files(file_paths, force).await;
    }

    pub async fn remove_file(&self, file_path: &PathBuf) {
        self.vecdb_handler.lock().await.remove(file_path).await;
    }

    pub async fn get_status(&self) -> Result<VecDbStatus, String> {
        self.retriever_service.lock().await.status().await
    }
}


#[async_trait]
impl VecdbSearch for VecDb {
    async fn search(&self, query: String, top_n: usize) -> Result<SearchResult, String> {
        let embedding = get_embedding(
            &self.cloud_name,
            &self.model_name,
            &self.endpoint_template,
            query.clone(),
            &self.api_key,
        ).await.unwrap();
        match embedding {
            Ok(vector) => {
                let mut binding = self.vecdb_handler.lock().await;
                let results = binding.search(vector, top_n).await.unwrap();
                binding.update_record_statistic(results.clone()).await;
                Ok(
                    SearchResult {
                        query_text: query,
                        results: results,
                    }
                )
            }
            Err(_) => {
                return Err("Failed to get embedding".to_string());
            }
        }
    }
}
