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

use crate::agent_db::db_structs::{ChoreDB, CMessage};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;


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



#[derive(Deserialize)]
struct CMessagesSubscription {
    cmessage_belongs_to_cthread_id: String,
}

// HTTP handler
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

// HTTP handler
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
    crate::agent_db::merge_json(&mut cmessage_json, &incoming_json);

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

struct _CMessageKey {
    cmessage_belongs_to_cthread_id: String,
    cmessage_alt: i32,
    cmessage_num: i32,
}

fn _cmessage_subscription_poll(
    lite_arc: Arc<ParkMutex<rusqlite::Connection>>,
    seen_id: &mut i64
) -> Result<(Vec<_CMessageKey>, Vec<_CMessageKey>), String> {
    let conn = lite_arc.lock();
    let mut stmt = conn.prepare("
        SELECT pubevent_id, pubevent_action, pubevent_json
        FROM pubsub_events
        WHERE pubevent_id > ?1
        AND pubevent_channel = 'cmessage' AND (pubevent_action = 'update' OR pubevent_action = 'delete')
        ORDER BY pubevent_id ASC
    ").unwrap();
    let mut rows = stmt.query([*seen_id]).map_err(|e| format!("Failed to execute query: {}", e))?;
    let mut deleted_cmessage_keys = Vec::new();
    let mut updated_cmessage_keys = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("Failed to fetch row: {}", e))? {
        let id: i64 = row.get(0).unwrap();
        let action: String = row.get(1).unwrap();
        let json: String = row.get(2).unwrap();
        let cmessage_key = match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(parsed_json) => {
                let cthread_id = parsed_json["cmessage_belongs_to_cthread_id"].as_str();
                let alt = parsed_json["cmessage_alt"].as_i64();
                let num = parsed_json["cmessage_num"].as_i64();
                match (cthread_id, alt, num) {
                    (Some(id), Some(alt), Some(num)) => _CMessageKey {
                        cmessage_belongs_to_cthread_id: id.to_string(),
                        cmessage_alt: alt as i32,
                        cmessage_num: num as i32,
                    },
                    _ => {
                        tracing::error!("Missing or invalid cmessage key components in JSON: {}", json);
                        *seen_id = id;
                        continue;
                    }
                }
            },
            Err(e) => {
                tracing::error!("Failed to parse JSON: {}. Error: {}", json, e);
                *seen_id = id;
                continue;
            }
        };
        match action.as_str() {
            "delete" => deleted_cmessage_keys.push(cmessage_key),
            "update" => updated_cmessage_keys.push(cmessage_key),
            _ => return Err(format!("Unknown action: {}", action)),
        }
        *seen_id = id;
    }
    Ok((deleted_cmessage_keys, updated_cmessage_keys))
}

