use std::sync::Arc;
use std::path::PathBuf;
use tracing::{info, warn};

use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tokio::process::Command;

use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::execute_at::AtCommandMember;
use crate::call_validation::{ContextEnum, DiffChunk};


pub struct AtDiff {
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtDiff {
    pub fn new() -> Self {
        AtDiff { params: vec![] }
    }
}

async fn execute_diff(vcs: &str, project_dir: &str, args: &[&str]) -> Result<Vec<DiffChunk>, String> {
    fn process_diff_line(line: &str, current_chunk: &mut DiffChunk) {
        if line.starts_with('-') {
            current_chunk.lines_remove.push_str(&line[1..]);
            current_chunk.lines_remove.push('\n');
        } else if line.starts_with('+') {
            current_chunk.lines_add.push_str(&line[1..]);
            current_chunk.lines_add.push('\n');
        } else if line.starts_with(' ') {
            current_chunk.lines_remove.push_str(&line[1..]);
            current_chunk.lines_remove.push('\n');
            current_chunk.lines_add.push_str(&line[1..]);
            current_chunk.lines_add.push('\n');
        }
    }

    let output = Command::new(vcs)
        .arg("diff")
        .args(args)
        .current_dir(PathBuf::from(project_dir))
        .output()
        .await
        .map_err(|e| e.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !stderr.is_empty() {
        return Err(stderr);
    }

    let mut diff_chunks = Vec::new();
    let mut current_chunk = DiffChunk::default();
    let mut file_name = String::new();
    let mut in_diff_block = false;

    for line in stdout.lines() {
        if line.starts_with("diff --git") || line.starts_with("Index:") || line.starts_with("diff -r") {
            file_name = line.split_whitespace().last().unwrap_or("").to_string();
            if in_diff_block {
                diff_chunks.push(current_chunk);
            }
            current_chunk = DiffChunk {
                file_name: file_name.clone(),
                file_action: "edit".to_string(),
                ..Default::default()
            };
            in_diff_block = true;
        } else if line.starts_with("@@") {
            if !current_chunk.lines_remove.is_empty() || !current_chunk.lines_add.is_empty() {
                current_chunk.lines_add = current_chunk.lines_add.trim_end_matches('\n').to_string();
                current_chunk.lines_remove = current_chunk.lines_remove.trim_end_matches('\n').to_string();
                diff_chunks.push(current_chunk);
                current_chunk = DiffChunk {
                    file_name: file_name.clone(),
                    file_action: "edit".to_string(),
                    ..Default::default()
                };
            }
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() > 2 {
                let l1_numbers = parts[1].split(',').collect::<Vec<_>>();
                let l2_numbers = parts[2].split(',').collect::<Vec<_>>();
                if !l1_numbers.is_empty() && l2_numbers.len() > 1 {
                    current_chunk.line1 = l1_numbers[0].trim_start_matches('-').parse().unwrap_or(0);
                    current_chunk.line2 = current_chunk.line1 + l2_numbers[1].trim_start_matches('+').trim_start_matches(',').parse().unwrap_or(0);
                }
            }
        }
        process_diff_line(line, &mut current_chunk);
    }
    if in_diff_block && (!current_chunk.lines_remove.is_empty() || !current_chunk.lines_add.is_empty()) {
        diff_chunks.push(current_chunk);
    }
    Ok(diff_chunks)
}

pub async fn execute_git_diff(project_dir: &str, args: &[&str]) -> Result<Vec<DiffChunk>, String> {
    execute_diff("git", project_dir, args).await
}

pub async fn execute_svn_diff(project_dir: &str, args: &[&str]) -> Result<Vec<DiffChunk>, String> {
    execute_diff("svn", project_dir, args).await
}

pub async fn execute_hg_diff(project_dir: &str, args: &[&str]) -> Result<Vec<DiffChunk>, String> {
    execute_diff("hg", project_dir, args).await
}

// TODO we'll have the same one in at_file.rs, import 
pub async fn get_project_paths(ccx: &AtCommandsContext) -> Vec<PathBuf> {
    let cx = ccx.global_context.read().await;
    let workspace_folders = cx.documents_state.workspace_folders.lock().unwrap();
    workspace_folders.iter().cloned().collect::<Vec<_>>()
}

pub fn text_on_clip(args: &Vec<AtCommandMember>) -> String {
    let text = match args.len() { 
        0 => "executed: git diff".to_string(),
        1 => format!("executed: git diff {}", args[0].text),
        _ => "".to_string(),
    };
    text
}

pub async fn last_accessed_project(ccx: &mut AtCommandsContext) -> Result<String, String> {
    let p_paths = get_project_paths(ccx).await;
    if p_paths.is_empty() {
        return Err("No project paths found. Try again later".to_string());
    }
    if let Some(l_used_file) = ccx.global_context.read().await.documents_state.last_accessed_file.lock().unwrap().clone() {
        for p_path in p_paths.iter() {
            if l_used_file.starts_with(&p_path) {
                return Ok(p_path.to_string_lossy().to_string());
            }
        }
        warn!("last accessed file: {:?} is out of any of project paths available:\n{}", l_used_file, p_paths.into_iter().map(|x|x.to_string_lossy().to_string()).collect::<Vec<_>>().join("\n"));
    } else {
        warn!("no last accessed file found");
    }
    todo!();
}

#[async_trait]
impl AtCommand for AtDiff {
    fn params(&self) -> &Vec<Arc<AMutex<dyn AtParam>>> {
        &self.params
    }

    async fn execute(&self, ccx: &mut AtCommandsContext, cmd: &mut AtCommandMember, args: &mut Vec<AtCommandMember>) -> Result<(Vec<ContextEnum>, String), String> {
        // TODO: take the project path user interacted more recently with
        let project_path = match get_project_paths(ccx).await.get(0) {
            Some(path) => path.to_str().unwrap().to_string(),
            None => {
                cmd.ok = false; cmd.reason = Some("Project path is empty".to_string());
                args.clear();
                return Err("Project path is empty".to_string());
            }
        };
        info!("project_path: {}", project_path);
        let diff_chunks = match args.iter().take_while(|arg| arg.text != "\n").take(2).count() {
            0 => {
                // No arguments: git diff for all tracked files
                args.clear();
                execute_git_diff(&project_path, &[]).await.map_err(|e|format!("Couldn't execute git diff.\nError: {}", e))
            },
            1 => {
                // TODO: if file_path is rel, complete it
                // 1 argument: git diff for a specific file
                args.truncate(1);
                let file_path = &args[0].text;
                execute_git_diff(&project_path, &[file_path]).await.map_err(|e|format!("Couldn't execute git diff {}.\nError: {}", file_path, e))
            },
            _ => {
                cmd.ok = false; cmd.reason = Some("Invalid number of arguments".to_string());
                args.clear();
                return Err("Invalid number of arguments".to_string()); 
            },
        }?;

        info!("executed @diff {:?}", args);
        Ok((diff_chunks.into_iter().map(ContextEnum::DiffChunk).collect(), text_on_clip(args)))
    }

    fn depends_on(&self) -> Vec<String> {
        vec![]
    }
}
