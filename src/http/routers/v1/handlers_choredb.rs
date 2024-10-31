use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use serde_json::json;
use indexmap::IndexMap;
use axum::Extension;
use axum::response::Result;
use axum::extract::Query;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;
use async_stream::stream;
use rusqlite::{params, Connection};

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::chore_db::{cthread_get, cthreads_from_rows, cthread_set};
use crate::chore_schema::{ChatThread, ChoreEvent, Chore};
use crate::call_validation::ChatMessage;


// db_v1/cthread_sub     { quicksearch, limit } -> SSE
// db_v1/cthread_update  { Option<cthread_id>, fields } -> cthread_id (and SSE in other channel)
// db_v1/cthread_delete  { cthread_id } -> ok or detail
// db_v1/cmessages_sub     { cthread_id } -> SSE
// db_v1/cmessages_update  { cthread_id, n_onwards } -> ok or detail

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
        ChatThread::default()
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

pub async fn handle_db_v1_cthread_sub(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let mut subscription: CThreadSubscription = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    if subscription.limit == 0 {
        subscription.limit = 100;
    }

    let cdb = gcx.read().await.chore_db.clone();
    let lite_arc = cdb.lock().lite.clone();

    let cthreads = if subscription.quicksearch.is_empty() {
        let conn = lite_arc.lock();
        let mut stmt = conn.prepare("SELECT * FROM cthreads LIMIT ?1").map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e))
        })?;
        let rows = stmt.query(params![subscription.limit]).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Query error: {}", e))
        })?;
        cthreads_from_rows(rows)
    } else {
        let conn = lite_arc.lock();
        let mut stmt = conn.prepare("SELECT * FROM cthreads WHERE cthread_title LIKE ?1 LIMIT ?2").map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e))
        })?;
        let rows = stmt.query(params![format!("%{}%", subscription.quicksearch), subscription.limit]).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Query error: {}", e))
        })?;
        cthreads_from_rows(rows)
    };

    let sse = stream! {
        for cthread in cthreads {
            yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&cthread).unwrap()));
        }

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            yield Ok::<_, ScratchError>(format!("data: {}\n\n", serde_json::to_string(&serde_json::json!("")).unwrap()));
        }
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        // .header("Content-Type", "text/event-stream")
        // .header("Cache-Control", "no-cache")
        .body(Body::wrap_stream(sse))
        .unwrap();

    Ok(response)
}


// pub async fn handle_chat_message_get(
//     Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
//     query_bytes: hyper::body::Bytes,
// ) -> Result<Response<Body>, ScratchError> {
//     let params: ChatMessageGetQuery = serde_urlencoded::from_bytes(&query_bytes)
//         .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid query parameters: {}", e)))?;

//     let cdb = gcx.read().await.chore_db.clone();

//     let message = chat_message_get(cdb, params.cthread_id, params.i).await
//         .ok_or_else(|| ScratchError::new(StatusCode::NOT_FOUND, "Chat message not found".to_string()))?;

//     let response = Response::builder()
//         .header("Content-Type", "application/json")
//         .body(Body::from(serde_json::to_string(&message).unwrap()))
//         .unwrap();

//     Ok(response)
// }

// pub async fn handle_chat_message_set(
//     Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
//     body_bytes: hyper::body::Bytes,
// ) -> Result<Response<Body>, ScratchError> {
//     let post: ChatMessageSetRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
//         tracing::info!("cannot parse input:\n{:?}", body_bytes);
//         ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
//     })?;

//     let cdb = gcx.read().await.chore_db.clone();

//     chat_message_set(cdb, post.cthread_id, post.i, post.message).await;

//     let response = Response::builder()
//         .status(StatusCode::OK)
//         .header("Content-Type", "application/json")
//         .body(Body::from(json!({"status": "success"}).to_string()))
//         .unwrap();

//     Ok(response)
// }
