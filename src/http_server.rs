use tracing::info;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::io::Write;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use tokio::sync::RwLock as ARwLock;
use hyper::{Body, Request, Response, Server, Method, StatusCode};
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use serde_json::json;

use crate::caps;
use crate::scratchpads;
use crate::call_validation::{CodeCompletionPost, ChatPost};
use crate::global_context::GlobalContext;
use crate::caps::CodeAssistantCaps;
use crate::custom_error::ScratchError;
use crate::telemetry_basic;
use crate::telemetry_snippets;
use crate::completion_cache;
use crate::vectordb;


async fn handle_v1_vecdb_add(
    global_context: Arc<ARwLock<GlobalContext>>,
    body_bytes: hyper::body::Bytes
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<vectordb::VecDBPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cx_locked = global_context.read().await;

    let client = cx_locked.http_client.clone();
    let vecdb = cx_locked.vec_db.clone();

    let records = vectordb::get_embeddings(post, client).await;
    let res = vecdb.write().unwrap().add(records).await;
    match res {
        Ok(_) => {
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(json!({"success": true}).to_string()))
                .unwrap())
        }
        Err(e) => {
            Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("Vecdb problem: {}", e)))
        }
    }
}

async fn handle_v1_vecdb_search(
    global_context: Arc<ARwLock<GlobalContext>>,
    body_bytes: hyper::body::Bytes
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<vectordb::VecDBPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cx_locked = global_context.read().await;

    let client = cx_locked.http_client.clone();
    let vecdb = cx_locked.vec_db.clone();

    let records = vectordb::get_embeddings(post, client).await;
    assert!(records.len() == 1);
    let res = vecdb.write().unwrap().find(records[0].clone().vector).await;

    match res {
        Ok(recs) => {
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(json!(recs).to_string()))
                .unwrap())
        }
        Err(e) => {
            Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("Vecdb problem: {}", e)))
        }
    }
}
