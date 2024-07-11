use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use serde_json::json;

use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::vecdb::structs::VecdbSearch;


#[derive(Deserialize)]
struct MemAddRequest {
    mem_type: String,
    goal: String,
    project: String,
    payload: String,
}

#[derive(Deserialize)]
struct MemEraseRequest {
    memid: String,
}

#[derive(Deserialize)]
struct MemUpdateUsedRequest {
    memid: String,
    correct: f64,
    useful: f64,
}

#[derive(Deserialize)]
struct MemQuery {
    goal: String,
    project: String,
    top_n: usize,
}

pub async fn handle_mem_add(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemAddRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let vec_db = gcx.read().await.vec_db.clone();
    let memid = {
        let mut vec_db_locked = vec_db.lock().await;
        match vec_db_locked.as_mut() {
            None => {
                return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "VecDB is not initialized".to_string()));
            }
            Some(db) => {
                db.memories_add(&post.mem_type, &post.goal, &post.project, &post.payload).await.map_err(|e| {
                    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))
                })?
            }
        }
    };

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(format!("{{\"memid\": \"{}\"}}", memid)))
        .unwrap();

    Ok(response)
}

pub async fn handle_mem_erase(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemEraseRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let vec_db = gcx.read().await.vec_db.clone();
    let result = {
        let mut vec_db_locked = vec_db.lock().await;
        match vec_db_locked.as_mut() {
            None => {
                return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "VecDB is not initialized".to_string()));
            }
            Some(db) => {
                db.memories_erase(&post.memid).await.map_err(|e| {
                    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))
                })?
            }
        }
    };

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&json!({"success": 1})).unwrap()))
        .unwrap();

    Ok(response)
}

pub async fn handle_mem_update_used(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemUpdateUsedRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let vec_db = gcx.read().await.vec_db.clone();
    let result = {
        let mut vec_db_locked = vec_db.lock().await;
        match vec_db_locked.as_mut() {
            None => {
                return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "VecDB is not initialized".to_string()));
            }
            Some(db) => {
                db.memories_update(&post.memid, post.correct, post.useful).await.map_err(|e| {
                    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))
                })?
            }
        }
    };

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&json!({"success": 1})).unwrap()))
        .unwrap();

    Ok(response)
}

pub async fn handle_mem_query(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemQuery = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    // TODO: use project for filtering? (why is it f64??)

    let cx_locked = gcx.read().await;
    let search_res = match *cx_locked.vec_db.lock().await {
        Some(ref db) => db.memdb_search(post.goal.to_string(), post.top_n).await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error getting memdb search results: {e}")))?,
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "MemDB is not initialized".to_string()
            ));
        }
    };

    let response_body = serde_json::to_string_pretty(&search_res)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON serialization error: {}", e)))?;

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(response_body))
        .unwrap();
    Ok(response)
}
