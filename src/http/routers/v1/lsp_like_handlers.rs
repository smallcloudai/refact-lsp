use std::path::PathBuf;

use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::files_in_workspace;

#[derive(Serialize, Deserialize, Clone)]
struct LspLikeInit {
    pub project_roots: Vec<Url>,
}

#[derive(Serialize, Deserialize, Clone)]
struct LspLikeDidChange {
    pub uri: Url,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct LspLikeAddFolder {
    pub uri: Url,
}

pub async fn handle_v1_lsp_initialize(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<LspLikeInit>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    let folders: Vec<PathBuf> = post.project_roots.iter().map(|x| PathBuf::from(x.path())).collect();
    *global_context.write().await.documents_state.workspace_folders.lock().unwrap() = folders;
    let files_count = files_in_workspace::on_workspaces_init(global_context).await;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1, "files_found": files_count}).to_string()))
        .unwrap())
}

pub async fn handle_v1_lsp_did_change(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<LspLikeDidChange>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    files_in_workspace::on_did_change(
        global_context.clone(),
        &post.uri,
        &post.text,
    ).await;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}

pub async fn handle_v1_lsp_add_folder(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<LspLikeAddFolder>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    files_in_workspace::add_folder(global_context.clone(), &post.uri.to_file_path().unwrap()).await;
    Ok(Response::builder()
       .status(StatusCode::OK)
       .body(Body::from(json!({"success": 1}).to_string()))
       .unwrap())
}

pub async fn handle_v1_lsp_remove_folder(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<LspLikeAddFolder>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    files_in_workspace::remove_folder(global_context.clone(), &post.uri.to_file_path().unwrap()).await;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}
