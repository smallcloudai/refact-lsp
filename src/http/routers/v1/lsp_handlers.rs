use std::path::{Path, PathBuf};
use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;
use walkdir::WalkDir;

use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::vecdb::file_filter::is_valid_file;

#[derive(Serialize, Deserialize, Clone)]
struct PostInit {
    pub project_roots: Vec<Url>,
}

#[derive(Serialize, Deserialize, Clone)]
struct PostDocument {
    pub uri: Url,
    pub text: String,
}


pub async fn handle_v1_lsp_initialize(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<PostInit>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let files: Vec<PathBuf> = post.project_roots
        .iter()
        .map( |f| {
            return WalkDir::new(Path::new(f.path()))
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| !e.path().is_dir())
                .filter(|e| is_valid_file(&e.path().to_path_buf()))
                .map(|e| e.path().to_path_buf())
                .collect::<Vec<PathBuf>>();
        })
        .flatten()
        .collect();
    global_context.read().await.vec_db.lock().await.add_or_update_files(files, true).await;

    // Real work here
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}

pub async fn handle_v1_lsp_did_changed(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<PostDocument>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let file_path = PathBuf::from(post.uri.path());
    if is_valid_file(&file_path) {
        global_context.read().await.vec_db.lock().await.add_or_update_file(
            file_path, false
        ).await;
    }

    // Real work here
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}