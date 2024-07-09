use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;

use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::at_tools::att_knowledge::MemoriesDatabase;

#[derive(Deserialize)]
struct MemAddRequest {
    memtype: String,
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
    project: f64,
}

pub async fn gcx2memdb(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<Arc<AMutex<MemoriesDatabase>>, ScratchError> {
    let vec_db_module = gcx.read().await.vec_db.clone();
    let x = if let Some(ref mut db) = *vec_db_module.lock().await {
        Ok(db.memdb.clone())
    } else {
        Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "VecDB is not enabled or there's no vectorization model available, memory cannot work either :/".to_string()))
    };
    x
}

pub async fn handle_mem_add(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemAddRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    let memdb = gcx2memdb(gcx).await?;
    let memdb_locked = memdb.lock().await;
    match memdb_locked.add(&post.memtype, &post.goal, &post.project, &post.payload) {
        Ok(memid) => {
            let response = Response::builder()
                .header("Content-Type", "application/json")
                .body(Body::from(format!("{{\"memid\": \"{}\"}}", memid)))
                .unwrap();
            Ok(response)
        }
        Err(e) => Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

pub async fn handle_mem_erase(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemEraseRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    let memdb = gcx2memdb(gcx).await?;
    let memdb_locked = memdb.lock().await;
    match memdb_locked.erase(&post.memid) {
        Ok(_) => {
            let response = Response::builder()
                .header("Content-Type", "application/json")
                .body(Body::from("{\"status\": \"success\"}"))
                .unwrap();
            Ok(response)
        }
        Err(e) => Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

pub async fn handle_mem_update_used(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemUpdateUsedRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    let memdb = gcx2memdb(gcx).await?;
    let memdb_locked = memdb.lock().await;
    match memdb_locked.update_used(&post.memid, post.correct, post.useful) {
        Ok(_) => {
            let response = Response::builder()
                .header("Content-Type", "application/json")
                .body(Body::from("{\"status\": \"success\"}"))
                .unwrap();
            Ok(response)
        }
        Err(e) => Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

pub async fn handle_mem_query(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: MemQuery = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    let memdb = gcx2memdb(gcx).await?;
    let memdb_locked = memdb.lock().await;
    // Implement the query logic here using memdb_locked and post
    // For now, returning a placeholder response
    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from("{\"status\": \"query not implemented\"}"))
        .unwrap();
    Ok(response)
}
