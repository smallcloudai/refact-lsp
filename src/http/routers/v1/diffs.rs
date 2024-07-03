use std::collections::HashSet;
use std::sync::Arc;

use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::DiffPost;
use crate::custom_error::ScratchError;
use crate::diffs::{patch, write_to_file};
use crate::global_context::GlobalContext;


const MAX_FUZZY_N: usize = 10;


#[derive(Serialize)]
struct DiffResponseItem {
    chunk_id: usize,
    fuzzy_n_used: usize,
}

pub async fn handle_v1_diff_apply(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let mut post = serde_json::from_slice::<DiffPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    let diff_state = global_context.read().await.documents_state.diffs_applied_state.clone();

    let mut chunks_undo = vec![];
    let applied_state = diff_state.get(&(post.chat_id.clone(), post.message_id.clone())).map(|x|x.clone()).unwrap_or_default();
    // undo all chunks that are already applied to file, then re-apply them all + new chunks from post
    chunks_undo.extend(post.content.iter().filter(|x|applied_state.contains(&x.chunk_id)).cloned());
    post.content.iter_mut().for_each(|x| {
        if applied_state.contains(&x.chunk_id) {
            x.apply = true;
        }
    });

    let (texts_after_patch, results) = patch(&post.content, &chunks_undo, MAX_FUZZY_N).await
        .map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    
    for (file_name, new_text) in texts_after_patch.iter() {
        write_to_file(file_name, new_text).await.map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    }

    let new_state = chunks_undo.iter().map(|x|x.chunk_id).chain(results.keys().cloned()).collect::<HashSet<_>>();
    global_context.write().await.documents_state.diffs_applied_state.insert((post.chat_id, post.message_id), new_state.into_iter().collect::<Vec<_>>());

    let response_items: Vec<DiffResponseItem> = results.into_iter()
        .map(|(chunk_id, fuzzy_n_used)| DiffResponseItem {
            chunk_id, fuzzy_n_used,
        })
        .collect();

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response_items).unwrap()))
        .unwrap())
}

pub async fn handle_v1_diff_undo(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let mut post = serde_json::from_slice::<DiffPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let apply_ids_from_post = post.content.iter().filter(|x|x.apply).map(|x|x.chunk_id).collect::<Vec<_>>();
    let old_state = global_context.read().await.documents_state.diffs_applied_state.get(&(post.chat_id.clone(), post.message_id.clone())).map(|x|x.clone()).unwrap_or_default();

    
    if !apply_ids_from_post.iter().all(|x|old_state.contains(x)) {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "some chunks are not listed as applied. consult /diff-applied-chunks".to_string()));
    }
    
    // undo all chunks that are already applied to a file, then, apply them all, excluding the ones from post
    let undo_chunks = post.content.iter().filter(|x|old_state.contains(&x.chunk_id)).cloned().collect::<Vec<_>>();
    post.content.iter_mut().for_each(|x| {
        if old_state.contains(&x.chunk_id) {
            x.apply = true;
        }
        if apply_ids_from_post.contains(&x.chunk_id) {
            x.apply = false;
        }
    });
    
    let (texts_after_patch, results) = patch(&post.content, &undo_chunks, MAX_FUZZY_N).await
        .map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    for (file_name, new_text) in texts_after_patch.iter() {
        write_to_file(file_name, new_text).await.map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    }

    let new_state = old_state.iter().filter(|x|!apply_ids_from_post.contains(&x)).cloned().collect::<Vec<_>>();
    global_context.write().await.documents_state.diffs_applied_state.insert((post.chat_id, post.message_id), new_state);

    let response_items: Vec<DiffResponseItem> = results.into_iter()
        .map(|(chunk_id, fuzzy_n_used)| DiffResponseItem {
            chunk_id, fuzzy_n_used,
        })
        .collect();

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response_items).unwrap()))
        .unwrap())
}

#[derive(Deserialize)]
struct DiffAppliedStatePost {
    chat_id: String,
    message_id: String,
}

#[derive(Serialize)]
struct DiffAppliedStateResponse {
    applied_chunks: Vec<usize>,
}

pub async fn handle_v1_diff_applied_chunks(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<DiffAppliedStatePost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    
    let diff_state = global_context.read().await.documents_state.diffs_applied_state.clone();

    let applied_chunks = diff_state.get(&(post.chat_id, post.message_id))
        .cloned()
        .unwrap_or_default();

    let response = DiffAppliedStateResponse {
        applied_chunks,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}
