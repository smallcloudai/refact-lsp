use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::info;
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;
use crate::global_context::{CommandLine, GlobalContext};
use tokio::sync::RwLock as ARwLock;
use tracing::error;
use crate::background_tasks::BackgroundTasksHolder;

use crate::vecdb::handler::{VecDBHandler, VecDBHandlerRef};
use crate::fetch_embedding::get_embedding;
use crate::vecdb::vectorizer_service::FileVectorizerService;
use crate::vecdb::structs::{SearchResult, VecdbSearch, VecDbStatus};


#[derive(Debug)]
pub struct VecDb {
    vecdb_handler: VecDBHandlerRef,
    retriever_service: Arc<AMutex<FileVectorizerService>>,
    cmdline: CommandLine,

    model_name: String,
    endpoint_template: String,
}


const VECDB_BACKGROUND_RELOAD_ON_SUCCESS: u64 = 1200;
const VECDB_BACKGROUND_RELOAD_ON_FAIL: u64 = 30;


pub async fn vecdb_background_reload(
    global_context: Arc<ARwLock<GlobalContext>>,
) {
    let mut background_tasks = BackgroundTasksHolder::new(vec![]);

    let mut vecdb_fetched = false;
    let mut first_loop = true;
    loop {
        if !first_loop {
            if vecdb_fetched {
                tokio::time::sleep(tokio::time::Duration::from_secs(VECDB_BACKGROUND_RELOAD_ON_SUCCESS)).await;
            } else {
                tokio::time::sleep(tokio::time::Duration::from_secs(VECDB_BACKGROUND_RELOAD_ON_FAIL)).await;
            }
        }
        background_tasks.abort().await;
        background_tasks = BackgroundTasksHolder::new(vec![]);

        vecdb_fetched = false;
        first_loop = false;
        info!("attempting to launch vecdb");

        let mut gcx_locked = global_context.write().await;
        let caps_mb = gcx_locked.caps.clone();
        if caps_mb.is_none() {
            info!("vecd launch failed: no caps");
            continue;
        }

        let cache_dir = &gcx_locked.cache_dir;
        let cmdline = &gcx_locked.cmdline;

        if !cmdline.vecdb {
            info!("VecDB launch is disabled by cmdline");
            vecdb_fetched = true;
            continue;
        }

        let (default_embeddings_model, endpoint_embeddings_template, endpoint_embeddings_style) = {
            let caps = caps_mb.unwrap();
            let caps_locked = caps.read().unwrap();
            (
                caps_locked.default_embeddings_model.clone(),
                caps_locked.endpoint_embeddings_template.clone(),
                caps_locked.endpoint_embeddings_style.clone(),
            )
        };

        if default_embeddings_model.is_empty() || endpoint_embeddings_template.is_empty() {
            info!("vecd launch failed: default_embeddings_model.is_empty() || endpoint_embeddings_template.is_empty()");
            continue;
        }

        let vecdb_mb = create_vecdb_if_caps_present(
            default_embeddings_model,
            endpoint_embeddings_template,
            endpoint_embeddings_style,
            cmdline,
            cache_dir
        ).await;

        if vecdb_mb.is_none() {
            info!("vecd launch failed: vecdb_mb.is_none()");
            continue;
        }
        gcx_locked.vec_db = Arc::new(AMutex::new(vecdb_mb));
        info!("VECDB is launched successfully");
        vecdb_fetched = true;

        background_tasks.extend(match *gcx_locked.vec_db.lock().await {
            Some(ref db) => db.start_background_tasks().await,
            None => vec![]
        });
    }
}


pub async fn create_vecdb_if_caps_present(
    default_embeddings_model: String,
    endpoint_embeddings_template: String,
    endpoint_embeddings_style: String,

    cmdline: &CommandLine,
    cache_dir: &PathBuf,
) -> Option<VecDb> {
    let vec_db = match VecDb::init(
        &cache_dir, cmdline.clone(),
        384, 60, 512, 1024,
        default_embeddings_model.clone(),
        endpoint_embeddings_template.clone(),
        endpoint_embeddings_style.clone(),
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
        cache_dir: &PathBuf,
        cmdline: CommandLine,
        embedding_size: i32,
        cooldown_secs: u64,
        splitter_window_size: usize,
        splitter_soft_limit: usize,

        model_name: String,
        endpoint_template: String,
        endpoint_embeddings_style: String,
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
            cmdline.api_key.clone(),
            endpoint_embeddings_style.clone(),
            endpoint_template.clone(),
        ).await));

        Ok(VecDb {
            vecdb_handler,
            retriever_service,
            cmdline: cmdline.clone(),

            model_name,
            endpoint_template,
        })
    }

    pub async fn start_background_tasks(&self) -> Vec<JoinHandle<()>> {
        info!("vecdb: start_background_tasks");
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
        let embedding_mb = get_embedding(
            &self.cmdline.address_url,
            &self.model_name,
            &self.endpoint_template,
            query.clone(),
            &self.cmdline.api_key,
        ).await;
        if embedding_mb.is_err() {
            return Err("Failed to get embedding".to_string());
        }
        let mut binding = self.vecdb_handler.lock().await;

        let results = binding.search(embedding_mb.unwrap(), top_n).await.unwrap();
        binding.update_record_statistic(results.clone()).await;
        Ok(
            SearchResult {
                query_text: query,
                results,
            }
        )
    }
}
