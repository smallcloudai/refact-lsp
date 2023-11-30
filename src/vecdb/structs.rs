use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VecDbStatus {
    pub unprocessed_chunk_count: usize,
    pub requests_count: usize,
    pub db_size: usize,
    pub db_last_time_updated: SystemTime,
}

pub type VecDbStatusRef = Arc<Mutex<VecDbStatus>>;


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Record {
    pub vector: Vec<f32>,
    pub window_text: String,
    pub window_text_hash: String,
    pub file_path: PathBuf,
    pub start_line: u64,
    pub end_line: u64,
    pub time_added: SystemTime,
    pub model_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct SplitResult {
    pub file_path: PathBuf,
    pub window_text: String,
    pub window_text_hash: String,
    pub start_line: u64,
    pub end_line: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub query_text: String,
    pub results: Vec<Record>,
    pub db_status: VecDbStatus,
}
