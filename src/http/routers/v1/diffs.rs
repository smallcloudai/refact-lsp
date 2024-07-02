use std::collections::HashSet;
use std::mem;
use std::path::PathBuf;
use std::sync::Arc;
use hashbrown::HashMap;

use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::fs::OpenOptions;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{DiffChunk, DiffPost};
use crate::custom_error::ScratchError;
use crate::files_in_workspace::read_file_from_disk;
use crate::global_context::GlobalContext;


const MAX_FUZZY_N: usize = 10;


#[derive(Clone, Debug, Default)]
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

fn find_chunk_matches(chunk_lines_remove: &Vec<DiffLine>, orig_lines: &Vec<&DiffLine>) -> Result<Vec<Vec<usize>>, String> {
    let chunk_len = chunk_lines_remove.len();
    let orig_len = orig_lines.len();

    if chunk_len == 0 || orig_len < chunk_len {
        return Err("Invalid input: chunk_lines is empty or orig_lines is smaller than chunk_lines".to_string());
    }

    let mut matches = vec![];
    for i in 0..=(orig_len - chunk_len) {
        let mut match_found = true;

        for j in 0..chunk_len {
            if orig_lines[i + j].text != chunk_lines_remove[j].text {
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
) -> Result<(usize, Vec<DiffLine>), String> {
    let chunk_lines_remove: Vec<_> = chunk.lines_remove.lines().map(|l| DiffLine { line_n: 0, text: l.to_string(), overwritten_by_id: None}).collect();
    let chunk_lines_add: Vec<_> = chunk.lines_add.lines().map(|l| DiffLine { line_n: 0, text: l.to_string(), overwritten_by_id: Some(chunk_id)}).collect();
    let mut new_lines = vec![];
    
    if chunk_lines_remove.is_empty() {
        new_lines.extend(lines_orig[..chunk.line1 - 1].iter().cloned().collect::<Vec<_>>());
        new_lines.extend(chunk_lines_add.iter().cloned().collect::<Vec<_>>());
        new_lines.extend(lines_orig[chunk.line1 - 1..].iter().cloned().collect::<Vec<_>>());
        return Ok((0, new_lines));
    }
    
    let mut fuzzy_n_used = 0;
    for fuzzy_n in 0..=MAX_FUZZY_N {
        let search_from = (chunk.line1 as i32 - fuzzy_n as i32).max(0) as usize;
        let search_till = (chunk.line2 as i32 - 1 + fuzzy_n as i32) as usize;
        let search_in_window: Vec<_> = lines_orig.iter()
            .filter(|l| l.overwritten_by_id.is_none() && l.line_n >= search_from && l.line_n <= search_till).collect();
        
        let matches = find_chunk_matches(&chunk_lines_remove, &search_in_window);

        let best_match = match matches {
            Ok(m) => {
                fuzzy_n_used = fuzzy_n;
                m[0].clone()
            },
            Err(e) => {
                if fuzzy_n >= MAX_FUZZY_N {
                    return Err(e)
                }
                continue;
            }
        };

        for l in lines_orig.iter() {
            if best_match.ends_with(&[l.line_n]) {
                new_lines.extend(chunk_lines_add.clone());
            }
            if !best_match.contains(&l.line_n) {
                new_lines.push(l.clone());
            }
        }
        break;
    }
    if new_lines.is_empty() {
        return Err("No streaks found".to_string());
    }
    Ok((fuzzy_n_used, new_lines))
}

fn validate_chunk(chunk: &DiffChunk) -> Result<(), String> {
    if chunk.line1 < 1 {
        return Err("Invalid line range: line1 cannot be < 1".to_string());
    }
    Ok(())
}

fn apply_chunks(
    chunks: &mut Vec<DiffChunk>,
    file_text: &String
) -> Result<(HashMap<usize, usize>, Vec<DiffLine>), String> {
    let mut lines_orig = file_text.lines().enumerate().map(|(line_n, l)| DiffLine { line_n: line_n + 1, text: l.to_string(), ..Default::default()}).collect::<Vec<_>>();
    // info!("apply_chunks lines_orig: \n\n{}\n\n", lines_orig.iter().map(|x|x.text.clone()).collect::<Vec<_>>().join("\n"));

    let mut results_fuzzy_ns = HashMap::new();
    for chunk in chunks.iter_mut() {
        if !chunk.apply { continue; }
        
        validate_chunk(chunk)?;

        let (fuzzy_n_used, lines_orig_new) = apply_chunk_to_text_fuzzy(chunk.chunk_id, &lines_orig, &chunk)?;
        lines_orig = lines_orig_new;
        results_fuzzy_ns.insert(chunk.chunk_id, fuzzy_n_used);
    }
    Ok((results_fuzzy_ns, lines_orig))
}

fn undo_chunks(
    chunks: &mut Vec<DiffChunk>,
    file_text: &String
) -> Result<(HashMap<usize, usize>, Vec<DiffLine>), String> {
    let mut lines_orig = file_text.lines().enumerate().map(|(line_n, l)| DiffLine { line_n: line_n + 1, text: l.to_string(), ..Default::default()}).collect::<Vec<_>>();

    let mut results_fuzzy_ns = HashMap::new();
    for chunk in chunks.iter_mut() {
        if !chunk.apply { continue; }

        validate_chunk(chunk)?;
        mem::swap(&mut chunk.lines_remove, &mut chunk.lines_add);
        
        chunk.line2 = chunk.line1 + chunk.lines_remove.lines().count();

        let (fuzzy_n_used, mut lines_orig_new) = apply_chunk_to_text_fuzzy(chunk.chunk_id, &lines_orig, &chunk)?;
        lines_orig_new = lines_orig_new.iter_mut().enumerate().map(|(idx, l)|{
            l.line_n = idx + 1;
            return l.clone();
        }).collect::<Vec<_>>();
        results_fuzzy_ns.insert(chunk.chunk_id, fuzzy_n_used);
        lines_orig = lines_orig_new;
    }
    Ok((results_fuzzy_ns, lines_orig))
}

async fn patch(chunks: &Vec<DiffChunk>, chunks_undo: &Vec<DiffChunk>) -> Result<HashMap<usize, usize>, String> {
    let mut chunk_groups = HashMap::new();
    for c in chunks.iter().cloned() {
        chunk_groups.entry(c.file_name.clone()).or_insert(Vec::new()).push(c);
    }
    let mut chunk_undo_groups = HashMap::new();
    for mut c in chunks_undo.iter().cloned() {
        c.apply = true;
        chunk_undo_groups.entry(c.file_name.clone()).or_insert(Vec::new()).push(c);
    }
    
    let mut results = HashMap::new();
    for (file_name, chunks_group) in chunk_groups.iter_mut() {
        chunks_group.sort_by_key(|c| c.line1);
        
        let mut file_text = read_file_from_disk(&PathBuf::from(file_name)).await.map_err(|e| {
            format!("couldn't read file: {:?}. Error: {}", file_name, e)
        }).map(|x| x.to_string())?;
        
        if let Some(mut chunks_undo_group) = chunk_undo_groups.get(file_name).cloned() {
            let (_, new_lines) = undo_chunks(&mut chunks_undo_group, &file_text).map_err(|e| e.to_string())?;
            file_text = new_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        }

        let (fuzzy_ns, new_lines) = apply_chunks(chunks_group, &file_text).map_err(|e| e.to_string())?;
        results.extend(fuzzy_ns);

        let new_text = new_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        write_to_file(&file_name, &new_text).await.map_err(|e| e.to_string())?;
    }
    Ok(results)
}

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
    
    let results = patch(&post.content, &chunks_undo).await
        .map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;

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
    
    let results = patch(&post.content, &undo_chunks).await
        .map_err(|e|ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let results_vec = results.keys().collect::<Vec<_>>();
    let new_state = old_state.iter().filter(|x|!results_vec.contains(&x)).cloned().collect::<Vec<_>>();
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
