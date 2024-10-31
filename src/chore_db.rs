use std::sync::Arc;
use std::time::Instant;
use indexmap::IndexMap;
use parking_lot::Mutex as ParkMutex;
use tokio::task;
use serde::de::DeserializeOwned;
use serde::Serialize;
use rusqlite::{params, Connection};

use crate::chore_schema::{ChoreDB, Chore, ChoreEvent, ChatThread};
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
            cmessage_n INT NOT NULL,
            cmessage_json TEXT NOT NULL,
            PRIMARY KEY (cmessage_belongs_to_cthread_id, cmessage_n)
        )",
        [],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn cthread_get(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cthread_id: String,
) -> Option<ChatThread> {
    let db = cdb.lock();
    let conn = db.lite.lock();
    let mut stmt = conn.prepare("SELECT * FROM cthreads WHERE cthread_id = ?1").unwrap();
    let mut rows = match stmt.query(params![cthread_id]) {
        Ok(rows) => rows,
        Err(_) => return None,
    };
    if let Some(row) = rows.next().unwrap_or(None) {
        Some(ChatThread {
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
        })
    } else {
        None
    }
}

pub async fn cthread_set(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cthread: ChatThread,
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



// pub async fn chore_db_init(chore_db_fn: String) -> Arc<ParkMutex<ChoreDB>>
// {
//     let config = sled::Config::default()
//         .cache_capacity(2_000_000)
//         .use_compression(false)
//         .mode(sled::Mode::HighThroughput)
//         .flush_every_ms(Some(5000))
//         .path(chore_db_fn.clone());

//     tracing::info!("starting Chore DB, chore_db_fn={:?}", chore_db_fn);
//     let db: Arc<sled::Db> = Arc::new(task::spawn_blocking(
//         move || config.open().unwrap()
//     ).await.unwrap());
//     tracing::info!("/starting Chore DB");

//     Arc::new(ParkMutex::new(ChoreDB {
//         sleddb: db,
//     }))
// }


// // getters

// async fn _deserialize_json_from_sled<T: DeserializeOwned>(
//     db: Arc<sled::Db>,
//     key: &str,
// ) -> Option<T> {
//     match db.get(key.as_bytes()) {
//         Ok(Some(value)) => {
//             match serde_json::from_slice::<T>(value.as_ref()) {
//                 Ok(item) => Some(item),
//                 Err(e) => {
//                     tracing::error!("cannot deserialize {}:\n{}\n{}", std::any::type_name::<T>(), e, String::from_utf8_lossy(value.as_ref()));
//                     None
//                 }
//             }
//         },
//         Ok(None) => {
//             tracing::error!("cannot find key {:?}", key);
//             None
//         },
//         Err(e) => {
//             tracing::error!("cannot find key {:?}: {:?}", key, e);
//             None
//         }
//     }
// }

// pub async fn chore_get(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     chore_id: String,
// ) -> Option<Chore> {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_key = format!("chore|{}", chore_id);
//     _deserialize_json_from_sled::<Chore>(db, &chore_key).await
// }

// pub async fn chore_event_get(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     chore_event_id: String,
// ) -> Option<ChoreEvent> {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_event_key = format!("chore-event|{}", chore_event_id);
//     _deserialize_json_from_sled::<ChoreEvent>(db, &chore_event_key).await
// }

// pub async fn chat_thread_get(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     cthread_id: String,
// ) -> Option<ChatThread> {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chat_thread_key = format!("chat-thread|{}", cthread_id);
//     _deserialize_json_from_sled::<ChatThread>(db, &chat_thread_key).await
// }

// pub async fn chat_messages_load(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     cthread: &mut ChatThread
// ) {
//     let mut messages = Vec::new();
//     for i in 0 .. usize::MAX {
//         if let Some(message) = chat_message_get(cdb.clone(), cthread.cthread_id.clone(), i).await {
//             messages.push(message);
//         } else {
//             break;
//         }
//     }
//     cthread.cthread_messages = messages;
// }


// // setters

// async fn _serialize_json_to_sled<T: Serialize>(
//     db: Arc<sled::Db>,
//     key: &str,
//     value: &T,
// ) {
//     match serde_json::to_vec(value) {
//         Ok(serialized) => {
//             match db.insert(key.as_bytes(), serialized) {
//                 Ok(_) => (),
//                 Err(e) => {
//                     tracing::error!("Failed to insert into db: key={}, error={}", key, e);
//                 }
//             }
//         },
//         Err(e) => {
//             tracing::error!("Failed to serialize value: key={}, error={}", key, e);
//         }
//     }
// }

// pub async fn chore_set(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     chore: Chore,
// ) {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_key = format!("chore|{}", chore.chore_id);
//     _serialize_json_to_sled(db, &chore_key, &chore).await;
// }

// pub async fn chore_event_set(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     chore_event: ChoreEvent,
// ) {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_event_key = format!("chore-event|{}", chore_event.chore_event_id);
//     _serialize_json_to_sled(db, &chore_event_key, &chore_event).await;
// }


// pub async fn chat_thread_set(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     chat_thread: ChatThread,
// ) {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chat_thread_key = format!("chat-thread|{}", chat_thread.cthread_id);
//     _serialize_json_to_sled(db, &chat_thread_key, &chat_thread).await;
// }

// pub async fn chat_messages_save(
//     cdb: Arc<ParkMutex<ChoreDB>>,
//     cthread: &ChatThread,
// ) {
//     for (i, message) in cthread.cthread_messages.iter().enumerate() {
//         chat_message_set(cdb.clone(), cthread.cthread_id.clone(), i, message.clone()).await;
//     }
// }



// pub fn chore_new(
//     cdb: Arc<ParkMutex<ChoreStuff>>,
//     chore_id: String,  // generate random guid if empty
//     chore_title: String,
//     chore_spontaneous_work_enable: bool,
// ) -> Chore {
//     // TODO
//     return chore_get(chore_id);
// }

