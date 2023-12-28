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
    embedding_model_name: String,
    cmdline: crate::global_context::CommandLine,
    endpoint_embedding: String,
    provider_embedding: String,
}


fn resolve_endpoint_embeddings_url(
    endpoint_embeddings_template: &String,
    vecdb_provider: &String,
    embeddings_default_model: &String,
) -> String {
    return if vecdb_provider == "hf" {
        format!("{}/models/{}", "https://api-inference.huggingface.co".to_string(), embeddings_default_model)
    } else if vecdb_provider == "Refact" {
        endpoint_embeddings_template.clone()
    } else if vecdb_provider == "openai" {
        "".to_string()
    } else {
        "".to_string()
    }
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
    if cmdline.vecdb_provider.is_empty() || cmdline.vecdb_api_key.is_empty() {
        info!("vecdb_provider or vecdb_api_key is empty");
        return None
    }
    let endpoint_embeddings_url = resolve_endpoint_embeddings_url(
        &caps_locked.endpoint_embeddings_template,
        &cmdline.vecdb_provider,
        &caps_locked.default_embeddings_model,
    );
    if endpoint_embeddings_url.is_empty() {
        info!("endpoint_embeddings_url is empty");
        return None
    }

    let vec_db = match VecDb::init(
        cache_dir, cmdline.clone(),
        384, 60, 512, 1024,
        caps_locked.default_embeddings_model.clone(),
        endpoint_embeddings_url,
        cmdline.vecdb_provider.clone(),
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
        embedding_model_name: String,
        endpoint_embedding: String,
        provider_embedding: String,
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
            embedding_model_name.clone(),
            cmdline.vecdb_api_key.clone(),
            provider_embedding.clone(),
            endpoint_embedding.clone(),
        ).await));

        Ok(VecDb {
            vecdb_handler,
            retriever_service,
            embedding_model_name,
            cmdline,
            endpoint_embedding,
            provider_embedding,
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
            &self.provider_embedding,
            &self.embedding_model_name,
            &self.endpoint_embedding,
            query.clone(),
            &self.cmdline.vecdb_api_key,
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
