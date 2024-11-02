use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tokio::time::{interval, Duration};
use parking_lot::Mutex as ParkMutex;
use serde_json::json;
use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;
use async_stream::stream;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::agent_db::db_cthread::{cthread_get, cthreads_from_rows, cthread_set};
use crate::agent_db::db_cmessage::{cmessage_get, cmessages_from_rows, cmessage_set};
use crate::agent_db::db_structs::{CThread, CMessage};


pub async fn handle_db_v1_cthread_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let cdb = gcx.read().await.chore_db.clone();

    let incoming_json: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cthread_id = incoming_json.get("cthread_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let mut cthread_rec = if !cthread_id.is_empty() {
        cthread_get(cdb.clone(), cthread_id.clone()).unwrap_or_default()
    } else {
        CThread::default()
    };
    let mut chat_thread_json = serde_json::to_value(&cthread_rec).unwrap();
    _merge_json(&mut chat_thread_json, &incoming_json);

    cthread_rec = serde_json::from_value(chat_thread_json).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("Deserialization error: {}", e))
    })?;

    cthread_set(cdb, cthread_rec);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"status": "success"}).to_string()))
        .unwrap();

    Ok(response)
}

fn _merge_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                // yay, it's recursive!
                _merge_json(a.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (a, b) => {
            *a = b.clone();
        }
    }
}

#[derive(Deserialize)]
struct CThreadSubscription {
    #[serde(default)]
    quicksearch: String,
    #[serde(default)]
    limit: usize,
}

pub async fn handle_db_v1_cthreads_sub(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let mut post: CThreadSubscription = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    if post.limit == 0 {
        post.limit = 100;
    }

    let cdb = gcx.read().await.chore_db.clone();
    let lite_arc = cdb.lock().lite.clone();

    let (pre_existing_cthreads, mut last_event_id) = {
        let mut conn = lite_arc.lock();
        let tx = conn.transaction().unwrap();

        let query = if post.quicksearch.is_empty() {
            "SELECT * FROM cthreads ORDER BY cthread_id LIMIT ?"
        } else {
            "SELECT * FROM cthreads WHERE cthread_title LIKE ? ORDER BY cthread_id LIMIT ?"
        };
        let mut stmt = tx.prepare(query).unwrap();
        let rows = if post.quicksearch.is_empty() {
            stmt.query(rusqlite::params![post.limit])
        } else {
            stmt.query(rusqlite::params![format!("%{}%", post.quicksearch), post.limit])
        }.map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Query error: {}", e))
        })?;
        let cthreads = cthreads_from_rows(rows);

        let max_event_id: i64 = tx.query_row(
            "SELECT COALESCE(MAX(pubevent_id), 0) FROM pubsub_events",
            [],
            |row| row.get(0)
        ).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get max event ID: {}", e))
        })?;

        (cthreads, max_event_id)
    };

    let sse = stream! {
        for cthread in pre_existing_cthreads {
            let e = json!({
                "sub_event": "cthread_update",
                "cthread_rec": cthread
            });
            yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&e).unwrap()));
        }
        let mut interval = interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            match _cthread_poll_sub(lite_arc.clone(), &mut last_event_id) {
                Ok((deleted_cthread_ids, updated_cthread_ids)) => {
                    for deleted_id in deleted_cthread_ids {
                        let delete_event = json!({
                            "sub_event": "cthread_delete",
                            "cthread_id": deleted_id,
                        });
                        yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&delete_event).unwrap()));
                    }
                    for updated_id in updated_cthread_ids {
                        if let Ok(updated_cthread) = cthread_get(cdb.clone(), updated_id) {
                            let update_event = json!({
                                "sub_event": "cthread_update",
                                "cthread_rec": updated_cthread
                            });
                            yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&update_event).unwrap()));
                        }
                    }
                },
                Err(e) => {
                    tracing::error!("Error polling cthreads: {:?}", e);
                    // yield an error event to the client here?
                    break;
                }
            }
        }
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-cache")
        .body(Body::wrap_stream(sse))
        .unwrap();

    Ok(response)
}

fn _cthread_poll_sub(
    lite_arc: Arc<ParkMutex<rusqlite::Connection>>,
    seen_id: &mut i64
) -> Result<(Vec<String>, Vec<String>), String> {
    let conn = lite_arc.lock();
    let mut stmt = conn.prepare("
        SELECT pubevent_id, pubevent_action, pubevent_json
        FROM pubsub_events
        WHERE pubevent_id > ?1
        AND pubevent_channel = 'cthread'
        AND (pubevent_action = 'update' OR pubevent_action = 'delete')
        ORDER BY pubevent_id ASC
    ").unwrap();
    let mut rows = stmt.query([*seen_id]).map_err(|e| format!("Failed to execute query: {}", e))?;
    let mut deleted_cthread_ids = Vec::new();
    let mut updated_cthread_ids = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("Failed to fetch row: {}", e))? {
        let id: i64 = row.get(0).map_err(|e| format!("Failed to get pubevent_id: {}", e))?;
        let action: String = row.get(1).map_err(|e| format!("Failed to get pubevent_action: {}", e))?;
        let json: String = row.get(2).map_err(|e| format!("Failed to get pubevent_json: {}", e))?;
        let parsed_json: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;
        let cthread_id = parsed_json["cthread_id"].as_str()
            .ok_or_else(|| "Missing cthread_id in JSON".to_string())?
            .to_string();
        match action.as_str() {
            "delete" => deleted_cthread_ids.push(cthread_id),
            "update" => updated_cthread_ids.push(cthread_id),
            _ => return Err(format!("Unknown action: {}", action)),
        }
        *seen_id = id;
    }
    Ok((deleted_cthread_ids, updated_cthread_ids))
}


#[derive(Deserialize)]
struct CMessagesSubscription {
    cmessage_belongs_to_cthread_id: String,
}

pub async fn handle_db_v1_cmessages_sub(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: CMessagesSubscription = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cdb = gcx.read().await.chore_db.clone();
    let lite_arc = cdb.lock().lite.clone();

    let cmessages = {
        let conn = lite_arc.lock();
        let mut stmt = conn.prepare("SELECT * FROM cmessages WHERE cmessage_belongs_to_cthread_id = ?1 ORDER BY cmessage_num, cmessage_alt").map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e))
        })?;
        let rows = stmt.query(rusqlite::params![post.cmessage_belongs_to_cthread_id]).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Query error: {}", e))
        })?;
        cmessages_from_rows(rows)
    };

    let sse = stream! {
        for cmessage in cmessages {
            yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&cmessage).unwrap()));
        }

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&serde_json::json!({"type": "heartbeat"})).unwrap()));
        }
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-cache")
        .body(Body::wrap_stream(sse))
        .unwrap();

    Ok(response)
}

pub async fn handle_db_v1_cmessage_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let cdb = gcx.read().await.chore_db.clone();

    let incoming_json: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cmessage_belongs_to_cthread_id = incoming_json.get("cmessage_belongs_to_cthread_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let cmessage_num = incoming_json.get("cmessage_num").and_then(|v| v.as_i64()).unwrap_or_default() as i32;
    let cmessage_alt = incoming_json.get("cmessage_alt").and_then(|v| v.as_i64()).unwrap_or_default() as i32;
    // alt is special, introduce allocate_cmessage_alt and find next available alt within cthread_id?

    let cmessage_rec = cmessage_get(cdb.clone(), cmessage_belongs_to_cthread_id.clone(), cmessage_alt, cmessage_num)
        .map_err(|e| ScratchError::new(StatusCode::NOT_FOUND, format!("CMessage not found: {}", e)))?;

    let mut cmessage_json = serde_json::to_value(&cmessage_rec).unwrap();
    _merge_json(&mut cmessage_json, &incoming_json);

    let cmessage_rec: CMessage = serde_json::from_value(cmessage_json).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("Deserialization error: {}", e))
    })?;

    cmessage_set(cdb, cmessage_rec);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"status": "success"}).to_string()))
        .unwrap();

    Ok(response)
}
