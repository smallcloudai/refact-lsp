use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::vecdb::structs::VecdbSearch;

#[derive(Serialize, Deserialize, Clone)]
struct VecDBPost {
    query: String,
    top_n: usize,
}

pub async fn handle_v1_vecdb_search(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<VecDBPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let res = match global_context.read().await.vec_db.clone() {
        Some(vecdb) => {
            let res = vecdb.lock().await.search(post.query.to_string(), post.top_n).await;
            res
        }
        None => {
            Err("vecdb is not configured".to_string())
        }
    };

    match res {
        Ok(search_res) => {
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(json!(search_res).to_string()))
                .unwrap())
        }
        Err(e) => {
            Err(ScratchError::new(StatusCode::BAD_REQUEST, e))
        }
    }
}

pub async fn handle_v1_vecdb_status(
    Extension(global_context): Extension<SharedGlobalContext>,
    _: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let res = match &global_context.read().await.vec_db {
        Some(vecdb) => {
            let status = vecdb.lock().await.get_status().await;
            Ok(status)
        }
        None => {
            Err("vecdb is not configured".to_string())
        }
    };

    match res {
        Ok(status) => {
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(json!(status).to_string()))
                .unwrap()
            )
        }
        Err(e) => {
            Err(ScratchError::new(StatusCode::BAD_REQUEST, e))
        }
    }
}
