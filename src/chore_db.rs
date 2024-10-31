use std::sync::Arc;
use std::time::Instant;
use indexmap::IndexMap;
use tokio::sync::Mutex as AMutex;
use parking_lot::Mutex as ParkMutex;
use tokio::task;
use serde::de::DeserializeOwned;
use serde::Serialize;
use rusqlite::{params, Connection};

use crate::chore_schema::{ChoreDB, Chore, ChoreEvent, ChatThread};
use crate::call_validation::ChatMessage;


fn _chore_db_init(
    chore_db_fn: String,
) -> Result<Arc<AMutex<ChoreDB>>, String> {
    let db = Connection::open_with_flags(
        chore_db_fn,
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
    Ok(Arc::new(AMutex::new(db)))
}

pub async fn chore_db_init(
    chore_db_fn: String,
) -> Arc<AMutex<ChoreDB>> {
    match _chore_db_init(chore_db_fn) {
        Ok(db) => db,
        Err(err) => panic!("Failed to initialize chore database: {}", err),
    }
}

pub async fn cthread_get(
    cdb: Arc<AMutex<ChoreDB>>,
    cthread_id: String,
) -> Option<ChatThread> {
    // let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    // let chat_message_key = format!("chat-thread-messages|{}/{:03}", cthread_id, i);
    // _deserialize_json_from_sled::<ChatMessage>(db, &chat_message_key).await
    return Some(ChatThread::default());
}

pub async fn cthread_set(
    cdb: Arc<AMutex<ChoreDB>>,
    cthread: ChatThread,
) {
    // let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    // let chat_message_key = format!("chat-thread-messages|{}/{:03}", cthread_id, i);
    // _serialize_json_to_sled(db, &chat_message_key, &message).await;
}


// fn _permdb_create_table(&self, reset_memory: bool) -> Result<(), String> {
//     let conn = self.conn.lock();
//     if reset_memory {
//         conn.execute("DROP TABLE IF EXISTS memories", []).map_err(|e| e.to_string())?;
//     }
//     conn.execute(
//         "CREATE TABLE IF NOT EXISTS memories (
//             memid TEXT PRIMARY KEY,
//             m_type TEXT NOT NULL,
//             m_goal TEXT NOT NULL,
//             m_project TEXT NOT NULL,
//             m_payload TEXT NOT NULL,
//             mstat_correct REAL NOT NULL DEFAULT 0,
//             mstat_relevant REAL NOT NULL DEFAULT 0,
//             mstat_times_used INTEGER NOT NULL DEFAULT 0
//         )",
//         [],
//     ).map_err(|e| e.to_string())?;
//     Ok(())
// }

// pub async fn chore_db_init(chore_db_fn: String) -> Arc<AMutex<ChoreDB>>
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

//     Arc::new(AMutex::new(ChoreDB {
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
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore_id: String,
// ) -> Option<Chore> {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_key = format!("chore|{}", chore_id);
//     _deserialize_json_from_sled::<Chore>(db, &chore_key).await
// }

// pub async fn chore_event_get(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore_event_id: String,
// ) -> Option<ChoreEvent> {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_event_key = format!("chore-event|{}", chore_event_id);
//     _deserialize_json_from_sled::<ChoreEvent>(db, &chore_event_key).await
// }

// pub async fn chat_thread_get(
//     cdb: Arc<AMutex<ChoreDB>>,
//     cthread_id: String,
// ) -> Option<ChatThread> {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chat_thread_key = format!("chat-thread|{}", cthread_id);
//     _deserialize_json_from_sled::<ChatThread>(db, &chat_thread_key).await
// }

// pub async fn chat_messages_load(
//     cdb: Arc<AMutex<ChoreDB>>,
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
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore: Chore,
// ) {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_key = format!("chore|{}", chore.chore_id);
//     _serialize_json_to_sled(db, &chore_key, &chore).await;
// }

// pub async fn chore_event_set(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore_event: ChoreEvent,
// ) {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chore_event_key = format!("chore-event|{}", chore_event.chore_event_id);
//     _serialize_json_to_sled(db, &chore_event_key, &chore_event).await;
// }


// pub async fn chat_thread_set(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chat_thread: ChatThread,
// ) {
//     let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
//     let chat_thread_key = format!("chat-thread|{}", chat_thread.cthread_id);
//     _serialize_json_to_sled(db, &chat_thread_key, &chat_thread).await;
// }

// pub async fn chat_messages_save(
//     cdb: Arc<AMutex<ChoreDB>>,
//     cthread: &ChatThread,
// ) {
//     for (i, message) in cthread.cthread_messages.iter().enumerate() {
//         chat_message_set(cdb.clone(), cthread.cthread_id.clone(), i, message.clone()).await;
//     }
// }



// pub fn chore_new(
//     cdb: Arc<AMutex<ChoreStuff>>,
//     chore_id: String,  // generate random guid if empty
//     chore_title: String,
//     chore_spontaneous_work_enable: bool,
// ) -> Chore {
//     // TODO
//     return chore_get(chore_id);
// }

