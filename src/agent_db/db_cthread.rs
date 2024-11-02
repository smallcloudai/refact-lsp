use std::sync::Arc;
use std::time::Instant;
use indexmap::IndexMap;
use parking_lot::Mutex as ParkMutex;
use tokio::task;
use serde::de::DeserializeOwned;
use serde::Serialize;
use rusqlite::{params, Connection};

use crate::agent_db::db_structs::{ChoreDB, Chore, ChoreEvent, CThread, CMessage};
use crate::call_validation::ChatMessage;


pub fn cthread_get(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cthread_id: String,
) -> Result<CThread, String> {
    let db = cdb.lock();
    let conn = db.lite.lock();
    let mut stmt = conn.prepare("SELECT * FROM cthreads WHERE cthread_id = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query(params![cthread_id])
        .map_err(|e| e.to_string())?;
    let mut cthreads = cthreads_from_rows(rows);
    cthreads.pop().ok_or_else(|| format!("No CThread found with id: {}", cthread_id))
}

pub fn cthreads_from_rows(
    mut rows: rusqlite::Rows,
) -> Vec<CThread> {
    let mut cthreads = Vec::new();
    while let Some(row) = rows.next().unwrap_or(None) {
        cthreads.push(CThread {
            cthread_id: row.get("cthread_id").unwrap(),
            cthread_belongs_to_chore_event_id: row.get::<_, Option<String>>("cthread_belongs_to_chore_event_id").unwrap(),
            cthread_title: row.get("cthread_title").unwrap(),
            cthread_toolset: row.get("cthread_toolset").unwrap(),
            cthread_model_used: row.get("cthread_model_used").unwrap(),
            cthread_error: row.get("cthread_error").unwrap(),
            cthread_anything_new: row.get("cthread_anything_new").unwrap(),
            cthread_created_ts: row.get("cthread_created_ts").unwrap(),
            cthread_updated_ts: row.get("cthread_updated_ts").unwrap(),
            cthread_archived_ts: row.get("cthread_archived_ts").unwrap(),
        });
    }
    cthreads
}

pub fn cthread_set(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cthread: CThread,
) {
    let db = cdb.lock();
    let conn = db.lite.lock();
    // sqlite dialect "INSERT OR REPLACE INTO"
    // mysql has INSERT INTO .. ON DUPLICATE KEY UPDATE ..
    // postgres has INSERT INTO .. ON CONFLICT .. DO UPDATE SET
    conn.execute(
        "INSERT OR REPLACE INTO cthreads (
            cthread_id,
            cthread_belongs_to_chore_event_id,
            cthread_title,
            cthread_toolset,
            cthread_model_used,
            cthread_error,
            cthread_anything_new,
            cthread_created_ts,
            cthread_updated_ts,
            cthread_archived_ts
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            cthread.cthread_id,
            cthread.cthread_belongs_to_chore_event_id,
            cthread.cthread_title,
            cthread.cthread_toolset,
            cthread.cthread_model_used,
            cthread.cthread_error,
            cthread.cthread_anything_new,
            cthread.cthread_created_ts,
            cthread.cthread_updated_ts,
            cthread.cthread_archived_ts,
        ],
    ).expect("Failed to insert or replace chat thread");
}
