use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::SystemTime;

use tokio::task::JoinHandle;

use crate::vecdb::file_splitter::FileSplitter;
use crate::vecdb::handler::VecDBHandlerRef;
use crate::vecdb::req_client::get_embedding;
use crate::vecdb::structs::{Record, SplitResult, VecDbStatus, VecDbStatusRef};

#[derive(Debug)]
pub struct RetrieverService {
    update_request_queue: Arc<Mutex<VecDeque<PathBuf>>>,
    output_queue: Arc<Mutex<VecDeque<PathBuf>>>,
    vecdb_handler: VecDBHandlerRef,
    cooldown_queue_thread_handle: Option<thread::JoinHandle<()>>,
    cooldown_queue_thread_end_flag: Arc<AtomicBool>,
    retrieve_thread_handle: Option<thread::JoinHandle<()>>,
    retrieve_thread_end_flag: Arc<AtomicBool>,
    status: VecDbStatusRef,
}

fn cooldown_queue_thread(
    queue: Arc<Mutex<VecDeque<PathBuf>>>,
    out_queue: Arc<Mutex<VecDeque<PathBuf>>>,
    end_flag: Arc<AtomicBool>,
    status: VecDbStatusRef,
    cooldown_secs: u64,
) {
    let mut last_updated: HashMap<PathBuf, SystemTime> = HashMap::new();
    loop {
        if end_flag.load(Ordering::SeqCst) {
            break;
        }

        let Some(path) = queue.lock().unwrap().pop_front() else { continue; };
        status.lock().unwrap().unprocessed_chunk_count = queue.lock().unwrap().len();
        last_updated.insert(path, SystemTime::now());
        let mut paths_to_process: Vec<PathBuf> = Vec::new();
        for (path, time) in &last_updated {
            if time.elapsed().unwrap().as_secs() > cooldown_secs {
                paths_to_process.push(path.clone());
            }
        }
        for path in paths_to_process {
            last_updated.remove(&path);
            out_queue.lock().unwrap().push_back(path);
        }

        thread::sleep(std::time::Duration::from_millis(100));
    }
}


fn retrieve_thread(
    queue: Arc<Mutex<VecDeque<PathBuf>>>,
    vecdb_handler_ref: VecDBHandlerRef,
    end_flag: Arc<AtomicBool>,
    status: VecDbStatusRef,
    splitter_window_size: usize,
    embedding_model_name: String,
    api_key: String,
) {
    let file_splitter = FileSplitter::new(splitter_window_size);
    let runtime = tokio::runtime::Handle::current();

    loop {
        if end_flag.load(Ordering::SeqCst) {
            break;
        }

        let path = {
            let mut queue = queue.lock().unwrap();
            match queue.pop_front() {
                Some(path) => path,
                None => continue,
            }
        };

        let splat_data = file_splitter.split(&path);
        let vecdb_handler = runtime.block_on(vecdb_handler_ref.lock());
        let splat_data_filtered: Vec<SplitResult> = splat_data
            .iter()
            .filter(|x| vecdb_handler.contains(&x.window_text_hash))
            .cloned() // Clone to avoid borrowing issues
            .collect();
        drop(vecdb_handler);

        let join_handles: Vec<_> = splat_data_filtered.iter().map(
            |x| get_embedding(x.window_text.clone(), &embedding_model_name, api_key.clone())
        ).collect();

        let mut splat_join_data: VecDeque<(SplitResult, JoinHandle<Result<Vec<f32>, String>>)>
            = splat_data_filtered.into_iter()
            .zip(join_handles.into_iter())
            .collect::<VecDeque<_>>();


        let mut records: Vec<Record> = Vec::new();
        while let Some((data_res, handle)) = splat_join_data.pop_front() {
            if end_flag.load(Ordering::SeqCst) {
                break;
            }

            if !handle.is_finished() {
                splat_join_data.push_back((data_res, handle));
                continue;
            }

            match runtime.block_on(handle) {
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
                            score: 1.0
                        }
                    );
                }
                _ => {}
            }
        }

        runtime.block_on(vecdb_handler_ref.lock()).add_or_update(records);
        thread::sleep(std::time::Duration::from_millis(25));
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
        let cooldown_queue = Arc::new(Mutex::new(VecDeque::new()));
        let output_queue = Arc::new(Mutex::new(VecDeque::new()));
        let cooldown_queue_thread_end_flag = Arc::new(AtomicBool::new(false));
        let retrieve_thread_end_flag = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(
            VecDbStatus {
                unprocessed_files_count: 0,
                unprocessed_chunk_count: 0,
                requests_count: 0,
                db_size: 0,
                db_last_time_updated: SystemTime::now(),
            }
        ));
        let output_queue_clone_1 = output_queue.clone();
        let output_queue_clone_2 = output_queue.clone();
        let status_clone_1 = status.clone();
        let status_clone_2 = status.clone();
        RetrieverService {
            update_request_queue: cooldown_queue.clone(),
            output_queue: output_queue.clone(),
            vecdb_handler: vecdb_handler.clone(),
            cooldown_queue_thread_end_flag: cooldown_queue_thread_end_flag.clone(),
            retrieve_thread_end_flag: retrieve_thread_end_flag.clone(),
            cooldown_queue_thread_handle: Option::from(
                thread::spawn(move || {
                    cooldown_queue_thread(
                        cooldown_queue.clone(),
                        output_queue_clone_1.clone(),
                        cooldown_queue_thread_end_flag.clone(),
                        status_clone_1,
                        cooldown_secs,
                    )
                })
            ),
            retrieve_thread_handle: Option::from(
                thread::spawn(move || {
                    retrieve_thread(
                        output_queue_clone_2.clone(),
                        vecdb_handler.clone(),
                        retrieve_thread_end_flag.clone(),
                        status_clone_2.clone(),
                        splitter_window_size,
                        embedding_model_name,
                        api_key,
                    )
                })
            ),
            status: status.clone(),
        }
    }

    pub async fn process_file(&self, path: PathBuf, force: bool) {
        if !force {
            self.update_request_queue.lock().unwrap().push_back(path);
        } else {
            self.output_queue.lock().unwrap().push_back(path);
        }
    }

    pub async fn process_files(&self, paths: Vec<PathBuf>, force: bool) {
        if !force {
            self.update_request_queue.lock().unwrap().extend(paths);
        } else {
            self.output_queue.lock().unwrap().extend(paths);
        }
    }

    pub async fn status(&self) -> VecDbStatus {
        self.status.lock().unwrap().clone()
    }
}

impl Drop for RetrieverService {
    fn drop(&mut self) {
        self.cooldown_queue_thread_end_flag.store(true, Ordering::SeqCst);
        self.retrieve_thread_end_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.cooldown_queue_thread_handle.take() {
            handle.join().unwrap();
        }
        if let Some(handle) = self.retrieve_thread_handle.take() {
            handle.join().unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::*;

    struct VecDbStatus {
        pub unprocessed_chunk_count: usize,
    }

    type VecDbStatusRef = Arc<Mutex<VecDbStatus>>;

    // Helper function to create a test environment
    fn setup_test_env() -> (Arc<Mutex<VecDeque<PathBuf>>>, Arc<Mutex<VecDeque<PathBuf>>>, Arc<AtomicBool>, VecDbStatusRef, u64) {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let out_queue = Arc::new(Mutex::new(VecDeque::new()));
        let end_flag = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(VecDbStatus { unprocessed_chunk_count: 0 }));
        let cooldown_secs = 1; // 1 second for quick testing

        (queue, out_queue, end_flag, status, cooldown_secs)
    }
}
