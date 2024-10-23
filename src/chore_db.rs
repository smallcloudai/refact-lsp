use std::sync::Arc;
use std::time::Instant;
// use std::collections::{HashMap, HashSet};
use indexmap::IndexMap;
use tokio::sync::Mutex as AMutex;
use tokio::task;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::chore_schema::{ChoreDB, Chore, ChoreEvent, ChatThread};
use crate::call_validation::ChatMessage;

// How this database works:
//
// chore|<chore_id>                        value=json<Chore>
// chore-event|<chore_event_id>            value=json<ChoreEvent>
// chat-thread|<cthread_id>                value=json<ChatThread>
// chat-thread-messages|<cthread_id>/000   value=json<ChatMessage>


pub async fn chore_db_init(chore_db_fn: String) -> Arc<AMutex<ChoreDB>>
{
    let config = sled::Config::default()
        .cache_capacity(2_000_000)
        .use_compression(false)
        .mode(sled::Mode::HighThroughput)
        .flush_every_ms(Some(5000))
        .path(chore_db_fn.clone());

    tracing::info!("starting Chore DB, chore_db_fn={:?}", chore_db_fn);
    let db: Arc<sled::Db> = Arc::new(task::spawn_blocking(
        move || config.open().unwrap()
    ).await.unwrap());
    tracing::info!("/starting Chore DB");

    Arc::new(AMutex::new(ChoreDB {
        sleddb: db,
    }))
}


// getters

async fn _deserialize_json_from_sled<T: DeserializeOwned>(
    db: Arc<sled::Db>,
    key: &str,
) -> Option<T> {
    match db.get(key.as_bytes()) {
        Ok(Some(value)) => {
            match serde_json::from_slice::<T>(value.as_ref()) {
                Ok(item) => Some(item),
                Err(e) => {
                    tracing::error!("cannot deserialize {}:\n{}\n{}", std::any::type_name::<T>(), e, String::from_utf8_lossy(value.as_ref()));
                    None
                }
            }
        },
        Ok(None) => {
            tracing::error!("cannot find key {:?}", key);
            None
        },
        Err(e) => {
            tracing::error!("cannot find key {:?}: {:?}", key, e);
            None
        }
    }
}

pub async fn chore_get(
    cdb: Arc<AMutex<ChoreDB>>,
    chore_id: String,
) -> Option<Chore> {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chore_key = format!("chore|{}", chore_id);
    _deserialize_json_from_sled::<Chore>(db, &chore_key).await
}

pub async fn chore_event_get(
    cdb: Arc<AMutex<ChoreDB>>,
    chore_event_id: String,
) -> Option<ChoreEvent> {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chore_event_key = format!("chore-event|{}", chore_event_id);
    _deserialize_json_from_sled::<ChoreEvent>(db, &chore_event_key).await
}

pub async fn chat_message_get(
    cdb: Arc<AMutex<ChoreDB>>,
    cthread_id: String,
    i: usize,
) -> Option<ChatMessage> {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chat_message_key = format!("chat-thread-messages|{}/{:03}", cthread_id, i);
    _deserialize_json_from_sled::<ChatMessage>(db, &chat_message_key).await
}

pub async fn chat_thread_get(
    cdb: Arc<AMutex<ChoreDB>>,
    cthread_id: String,
) -> Option<ChatThread> {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chat_thread_key = format!("chat-thread|{}", cthread_id);
    _deserialize_json_from_sled::<ChatThread>(db, &chat_thread_key).await
}

pub async fn chat_messages_load(
    cdb: Arc<AMutex<ChoreDB>>,
    cthread: &mut ChatThread
) {
    let mut messages = Vec::new();
    for i in 0 .. usize::MAX {
        if let Some(message) = chat_message_get(cdb.clone(), cthread.cthread_id.clone(), i).await {
            messages.push(message);
        } else {
            break;
        }
    }
    cthread.cthread_messages = messages;
}


// setters

async fn _serialize_json_to_sled<T: Serialize>(
    db: Arc<sled::Db>,
    key: &str,
    value: &T,
) {
    match serde_json::to_vec(value) {
        Ok(serialized) => {
            match db.insert(key.as_bytes(), serialized) {
                Ok(_) => (),
                Err(e) => {
                    tracing::error!("Failed to insert into db: key={}, error={}", key, e);
                }
            }
        },
        Err(e) => {
            tracing::error!("Failed to serialize value: key={}, error={}", key, e);
        }
    }
}

pub async fn chore_set(
    cdb: Arc<AMutex<ChoreDB>>,
    chore: Chore,
) {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chore_key = format!("chore|{}", chore.chore_id);
    _serialize_json_to_sled(db, &chore_key, &chore).await;
}

pub async fn chore_event_set(
    cdb: Arc<AMutex<ChoreDB>>,
    chore_event: ChoreEvent,
) {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chore_event_key = format!("chore-event|{}", chore_event.chore_event_id);
    _serialize_json_to_sled(db, &chore_event_key, &chore_event).await;
}

pub async fn chat_message_set(
    cdb: Arc<AMutex<ChoreDB>>,
    cthread_id: String,
    i: usize,
    message: ChatMessage,
) {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chat_message_key = format!("chat-thread-messages|{}/{:03}", cthread_id, i);
    _serialize_json_to_sled(db, &chat_message_key, &message).await;
}

pub async fn chat_thread_set(
    cdb: Arc<AMutex<ChoreDB>>,
    chat_thread: ChatThread,
) {
    let db: Arc<sled::Db> = cdb.lock().await.sleddb.clone();
    let chat_thread_key = format!("chat-thread|{}", chat_thread.cthread_id);
    _serialize_json_to_sled(db, &chat_thread_key, &chat_thread).await;
}

pub async fn chat_messages_save(
    cdb: Arc<AMutex<ChoreDB>>,
    cthread: &ChatThread,
) {
    for (i, message) in cthread.cthread_messages.iter().enumerate() {
        chat_message_set(cdb.clone(), cthread.cthread_id.clone(), i, message.clone()).await;
    }
}



// pub fn chore_new(
//     cdb: Arc<AMutex<ChoreStuff>>,
//     chore_id: String,  // generate random guid if empty
//     chore_title: String,
//     chore_spontaneous_work_enable: bool,
// ) -> Chore {
//     // TODO
//     return chore_get(chore_id);
// }

