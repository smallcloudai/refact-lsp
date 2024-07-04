use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use hashbrown::HashMap;
use std::sync::Arc;

use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{at_file_repair_candidates, file_repair_candidates};
use crate::call_validation::DiffChunk;
use crate::custom_error::ScratchError;
use crate::diffs::{patch, write_to_file};
use crate::global_context::GlobalContext;


const MAX_FUZZY_N: usize = 10;


#[derive(Deserialize)]
pub struct DiffPost {
    pub apply: Vec<bool>,
    pub chunks: Vec<DiffChunk>,
    #[serde(skip_serializing, default)]
    pub id: u64
}

impl DiffPost {
    pub fn set_id(&mut self) {
        let mut hasher = DefaultHasher::new();
        self.chunks.hash(&mut hasher);
        self.id = hasher.finish();
    }
}

#[derive(Serialize)]
struct DiffResponseItem {
    chunk_id: usize,
    fuzzy_n_used: usize,
}

#[derive(Serialize)]
struct HandleDiffResponse {
    fuzzy_results: Vec<DiffResponseItem>,
    state: Vec<usize>,
}

fn results_into_state_vector(results: &HashMap<usize, Option<usize>>, total: usize) -> Vec<usize> {
    let mut state_vector = vec![0; total];
    for (k, v) in results {
        if *k < total {
            state_vector[*k] = if v.is_some() { 1 } else { 2 };
        }
    }
    state_vector
}

fn validate_post(post: &DiffPost) -> Result<(), ScratchError> {
    if post.chunks.is_empty() {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "`chunks` shouldn't be empty".to_string()));
    }
    if post.chunks.len() != post.apply.len() {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "`chunks` and `apply` arrays are not of the same length".to_string()));
    }
    if post.apply.iter().all(|&x|x == false) {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "`apply` array should contain at least one `true` value".to_string()));
    }
    Ok(())
}

async fn init_post_chunks(post: &mut DiffPost, global_context: Arc<ARwLock<GlobalContext>>) -> Result<(), ScratchError> {
    for ((c_idx, c), a) in post.chunks.iter_mut().enumerate().zip(post.apply.iter()) {
        c.chunk_id = c_idx;
        c.apply = *a;
        
        let file_path = PathBuf::from(&c.file_name);
        if !file_path.is_file() {
            let candidates = file_repair_candidates(&c.file_name, global_context.clone(), 5, false).await;
            let fuzzy_candidates = file_repair_candidates(&c.file_name, global_context.clone(), 5, true).await;

            if candidates.len() > 1 {
                return Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("file_name `{}` is ambiguous.\nIt could be interpreted as:\n{}", &c.file_name, candidates.join("\n"))));
            }
            if candidates.is_empty() {
                return if !fuzzy_candidates.is_empty() {
                    Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("file_name `{}` is not found.\nHowever, there are similar paths:\n{}", &c.file_name, fuzzy_candidates.join("\n"))))
                } else {
                    Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("file_name `{}` is not found", &c.file_name)))
                }
            }
            let candidate = candidates.get(0).unwrap();
            if !PathBuf::from(&candidate).is_file() {
                return Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("file_name `{}` is not found.\nHowever, there are similar paths:\n{}", &c.file_name, fuzzy_candidates.join("\n"))));
            }
            c.file_name = candidate.clone();
        }
    }
    Ok(())
}

pub async fn handle_v1_diff_apply(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let mut post = serde_json::from_slice::<DiffPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    post.set_id();

    validate_post(&post)?;
    init_post_chunks(&mut post, global_context.clone()).await?;

    let diff_state = global_context.read().await.documents_state.diffs_applied_state.clone();
    let applied_state = diff_state.get(&post.id).map(|x|x.clone()).unwrap_or_default();
    // undo all chunks that are already applied to file, then re-apply them all + new chunks from post
    let chunks_undo = post.chunks.iter().filter(|x|applied_state.get(x.chunk_id) == Some(&1)).cloned().collect::<Vec<_>>();
    
    post.chunks.iter_mut().for_each(|x| {
        if applied_state.get(x.chunk_id) == Some(&1) {
            x.apply = true;
        }
    });

    let (texts_after_patch, results) = patch(&post.chunks, &chunks_undo, MAX_FUZZY_N).await
        .map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    
    for (file_name, new_text) in texts_after_patch.iter() {
        write_to_file(file_name, new_text).await.map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    }

    let new_state = results_into_state_vector(&results, post.chunks.len());
    global_context.write().await.documents_state.diffs_applied_state.insert(post.id, new_state.clone());
    
    let fuzzy_results: Vec<DiffResponseItem> = results.iter().filter(|x|x.1.is_some())
        .map(|(chunk_id, fuzzy_n_used)| DiffResponseItem {
            chunk_id: chunk_id.clone(),
            fuzzy_n_used: fuzzy_n_used.unwrap()
        })
        .collect();
    
    let response = HandleDiffResponse {
        fuzzy_results,
        state: new_state,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string_pretty(&response).unwrap()))
        .unwrap())
}

pub async fn handle_v1_diff_undo(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let mut post = serde_json::from_slice::<DiffPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    post.set_id();

    validate_post(&post)?;
    init_post_chunks(&mut post, global_context.clone()).await?;

    let apply_ids_from_post = post.chunks.iter().filter(|x|x.apply).map(|x|x.chunk_id).collect::<Vec<_>>();
    
    let applied_state = global_context.read().await.documents_state.diffs_applied_state.get(&post.id).map(|x|x.clone()).unwrap_or_default();
    
    // undo all chunks that are already applied to a file, then, apply them all, excluding the ones from post
    let undo_chunks = post.chunks.iter().filter(|x|applied_state.get(x.chunk_id) == Some(&1)).cloned().collect::<Vec<_>>();
    
    post.chunks.iter_mut().for_each(|x| {
        if applied_state.get(x.chunk_id) == Some(&1) {
            x.apply = true;
        }
        if apply_ids_from_post.contains(&x.chunk_id) {
            x.apply = false;
        }
    });
    
    let (texts_after_patch, results) = patch(&post.chunks, &undo_chunks, MAX_FUZZY_N).await
        .map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    for (file_name, new_text) in texts_after_patch.iter() {
        write_to_file(file_name, new_text).await.map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    }

    let new_state = results_into_state_vector(&results, post.chunks.len());
    global_context.write().await.documents_state.diffs_applied_state.insert(post.id, new_state.clone());

    let fuzzy_results: Vec<DiffResponseItem> = results.iter().filter(|x|x.1.is_some())
        .map(|(chunk_id, fuzzy_n_used)| DiffResponseItem {
            chunk_id: chunk_id.clone(),
            fuzzy_n_used: fuzzy_n_used.unwrap()
        })
        .collect();

    let response = HandleDiffResponse {
        fuzzy_results,
        state: new_state,
    };
    
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string_pretty(&response).unwrap()))
        .unwrap())
}

#[derive(Deserialize)]
struct DiffAppliedStatePost {
    pub chunks: Vec<DiffChunk>,
    #[serde(skip_serializing, default)]
    pub id: u64
}

impl DiffAppliedStatePost {
    pub fn set_id(&mut self) {
        let mut hasher = DefaultHasher::new();
        self.chunks.hash(&mut hasher);
        self.id = hasher.finish();
    }
}

#[derive(Serialize)]
struct DiffAppliedStateResponse {
    id: u64,
    state: Vec<usize>,
}

pub async fn handle_v1_diff_applied_chunks(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let mut post = serde_json::from_slice::<DiffAppliedStatePost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    post.set_id();
    
    let diff_state = global_context.read().await.documents_state.diffs_applied_state.clone();

    let state = diff_state.get(&post.id)
        .cloned()
        .unwrap_or_default();

    let response = DiffAppliedStateResponse {
        id: post.id,
        state,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}
