use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use hashbrown::HashMap;
use serde::Deserialize;

use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use itertools::Itertools;
use tokio::io::AsyncWriteExt;
use tokio::fs::OpenOptions;
use tokio::sync::RwLock as ARwLock;
use tracing::info;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::files_in_workspace::read_file_from_disk;


#[derive(Deserialize)]
struct DiffApplyChunk {
    id: usize,
    line1: usize,
    line2: usize,
    text_orig: String,
    text: String,
}

#[derive(Deserialize)]
struct DiffApplyPost {
    file_name: String,
    chunks: Vec<DiffApplyChunk>,
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

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub line_n: usize,
    pub text: String,
    pub overwritten_by_id: Option<usize>,
    pub matches: Vec<usize>,
}

impl DiffLine {
    pub fn new(line_n: usize, text: String) -> Self {
        DiffLine {
            line_n,
            text,
            overwritten_by_id: None,
            matches: vec![]
        }
    }
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

fn find_streaks(hashsets: Vec<HashSet<usize>>, streaks: &mut Vec<HashSet<usize>>, streak_size: usize) {
    let mut res: Vec<HashSet<usize>> = vec![];
    info!("find streaks for hashsets: {:?}", hashsets);
    if hashsets.len() > 1 {
        for (h1, h2) in hashsets.iter().cloned().tuple_windows() {
            let h1 = if res.is_empty() { h1 } else { res.last().unwrap().clone() };
            let h2 = h2.iter().filter(|x| **x > 0).map(|x| x.clone()).collect::<HashSet<usize>>();
            info!("h1: {:?}; h2: {:?}", h1, h2);
            let i = h1.intersection(&h2.iter().map(|x| x - 1).collect::<HashSet<_>>()).cloned().collect::<HashSet<_>>();
            if res.is_empty() {
                res.push(i.clone());
            }
            res.push(i.iter().map(|x| x + 1).collect());
        }
    } else {
        res.push(hashsets.iter().next().unwrap().clone());
    }
    info!("res: {:?}", res);
    info!("streak size: {:?}", streak_size);

    let mut possible_streaks: usize = 0;
    for w in res.windows(streak_size).map(|w|w.to_vec()) {
        if w.iter().all(|s|s.len() == 1) {
            streaks.push(w.iter().map(|x|x.iter().next().unwrap().clone()).collect::<HashSet<usize>>());
        }
        if !w.iter().any(|s|s.is_empty()) {
            possible_streaks += 1;
        }
    }
    if possible_streaks > 1 {
        find_streaks(res, streaks, streak_size);
    } else {
        return;
    }
}

fn apply_chunk_to_text_fuzzy(
    lines_orig: &Vec<DiffLine>,
    chunk: &DiffApplyChunk,
    fuzzy_n: usize,
) -> Result<(Vec<DiffLine>, Vec<DiffLine>, Vec<DiffLine>), ScratchError> {
    let mut chunk_lines_orig = chunk.text_orig.lines().map(|l| DiffLine::new(0, l.to_string())).collect::<Vec<_>>();
    let mut chunk_lines = chunk.text.lines().map(|l| DiffLine{ line_n: 0, text: l.to_string(), overwritten_by_id: Some(chunk.id), matches: vec![]}).collect::<Vec<_>>();

    let lines_orig_filtered = lines_orig.iter().filter(|l|l.overwritten_by_id.is_none()).collect::<Vec<_>>();
    for l in chunk_lines_orig.iter_mut() {
        l.matches = lines_orig_filtered[(chunk.line1 as i32 - 1 - fuzzy_n as i32).max(0) as usize..(chunk.line2 as i32 - 1 + fuzzy_n as i32).min(lines_orig_filtered.len() as i32) as usize].iter()
            .filter(|ol| l.text == ol.text)
            .map(|ol| ol.line_n)
            .collect();

        if l.matches.is_empty() {
            return Err(ScratchError::new(StatusCode::BAD_REQUEST, "No matches found".to_string()));
        }
    }

    let hashsets = chunk_lines_orig.iter().map(|l| l.matches.iter().cloned().collect::<HashSet<_>>()).collect::<Vec<_>>();
    let mut streaks = vec![];
    find_streaks(hashsets, &mut streaks, chunk_lines_orig.len());

    let mut streak = match streaks.get(0) {
        Some(s) => s.into_iter().cloned().collect::<Vec<_>>(),
        None => return Err(ScratchError::new(StatusCode::BAD_REQUEST, "No streaks found".to_string()))
    };
    streak.sort();
    info!("streak: {:?}", streak);

    let start = *streak.first().unwrap();
    let mut new_lines = vec![];
    let mut replaced_lines = vec![];
    let mut prev_line_n = 0;
    for l in lines_orig.iter() {
        if l.overwritten_by_id.is_none() && !streak.contains(&l.line_n) {
            prev_line_n = l.line_n;
        }
        if !streak.contains(&l.line_n) {
            new_lines.push(l.clone());
        }
        if streak.contains(&l.line_n) {
            replaced_lines.push(l.clone());
        }
        if prev_line_n + 1 == start {
            for c in chunk_lines.iter_mut() {
                c.line_n = prev_line_n;
            }
            new_lines.extend(chunk_lines.clone());
        }
    }
    Ok((new_lines, replaced_lines, chunk_lines))
}

fn apply_chunks(
    chunks: &mut Vec<DiffApplyChunk>,
    file_text: &String
) -> Result<(Vec<DiffLine>, HashMap<usize, (Vec<DiffLine>, Vec<DiffLine>)>), ScratchError> {
    let mut lines_orig = file_text.lines().enumerate().map(|(line_n, l)| DiffLine::new(line_n + 1, l.to_string())).collect::<Vec<_>>();

    let mut mods = HashMap::new();
    for chunk in chunks.iter_mut() {
        if chunk.line1 < 1 {
            return Err(ScratchError::new(StatusCode::BAD_REQUEST, "Invalid line range: line1 cannot be < 1".to_string()));
        }

        let (lines_orig_new, replaced_lines, chunk_lines) = apply_chunk_to_text_fuzzy(&lines_orig, &chunk, 5)?;
        mods.insert(chunk.id, (replaced_lines, chunk_lines));
        lines_orig = lines_orig_new;
    }
    Ok((lines_orig, mods))
}

pub async fn handle_v1_diff_apply(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let mut post = serde_json::from_slice::<DiffApplyPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let (cpath, file_text) = get_file_text(global_context.clone(), &post.file_name).await?;

    let (new_lines, mods) = apply_chunks(&mut post.chunks, &file_text)?;
    let new_text = new_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    write_to_file(&cpath, &new_text).await?;

    // let cx = global_context.read().await;
    // cx.diffs_state.files.lock().unwrap().insert(post.file_name.clone(), result);
    // cx.diffs_state.edits.lock().unwrap().insert(post.id, (replaced, inserted));
    
    Ok(
        Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("OK"))
            .unwrap()
    )
}
