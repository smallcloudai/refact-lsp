use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::info;

use crate::vecdb::file_splitter::FileSplitter;
use crate::vecdb::handler::VecDBHandlerRef;
use crate::vecdb::req_client::get_embedding;
use crate::vecdb::structs::{Record, SplitResult, VecDbStatus, VecDbStatusRef};

#[derive(Debug)]
pub struct RetrieverService {
    update_request_queue: Arc<Mutex<VecDeque<PathBuf>>>,
    output_queue: Arc<Mutex<VecDeque<PathBuf>>>,
    vecdb_handler: VecDBHandlerRef,
    status: VecDbStatusRef,
    cooldown_secs: u64,
    splitter_window_size: usize,
    embedding_model_name: String,
    api_key: String,
}

async fn cooldown_queue_thread(
    update_request_queue: Arc<Mutex<VecDeque<PathBuf>>>,
    out_queue: Arc<Mutex<VecDeque<PathBuf>>>,
    status: VecDbStatusRef,
    cooldown_secs: u64,
) {
    let mut last_updated: HashMap<PathBuf, SystemTime> = HashMap::new();
    loop {
        let (path_maybe, unprocessed_chunk_count) = {
            let mut queue_locked = update_request_queue.lock().await;
            if !queue_locked.is_empty() {
                (Some(queue_locked.pop_front().unwrap()), queue_locked.len())
            } else {
                (None, 0)
            }
        };
        status.lock().await.unprocessed_chunk_count = unprocessed_chunk_count;

        if let Some(path) = path_maybe {
            last_updated.insert(path, SystemTime::now());
        }

        let mut paths_to_process: Vec<PathBuf> = Vec::new();
        for (path, time) in &last_updated {
            if time.elapsed().unwrap().as_secs() > cooldown_secs {
                paths_to_process.push(path.clone());
            }
        }
        for path in paths_to_process {
            last_updated.remove(&path);
            out_queue.lock().await.push_back(path);
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}


async fn retrieve_thread(
    queue: Arc<Mutex<VecDeque<PathBuf>>>,
    vecdb_handler_ref: VecDBHandlerRef,
    status: VecDbStatusRef,
    splitter_window_size: usize,
    embedding_model_name: String,
    api_key: String,
) {
    let file_splitter = FileSplitter::new(splitter_window_size);

    loop {
        let path = {
            match queue.lock().await.pop_front() {
                Some(path) => path,
                None => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    continue;
                }
            }
        };
        info!("Processing file: {}", path.display());

        let splat_data = match file_splitter.split(&path).await {
            Ok(data) => data,
            Err(e) => { continue }
        };
        let vecdb_handler = vecdb_handler_ref.lock().await;
        let splat_data_filtered: Vec<SplitResult> = splat_data
            .iter()
            .filter(|x| !vecdb_handler.contains(&x.window_text_hash))
            .cloned() // Clone to avoid borrowing issues
            .collect();
        drop(vecdb_handler);
        info!("Retrieving embeddings for {} chunks", splat_data_filtered.len());

        let join_handles: Vec<_> = splat_data_filtered.iter().map(
            |x| get_embedding(x.window_text.clone(), &embedding_model_name, api_key.clone())
        ).collect();

        let mut splat_join_data: VecDeque<(SplitResult, JoinHandle<Result<Vec<f32>, String>>)>
            = splat_data_filtered.into_iter()
            .zip(join_handles.into_iter())
            .collect::<VecDeque<_>>();

        let mut records: Vec<Record> = Vec::new();
        while let Some((data_res, handle)) = splat_join_data.pop_front() {
            match handle.await {
                Ok(Ok(result)) => {
                    records.push(
                        Record {
                            vector: result,
                            window_text: data_res.window_text,
                            window_text_hash: data_res.window_text_hash,
                            file_path: data_res.file_path,
                            start_line: data_res.start_line,
                            end_line: data_res.end_line,
                            time_added: SystemTime::now(),
                            model_name: embedding_model_name.clone(),
                            score: 1.0,
                        }
                    );
                }
                Ok(Err(e)) => {
                    info!("Error retrieving embeddings for {}: {}", data_res.window_text, e);
                }
                Err(e) => { continue; }
            }
        }
        match vecdb_handler_ref.lock().await.add_or_update(records).await {
            Err(e) => {
                info!("Error adding/updating records in VecDB: {}", e);
            }
            _ => {}
        }
    }
}

impl RetrieverService {
    pub async fn new(
        vecdb_handler: VecDBHandlerRef,
        cooldown_secs: u64,
        splitter_window_size: usize,
        embedding_model_name: String,
        api_key: String,
    ) -> Self {
        let update_request_queue = Arc::new(Mutex::new(VecDeque::new()));
        let output_queue = Arc::new(Mutex::new(VecDeque::new()));
        let status = Arc::new(Mutex::new(
            VecDbStatus {
                unprocessed_files_count: 0,
                unprocessed_chunk_count: 0,
                requests_count: 0,
                db_size: 0,
                db_last_time_updated: SystemTime::now(),
            }
        ));
        RetrieverService {
            update_request_queue: update_request_queue.clone(),
            output_queue: output_queue.clone(),
            vecdb_handler: vecdb_handler.clone(),
            status: status.clone(),
            cooldown_secs,
            splitter_window_size,
            embedding_model_name,
            api_key,
        }
    }

    pub async fn start_background_tasks(&self) -> Vec<JoinHandle<()>> {
        let cooldown_queue_join_handle = tokio::spawn(
            cooldown_queue_thread(
                self.update_request_queue.clone(),
                self.output_queue.clone(),
                self.status.clone(),
                self.cooldown_secs,
            )
        );

        let retrieve_thread_handle = tokio::spawn(
            retrieve_thread(
                self.output_queue.clone(),
                self.vecdb_handler.clone(),
                self.status.clone(),
                self.splitter_window_size,
                self.embedding_model_name.clone(),
                self.api_key.clone(),
            )
        );

        return vec![cooldown_queue_join_handle, retrieve_thread_handle];
    }

    pub async fn process_file(&self, path: PathBuf, force: bool) {
        if !force {
            self.update_request_queue.lock().await.push_back(path);
        } else {
            self.output_queue.lock().await.push_back(path);
        }
    }

    pub async fn process_files(&self, paths: Vec<PathBuf>, force: bool) {
        if !force {
            self.update_request_queue.lock().await.extend(paths);
        } else {
            self.output_queue.lock().await.extend(paths);
        }
    }

    pub async fn status(&self) -> VecDbStatus {
        self.status.lock().await.clone()
    }
}
