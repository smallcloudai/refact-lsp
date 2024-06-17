use std::path::PathBuf;
use std::sync::Arc;
use hashbrown::HashMap;

use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use tokio::io::AsyncWriteExt;
use tokio::fs::OpenOptions;
use tokio::sync::RwLock as ARwLock;
use tracing::info;

use crate::call_validation::{DiffChunk, DiffPost};
use crate::custom_error::ScratchError;
use crate::files_in_workspace::read_file_from_disk;
use crate::global_context::GlobalContext;


#[derive(Clone, Debug)]
struct DiffLine {
    line_n: usize,
    text: String,
    overwritten_by_id: Option<usize>,
}

async fn write_to_file(path: &String, text: &str) -> Result<(), ScratchError> {
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

fn find_chunk_streaks(chunk_lines: &Vec<DiffLine>, orig_lines: Vec<&DiffLine>) -> Result<Vec<Vec<usize>>, String> {
    let chunk_len = chunk_lines.len();
    let orig_len = orig_lines.len();

    if chunk_len == 0 || orig_len < chunk_len {
        return Err("Invalid input: chunk_lines is empty or orig_lines is smaller than chunk_lines".to_string());
    }

    let mut matches = vec![];
    for i in 0..=(orig_len - chunk_len) {
        let mut match_found = true;

        for j in 0..chunk_len {
            if orig_lines[i + j].text != chunk_lines[j].text {
                match_found = false;
                break;
            }
        }
        if match_found {
            let positions = (i..i + chunk_len).map(|index| orig_lines[index].line_n).collect::<Vec<usize>>();
            matches.push(positions);
        }
    }
    if matches.is_empty() {
        return Err("Chunk text not found in original text".to_string());
    }
    Ok(matches)
}

fn apply_chunk_to_text_fuzzy(
    chunk_id: usize,
    lines_orig: &Vec<DiffLine>,
    chunk: &DiffChunk,
    fuzzy_n: usize,
) -> Result<Vec<DiffLine>, ScratchError> {
    let chunk_lines_orig: Vec<_> = chunk.lines_remove.lines().map(|l| DiffLine { line_n: 0, text: l.to_string(), overwritten_by_id: None}).collect();
    let chunk_lines: Vec<_> = chunk.lines_add.lines().map(|l| DiffLine { line_n: 0, text: l.to_string(), overwritten_by_id: Some(chunk_id)}).collect();
    
    let search_in_window: Vec<_> = lines_orig.iter()
        .filter(|l|l.overwritten_by_id.is_none() && l.line_n >= (chunk.line1 as i32 - fuzzy_n as i32) as usize && l.line_n <= (chunk.line2 as i32 - 1 + fuzzy_n as i32) as usize).collect();
    
    info!("search in window: \n{}\n", search_in_window.iter().map(|x|x.text.clone()).collect::<Vec<_>>().join("\n"));
    
    let streaks = find_chunk_streaks(&chunk_lines_orig, search_in_window);
    let streak = streaks.map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("No streaks found: {}", e)))?[0].clone();

    info!("streak: {:?}", streak);
    
    let mut new_lines = vec![];
    let mut replaced_lines = vec![];
    let mut insert = false;
    for l in lines_orig.iter() {
        if streak.ends_with(&[l.line_n]) {
            insert = true;
        }
        if !streak.contains(&l.line_n) {
            new_lines.push(l.clone());
        } else {
            replaced_lines.push(l.clone());
        }
        if insert {
            new_lines.extend(chunk_lines.clone());
            insert = false;
        }
    }
    Ok(new_lines)
}

fn validate_chunk(chunk: &DiffChunk) -> Result<(), ScratchError> {
    if chunk.line1 < 1 {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "Invalid line range: line1 cannot be < 1".to_string()));
    }
    Ok(())
}

fn apply_chunks(
    chunks: &mut Vec<DiffChunk>,
    file_text: &String
) -> Result<Vec<DiffLine>, ScratchError> {
    let mut lines_orig = file_text.lines().enumerate().map(|(line_n, l)| DiffLine { line_n: line_n + 1, text: l.to_string(), overwritten_by_id: None}).collect::<Vec<_>>();

    for (idx, chunk) in chunks.iter_mut().enumerate() {
        validate_chunk(chunk)?;

        let lines_orig_new = apply_chunk_to_text_fuzzy(idx.clone(), &lines_orig, &chunk, 0)?;
        lines_orig = lines_orig_new;
    }
    Ok(lines_orig)
}

fn undo_chunks(
    chunks: &mut Vec<DiffChunk>,
    file_text: &String
) -> Result<Vec<DiffLine>, ScratchError> {
    let mut lines_orig = file_text.lines().enumerate().map(|(line_n, l)| DiffLine { line_n: line_n + 1, text: l.to_string(), overwritten_by_id: None}).collect::<Vec<_>>();
    
    for (idx, chunk) in chunks.iter_mut().enumerate() {
        validate_chunk(chunk)?;
        
        chunk.line2 = chunk.line1 + chunk.lines_remove.lines().count();
        info!("lines remove count: {}", chunk.lines_remove.lines().count());

        let mut lines_orig_new = apply_chunk_to_text_fuzzy(idx.clone(), &lines_orig, &chunk, 0)?;
        lines_orig_new = lines_orig_new.iter_mut().enumerate().map(|(idx, l)|{
            l.line_n = idx + 1;
            return l.clone();
        }).collect::<Vec<_>>();
        lines_orig = lines_orig_new;
    }
    Ok(lines_orig)
}

async fn process_content(content: &Vec<DiffChunk>, undo: bool) -> Result<(), ScratchError>{
    let mut chunk_groups = HashMap::new();
    for c in content.iter().cloned() {
        chunk_groups.entry(c.file_name.clone()).or_insert(Vec::new()).push(c);
    }
    for (file_name, chunks) in chunk_groups.iter_mut() {
        chunks.sort_by_key(|c|c.line1);

        let file_text = read_file_from_disk(&PathBuf::from(file_name)).await.map_err(|e| {
            ScratchError::new(StatusCode::NOT_FOUND, format!("couldn't read file: {:?}. Error: {}", file_name, e))
        }).map(|x| x.to_string())?;
        
        let new_lines = if undo {
            undo_chunks(chunks, &file_text)?
        } else {
            apply_chunks(chunks, &file_text)?
        };
        
        let new_text = new_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        write_to_file(&file_name, &new_text).await?;
    }

    Ok(())
}

pub async fn handle_v1_diff_apply(
    Extension(_global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<DiffPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    
    process_content(&post.content, false).await?;
    
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from("OK"))
        .unwrap())
}

pub async fn handle_v1_diff_undo(
    Extension(_global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<DiffPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    process_content(&post.content, true).await?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from("OK"))
        .unwrap())
}
