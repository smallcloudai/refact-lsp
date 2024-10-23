use std::sync::Arc;
use std::time::Instant;
// use std::collections::{HashMap, HashSet};
use indexmap::IndexMap;
use tokio::sync::Mutex as AMutex;
use tokio::task;
// use serde_cbor;
use sled::Db;
// use lazy_static::lazy_static;

use crate::chore_schema::{ChoreDB, Chore, ChoreEvent, ChatThread};

// How this database works:
//
// chore|<chore_id>                        value=json<Chore>
// chore-event|<chore_event_id>            value=json<ChoreEvent>
// chat-thread|<cthread_id>                value=json<ChatThread>
// chat-thread-messages|<cthread_id>/000   value=json<ChatMessage>


pub async fn chore_db_init(chore_db_fn: String) -> Arc<AMutex<ChoreDB>>
{
    let mut config = sled::Config::default()
        .cache_capacity(2_000_000)
        .use_compression(false)
        .mode(sled::Mode::HighThroughput)
        .flush_every_ms(Some(5000));
    config = config.path(chore_db_fn.clone());

    tracing::info!("starting Chore DB, chore_db_fn={:?}", chore_db_fn);
    let db: Arc<Db> = Arc::new(task::spawn_blocking(
        move || config.open().unwrap()
    ).await.unwrap());
    tracing::info!("/starting Chore DB");
    let ast_index = ChoreDB {
        sleddb: db,
    };
    Arc::new(AMutex::new(ast_index))
}

pub async fn chore_get(
    cs: Arc<AMutex<ChoreDB>>,
    chore_id: String,
    chore_spontaneous_work_enable: bool,
) -> Option<Chore> {
    let db = cs.lock().await.sleddb.clone();
    let chore_key = format!("chore|{}", chore_id);
    match db.get(chore_key.as_bytes()) {
        Ok(Some(value)) => {
            serde_json::from_slice::<Chore>(value.as_ref()).ok()
        },
        Ok(None) => None,
        Err(e) => {
            tracing::error!("cannot find key {:?}: {:?}", chore_key, e);
            None
        }
    }
}

// pub fn chore_new(
//     cs: Arc<AMutex<ChoreStuff>>,
//     chore_id: String,  // generate random guid if empty
//     chore_title: String,
//     chore_spontaneous_work_enable: bool,
// ) -> Chore {
//     // TODO
//     return chore_get(chore_id);
// }


// pub struct ChoreEvent {
//     pub chore_event_id: String,
//     pub chore_event_summary: String,
//     pub chore_event_ts: f64,
//     pub chore_event_link: Option<String>,
//     pub chore_event_cthread: ChatThread,
// }

// pub struct ChatThread {
//     pub chat_thread_id: String,
//     pub chat_thread_messages: Vec<ChatMessage>,
//     pub chat_thread_title: String,
//     pub chat_thread_toolset: String,      // quick/explore/agent
//     pub chat_thread_model_used: String,
//     pub chat_thread_error: String,        // assign to special value "pause" to avoid auto repost to the model
//     pub chat_thread_anything_new: bool,   // the ⚪
//     pub chat_thread_created_ts: f64,
//     pub chat_thread_updated_ts: f64,
//     pub chat_thread_archived_ts: f64,     // associated container died, cannot continue
// }
