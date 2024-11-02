use std::sync::Arc;
use std::time::Instant;
use indexmap::IndexMap;
use parking_lot::Mutex as ParkMutex;
use tokio::task;
use serde::de::DeserializeOwned;
use serde::Serialize;
use rusqlite::{params, Connection};

use crate::chore_schema::{ChoreDB, Chore, ChoreEvent, CThread, CMessage};
use crate::call_validation::ChatMessage;


fn _chore_db_init(
    chore_db_fn: String,
) -> Result<Arc<ParkMutex<ChoreDB>>, String> {
    let db = Connection::open_with_flags(
        "experimental_db.sqlite",
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
    ).map_err(|err| format!("Failed to open database: {}", err))?;
    db.busy_timeout(std::time::Duration::from_secs(30)).map_err(|err| format!("Failed to set busy timeout: {}", err))?;
    db.execute_batch("PRAGMA cache_size = 0; PRAGMA shared_cache = OFF;").map_err(|err| format!("Failed to set cache pragmas: {}", err))?;
    let journal_mode: String = db.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0)).map_err(|err| format!("Failed to set journal mode: {}", err))?;
    if journal_mode != "wal" {
        return Err(format!("Failed to set WAL journal mode. Current mode: {}", journal_mode));
    }

    let db = ChoreDB {
        lite: Arc::new(ParkMutex::new(db)),
    };
    // db._permdb_create_table(reset_memory)?;
    Ok(Arc::new(ParkMutex::new(db)))
}

pub async fn chore_db_init(
    chore_db_fn: String,
) -> Arc<ParkMutex<ChoreDB>> {
    let db = match _chore_db_init(chore_db_fn) {
        Ok(db) => db,
        Err(err) => panic!("Failed to initialize chore database: {}", err),
    };
    let lite_arc = {
        db.lock().lite.clone()
    };
    _create_tables(&*lite_arc.lock(), false).expect("Failed to create tables");
    db
}

fn _create_tables(conn: &rusqlite::Connection, reset_memory: bool) -> Result<(), String> {
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


pub fn cmessages_from_rows(
    mut rows: rusqlite::Rows,
) -> Vec<CMessage> {
    let mut cmessages = Vec::new();
    while let Some(row) = rows.next().unwrap_or(None) {
        cmessages.push(CMessage {
            cmessage_belongs_to_cthread_id: row.get("cmessage_belongs_to_cthread_id").unwrap(),
            cmessage_alt: row.get("cmessage_alt").unwrap(),
            cmessage_num: row.get("cmessage_num").unwrap(),
            cmessage_prev_alt: row.get("cmessage_prev_alt").unwrap(),
            cmessage_usage_model: row.get("cmessage_usage_model").unwrap(),
            cmessage_usage_prompt: row.get("cmessage_usage_prompt").unwrap(),
            cmessage_usage_completion: row.get("cmessage_usage_completion").unwrap(),
            cmessage_json: row.get("cmessage_json").unwrap(),
        });
    }
    cmessages
}

pub fn cmessage_set(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cmessage: CMessage,
) {
    let db = cdb.lock();
    let conn = db.lite.lock();
    conn.execute(
        "INSERT OR REPLACE INTO cmessage (
            cmessage_belongs_to_cthread_id,
            cmessage_alt,
            cmessage_num,
            cmessage_prev_alt,
            cmessage_usage_model,
            cmessage_usage_prompt,
            cmessage_usage_completion,
            cmessage_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            cmessage.cmessage_belongs_to_cthread_id,
            cmessage.cmessage_alt,
            cmessage.cmessage_num,
            cmessage.cmessage_prev_alt,
            cmessage.cmessage_usage_model,
            cmessage.cmessage_usage_prompt,
            cmessage.cmessage_usage_completion,
            cmessage.cmessage_json,
        ],
    ).expect("Failed to insert or replace chat message");
}

pub fn cmessage_get(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cmessage_belongs_to_cthread_id: String,
    cmessage_alt: i32,
    cmessage_num: i32,
) -> Result<CMessage, String> {
    let db = cdb.lock();
    let conn = db.lite.lock();
    let mut stmt = conn.prepare(
        "SELECT * FROM cmessage WHERE cmessage_belongs_to_cthread_id = ?1 AND cmessage_alt = ?2 AND cmessage_num = ?3"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query(params![cmessage_belongs_to_cthread_id, cmessage_alt, cmessage_num])
        .map_err(|e| e.to_string())?;
    let cmessages = cmessages_from_rows(rows);
    cmessages.into_iter().next()
        .ok_or_else(|| format!("No CMessage found with {}:{}:{}", cmessage_belongs_to_cthread_id, cmessage_alt, cmessage_num))
}