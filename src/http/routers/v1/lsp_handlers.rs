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
use crate::telemetry;
use crate::vecdb::file_filter::{is_valid_file, retrieve_files_by_proj_folders};

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

    let files = retrieve_files_by_proj_folders(
        post.project_roots.iter().map(|x| PathBuf::from(x.path())).collect()
    ).await;

    if let Some(vec_db) = global_context.read().await.vec_db.clone() {
        vec_db.lock().await.add_or_update_files(
            files, true
        ).await;
    }

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
        if let Some(vec_db) = global_context.read().await.vec_db.clone() {
            vec_db.lock().await.add_or_update_file(file_path, false).await;
        }
    }

    telemetry::snippets_collection::sources_changed(
        global_context,
        &post.uri.to_string(),
        &post.text,
    ).await;

    // Real work here
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}