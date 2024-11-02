use std::sync::Arc;
use parking_lot::Mutex as ParkMutex;
use tokio::sync::RwLock as ARwLock;
use rusqlite::params;
use serde_json::json;
use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;
use async_stream::stream;

use crate::agent_db::db_structs::{ChoreDB, Chore};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

pub fn chores_from_rows(
    mut rows: rusqlite::Rows,
) -> Vec<Chore> {
    let mut chores = Vec::new();
    while let Some(row) = rows.next().unwrap_or(None) {
        chores.push(Chore {
            chore_id: row.get("chore_id").unwrap(),
            chore_title: row.get("chore_title").unwrap(),
            chore_spontaneous_work_enable: row.get("chore_spontaneous_work_enable").unwrap(),
            chore_created_ts: row.get("chore_created_ts").unwrap(),
            chore_archived_ts: row.get("chore_archived_ts").unwrap(),
        });
    }
    chores
}

pub fn chore_set(
    cdb: Arc<ParkMutex<ChoreDB>>,
    chore: Chore,
) {
    let (lite, chore_sleeping_point) = {
        let db = cdb.lock();
        (db.lite.clone(), db.chore_sleeping_point.clone())
    };
    let conn = lite.lock();
    match conn.execute(
        "INSERT OR REPLACE INTO chores (
            chore_id,
            chore_title,
            chore_spontaneous_work_enable,
            chore_created_ts,
            chore_archived_ts
        ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            chore.chore_id,
            chore.chore_title,
            chore.chore_spontaneous_work_enable,
            chore.chore_created_ts,
            chore.chore_archived_ts,
        ],
    ) {
        Ok(_) => {},
        Err(e) => {
            tracing::error!("Failed to insert or replace chore:\n{} {}\nError: {}",
                chore.chore_id, chore.chore_title,
                e
            );
        }
    }
    drop(conn);
    let j = serde_json::json!({
        "chore_id": chore.chore_id,
    });
    crate::agent_db::chore_pubub_push(&lite, "chore", "update", &j, &chore_sleeping_point);
}

pub fn chore_get(
    cdb: Arc<ParkMutex<ChoreDB>>,
    chore_id: String,
) -> Result<Chore, String> {
    let db = cdb.lock();
    let conn = db.lite.lock();
    let mut stmt = conn.prepare("SELECT * FROM chores WHERE chore_id = ?1").unwrap();
    let rows = stmt.query(params![chore_id]).map_err(|e| e.to_string())?;
    let chores = chores_from_rows(rows);
    chores.into_iter().next().ok_or_else(|| format!("No Chore found with id: {}", chore_id))
}

// HTTP handler
pub async fn handle_db_v1_chore_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let cdb = gcx.read().await.chore_db.clone();

    let incoming_json: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let chore_id = incoming_json.get("chore_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();

    let chore_rec = match chore_get(cdb.clone(), chore_id.clone()) {
        Ok(existing_chore) => existing_chore,
        Err(_) => Chore {
            chore_id,
            ..Default::default()
        },
    };

    let mut chore_json = serde_json::to_value(&chore_rec).unwrap();
    crate::agent_db::merge_json(&mut chore_json, &incoming_json);

    let chore_rec: Chore = serde_json::from_value(chore_json).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("Deserialization error: {}", e))
    })?;

    chore_set(cdb, chore_rec);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"status": "success"}).to_string()))
        .unwrap();

    Ok(response)
}

#[derive(Deserialize, Default)]
struct ChoresSubscriptionPost {
    quicksearch: String,
    limit: usize,
    only_archived: bool,
}

// HTTP handler
pub async fn handle_db_v1_chores_sub(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChoresSubscriptionPost = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cdb = gcx.read().await.chore_db.clone();
    let lite_arc = cdb.lock().lite.clone();

    let (pre_existing_chores, mut last_event_id) = {
        let mut conn = lite_arc.lock();
        let tx = conn.transaction().unwrap();

        let mut stmt = tx.prepare("
            SELECT * FROM chores
            WHERE chore_title LIKE ?1 AND (?2 = 0 OR chore_archived_ts IS NOT NULL)
            ORDER BY chore_created_ts
            LIMIT ?3
        ").unwrap();
        let rows = stmt.query(rusqlite::params![
            format!("%{}%", post.quicksearch),
            post.only_archived as i32,
            post.limit as i64
        ]).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Query error: {}", e))
        })?;
        let chores = chores_from_rows(rows);

        let max_event_id: i64 = tx.query_row("SELECT COALESCE(MAX(pubevent_id), 0) FROM pubsub_events", [], |row| row.get(0))
            .map_err(|e| { ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get max event ID: {}", e)) })?;

        (chores, max_event_id)
    };

    let sse = stream! {
        for chore in pre_existing_chores {
            let e = json!({
                "sub_event": "chore_update",
                "chore_rec": chore
            });
            yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&e).unwrap()));
        }

        loop {
            if !crate::agent_db::chore_pubsub_sleeping_procedure(gcx.clone(), &cdb).await {
                break;
            }
            let (deleted_chore_keys, updated_chore_keys) = match _chore_subscription_poll(lite_arc.clone(), &mut last_event_id) {
                Ok(x) => x,
                Err(e) => {
                    tracing::error!("handle_db_v1_chores_sub(1): {:?}", e);
                    break;
                }
            };
            for deleted_key in deleted_chore_keys {
                let delete_event = json!({
                    "sub_event": "chore_delete",
                    "chore_id": deleted_key,
                });
                yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&delete_event).unwrap()));
            }
            for updated_key in updated_chore_keys {
                let chores = _chore_get_with_quicksearch(cdb.clone(), updated_key.clone(), &post);
                match chores.into_iter().next() {
                    Some(updated_chore) => {
                        let update_event = json!({
                            "sub_event": "chore_update",
                            "chore_rec": updated_chore
                        });
                        yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&update_event).unwrap()));
                    },
                    None => { }  // doesn't fit the quicksearch, fine
                }
            }
        }
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .body(Body::wrap_stream(sse))
        .unwrap();

    Ok(response)
}

fn _chore_get_with_quicksearch(
    cdb: Arc<ParkMutex<ChoreDB>>,
    chore_id: String,
    post: &ChoresSubscriptionPost,
) -> Vec<Chore> {
    let db = cdb.lock();
    let conn = db.lite.lock();
    let mut stmt = conn.prepare("
        SELECT * FROM chores
        WHERE chore_id = ?1 AND (chore_title LIKE ?2 AND (?3 = 0 OR chore_archived_ts IS NOT NULL))
    ").unwrap();
    let rows = stmt.query(params![
        chore_id,
        format!("%{}%", post.quicksearch),
        post.only_archived as i32
    ]).unwrap();
    chores_from_rows(rows)
}

fn _chore_subscription_poll(
    lite_arc: Arc<ParkMutex<rusqlite::Connection>>,
    seen_id: &mut i64
) -> Result<(Vec<String>, Vec<String>), String> {
    let conn = lite_arc.lock();
    let mut stmt = conn.prepare("
        SELECT pubevent_id, pubevent_action, pubevent_json
        FROM pubsub_events
        WHERE pubevent_id > ?1
        AND pubevent_channel = 'chore' AND (pubevent_action = 'update' OR pubevent_action = 'delete')
        ORDER BY pubevent_id ASC
    ").unwrap();
    let mut rows = stmt.query([*seen_id]).map_err(|e| format!("Failed to execute query: {}", e))?;
    let mut deleted_chore_ids = Vec::new();
    let mut updated_chore_ids = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("Failed to fetch row: {}", e))? {
        let id: i64 = row.get(0).unwrap();
        let action: String = row.get(1).unwrap();
        let json: String = row.get(2).unwrap();
        let chore_id = match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(parsed_json) => match parsed_json["chore_id"].as_str() {
                Some(id) => id.to_string(),
                None => {
                    tracing::error!("Missing chore_id in JSON: {}", json);
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
            "delete" => deleted_chore_ids.push(chore_id),
            "update" => updated_chore_ids.push(chore_id),
            _ => return Err(format!("Unknown action: {}", action)),
        }
        *seen_id = id;
    }
    Ok((deleted_chore_ids, updated_chore_ids))
}
