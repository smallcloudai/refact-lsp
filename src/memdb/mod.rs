use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use parking_lot::Mutex as ParkMutex;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;


pub mod db_chore;
pub mod db_cmessage;
pub mod db_cthread;
pub mod db_init;
pub mod db_schema;
pub mod db_structs;
mod db_memories;


pub async fn memdb_pubsub_trigerred(
    gcx: Arc<ARwLock<GlobalContext>>,
    db: &Arc<ParkMutex<db_structs::MemDB>>,
    sleep_seconds: u64
) -> bool {
    let shutdown_flag: Arc<AtomicBool> = gcx.read().await.shutdown_flag.clone();
    if shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let sleeping_point = db.lock().memdb_sleeping_point.clone();
    match tokio::time::timeout(tokio::time::Duration::from_secs(sleep_seconds), sleeping_point.notified()).await {
        Ok(_) => { },
        Err(_) => { },   // timeout
    }
    let should_continue = !shutdown_flag.load(std::sync::atomic::Ordering::Relaxed);
    should_continue
}


pub fn merge_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    // if let serde_json::Value::Object(_) = b {
    //     tracing::info!("merging json:\n{:#?}", b);
    // }
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                // yay, it's recursive!
                merge_json(a.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (a, b) => {
            *a = b.clone();
        }
    }
}

