use std::hash::Hash;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use axum::Extension;
use axum::http::{Response, StatusCode};
use hashbrown::HashMap;
use hyper::Body;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::fs::OpenOptions;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use tokio::sync::RwLock as ARwLock;
use crate::files_in_workspace::read_file_from_disk;


#[derive(Deserialize)]
struct DiffApplyPost {
    id: usize,
    file_name: String,
    file_action: String,
    line1: usize,
    line2: usize,
    text: String,
}

pub struct DiffsState {
    pub files: Arc<Mutex<HashMap<String, Vec<DiffLine>>>>,
    pub edits: Arc<Mutex<HashMap<usize, (Vec<DiffLine>, Vec<DiffLine>)>>>
}

impl DiffsState {
    pub fn new() -> Self {
        DiffsState {
            files: Arc::new(Mutex::new(HashMap::new())),
            edits: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Clone)]
pub struct DiffLine {
    pub text: String,
    pub overwritten_by_id: Option<usize>
}

async fn get_file_text(global_context: Arc<ARwLock<GlobalContext>>, file_path: &String) -> Result<(PathBuf, String), ScratchError>{
    let candidates: Vec<String> = crate::files_correction::correct_to_nearest_filename(
        global_context.clone(),
        file_path,
        false,
        5,
    ).await;
    if candidates.is_empty() {
        return Err(ScratchError::new(StatusCode::NOT_FOUND, format!("file {:?} not found in index", file_path)));
    }
    if candidates.len() > 1 {
        return Err(ScratchError::new(StatusCode::NOT_FOUND, format!("file {:?} correction was ambiguous. Correction results: {:?}", file_path, candidates)));
    }
    let cpath = crate::files_correction::canonical_path(candidates.get(0).unwrap());
    let file_text = read_file_from_disk(&cpath).await.map_err(|e|{
        ScratchError::new(StatusCode::NOT_FOUND, format!("couldn't read file: {:?}. Error: {}", file_path, e))
    }).map(|x|x.to_string())?;
    Ok((cpath, file_text))
}

fn apply_lines_to_text(
    file_text: &mut String, 
    post: &DiffApplyPost
) -> Result<(Vec<DiffLine>, Vec<DiffLine>, Vec<DiffLine>), ScratchError> {
    let lines: Vec<&str> = file_text.lines().collect();
    if post.line2 >= lines.len() || post.line1 > post.line2 {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "Invalid line range".to_string()));
    }
    let mut diff_lines = vec![];
    for l in lines {
        diff_lines.push(DiffLine {
            text: l.to_string(),
            overwritten_by_id: None
        })
    }
    
    let mut diff_lines_insert = vec![];
    for l in post.text.lines() {
        diff_lines_insert.push(DiffLine {
            text: l.to_string(),
            overwritten_by_id: Some(post.id)
        })
    }
    
    let to_replace = diff_lines[post.line1..=post.line2].into_iter().cloned().collect();
    
    let mut new_text = diff_lines[..post.line1].to_vec();
    new_text.extend(diff_lines_insert.clone());
    new_text.extend_from_slice(&diff_lines[post.line2 + 1..]);

    *file_text = new_text.iter().map(|d|d.text.clone()).collect::<Vec<String>>().join("\n");
    Ok((to_replace, diff_lines_insert, new_text))
}

async fn write_to_file(path: &PathBuf, text: &str) -> Result<(), ScratchError> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .await
        .map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file: {}", e))
        })?;

    file.write_all(text.as_bytes()).await.map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write to file: {}", e))
    })?;
    Ok(())
}

pub async fn handle_v1_diff_apply(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let mut post = serde_json::from_slice::<DiffApplyPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    if post.line1 < 1 {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "Invalid line range: line1 cannot be < 1".to_string()));
    }
    post.line1 -= 1;
    post.line2 -= 1;
    
    let (cpath, mut file_text) = get_file_text(global_context.clone(), &post.file_name).await?;
    write_to_file(&cpath, &file_text).await?;

    let (replaced, inserted, result) = apply_lines_to_text(&mut file_text, &post)?;
    let cx = global_context.read().await;
    cx.diffs_state.files.lock().unwrap().insert(post.file_name.clone(), result);
    cx.diffs_state.edits.lock().unwrap().insert(post.id, (replaced, inserted));
    
    Ok(
        Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("OK"))
            .unwrap()
    )
}