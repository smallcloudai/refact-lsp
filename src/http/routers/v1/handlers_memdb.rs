use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use serde_json::json;

use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::vecdb::vdb_structs::VecdbSearch;


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
    let memid = crate::vecdb::vdb_highlev::memories_add(
        vec_db,
        &post.mem_type,
        &post.goal,
        &post.project,
        &post.payload
    ).await.map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))
    })?;

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
    let erased_cnt = crate::vecdb::vecdb::memories_erase(vec_db, &post.memid).await.map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))
    })?;

    assert!(erased_cnt <= 1);

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&json!({"success": erased_cnt>0})).unwrap()))
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
    let updated_cnt = crate::vecdb::vecdb::memories_update(
        vec_db,
        &post.memid,
        post.correct,
        post.useful
    ).await.map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))
    })?;

    assert!(updated_cnt <= 1);

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&json!({"success": updated_cnt>0})).unwrap()))
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

    let cx_locked = gcx.read().await;
    let vec_db = cx_locked.vec_db.clone();

    let search_res = crate::vecdb::vecdb::memories_search(vec_db, post.goal, post.top_n).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error getting memdb search results: {e}")))?;

    let response_body = serde_json::to_string_pretty(&search_res).unwrap();

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(response_body))
        .unwrap();
    Ok(response)
}
