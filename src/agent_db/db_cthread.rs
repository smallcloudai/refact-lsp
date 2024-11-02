use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use parking_lot::Mutex as ParkMutex;
use serde_json::json;
use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;
use async_stream::stream;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::agent_db::db_structs::{ChoreDB, CThread};
use crate::agent_db::db_init::pubsub_sleeping_procedure;


pub fn cthread_get(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cthread_id: String,
) -> Result<CThread, String> {
    let db = cdb.lock();
    let conn = db.lite.lock();
    let mut stmt = conn.prepare("SELECT * FROM cthreads WHERE cthread_id = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query(rusqlite::params![cthread_id])
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
        rusqlite::params![
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

    let event_json = serde_json::json!({
        "cthread_id": cthread.cthread_id,
        "cthread_belongs_to_chore_event_id": cthread.cthread_belongs_to_chore_event_id,
    });
    conn.execute(
        "INSERT INTO pubsub_events (pubevent_channel, pubevent_action, pubevent_json)
         VALUES ('cthread', 'update', ?1)",
        rusqlite::params![event_json.to_string()],
    ).expect("Failed to insert pubsub event for chat thread update");
    db.chore_sleeping_point.notify_waiters();
}

// HTTP handler
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
    crate::agent_db::merge_json(&mut chat_thread_json, &incoming_json);

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

#[derive(Deserialize)]
struct CThreadSubscription {
    #[serde(default)]
    quicksearch: String,
    #[serde(default)]
    limit: usize,
}

// HTTP handler
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
        loop {
            if !pubsub_sleeping_procedure(gcx.clone(), &cdb).await {
                break;
            }
            match _cthread_subsription_poll(lite_arc.clone(), &mut last_event_id) {
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

fn _cthread_subsription_poll(
    lite_arc: Arc<ParkMutex<rusqlite::Connection>>,
    seen_id: &mut i64
) -> Result<(Vec<String>, Vec<String>), String> {
    let conn = lite_arc.lock();
    let mut stmt = conn.prepare("
        SELECT pubevent_id, pubevent_action, pubevent_json
        FROM pubsub_events
        WHERE pubevent_id > ?1
        AND pubevent_channel = 'cthread' AND (pubevent_action = 'update' OR pubevent_action = 'delete')
        ORDER BY pubevent_id ASC
    ").unwrap();
    let mut rows = stmt.query([*seen_id]).map_err(|e| format!("Failed to execute query: {}", e))?;
    let mut deleted_cthread_ids = Vec::new();
    let mut updated_cthread_ids = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("Failed to fetch row: {}", e))? {
        let id: i64 = row.get(0).unwrap();
        let action: String = row.get(1).unwrap();
        let json: String = row.get(2).unwrap();
        let cthread_id = match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(parsed_json) => match parsed_json["cthread_id"].as_str() {
                Some(id) => id.to_string(),
                None => {
                    tracing::error!("Missing cthread_id in JSON: {}", json);
                    *seen_id = id;
                    continue;
                }
            },
            Err(e) => {
                tracing::error!("Failed to parse JSON: {}. Error: {}", json, e);
                *seen_id = id;
                continue;
            }
        };
        match action.as_str() {
            "delete" => deleted_cthread_ids.push(cthread_id),
            "update" => updated_cthread_ids.push(cthread_id),
            _ => return Err(format!("Unknown action: {}", action)),
        }
        *seen_id = id;
    }
    Ok((deleted_cthread_ids, updated_cthread_ids))
}
