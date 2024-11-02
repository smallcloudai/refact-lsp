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


pub fn create_tables_20241102(conn: &rusqlite::Connection, reset_memory: bool) -> Result<(), String> {
    if reset_memory {
        conn.execute("DROP TABLE IF EXISTS cthreads", []).map_err(|e| e.to_string())?;
        conn.execute("DROP TABLE IF EXISTS cmessage", []).map_err(|e| e.to_string())?;
    }
    conn.execute(
        "CREATE TABLE IF NOT EXISTS cthreads (
            cthread_id TEXT PRIMARY KEY,
            cthread_belongs_to_chore_event_id TEXT DEFAULT NULL,
            cthread_title TEXT NOT NULL,
            cthread_toolset TEXT NOT NULL,
            cthread_model_used TEXT NOT NULL,
            cthread_error TEXT NOT NULL,
            cthread_anything_new BOOLEAN NOT NULL,
            cthread_created_ts REAL NOT NULL,
            cthread_updated_ts REAL NOT NULL,
            cthread_archived_ts REAL NOT NULL
        )",
        [],
    ).map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS cmessage (
            cmessage_belongs_to_cthread_id TEXT NOT NULL,
            cmessage_alt INT NOT NULL,
            cmessage_num INT NOT NULL,
            cmessage_prev_alt INT NOT NULL,
            cmessage_usage_model TEXT NOT NULL,
            cmessage_usage_prompt TEXT NOT NULL,
            cmessage_usage_completion TEXT NOT NULL,
            cmessage_json TEXT NOT NULL,
            PRIMARY KEY (cmessage_belongs_to_cthread_id, cmessage_alt, cmessage_num)
        )",
        [],
    ).map_err(|e| e.to_string())?;
    Ok(())
}
