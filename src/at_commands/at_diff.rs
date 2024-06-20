use std::env;
use std::sync::Arc;
use std::path::PathBuf;
use tracing::info;

use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tokio::process::Command;

use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::execute_at::AtCommandMember;
use crate::call_validation::ContextEnum;
use crate::call_validation::ChatMessage;


pub struct AtDiff {
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtDiff {
    pub fn new() -> Self {
        AtDiff { params: vec![] }
    }
}

async fn execute_git_diff(project_dir: &str, args: &[&str]) -> Result<(String, String), String> {
    let output = match args.is_empty() { 
        true => {
            Command::new("git")
                .arg("diff")
                .args(args)
                .current_dir(PathBuf::from(project_dir))
                .output()
                .await
                .map_err(|e| e.to_string())?
        },
        false => {
            Command::new("git")
                .arg("diff")
                .current_dir(PathBuf::from(project_dir))
                .output()
                .await
                .map_err(|e| e.to_string())?
        }
    };
    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout).to_string(), String::from_utf8_lossy(&output.stderr).to_string());
    Ok((stdout, stderr))
}

async fn execute_diff(file1: &str, file2: &str) -> Result<(String, String), String> {
    let output = Command::new("diff")
        .arg(file1)
        .arg(file2)
        .output()
        .await
        .map_err(|e| e.to_string())?;

    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout).to_string(), String::from_utf8_lossy(&output.stderr).to_string());
    Ok((stdout, stderr))
}

// TODO we'll have the same one in at_file.rs, import 
async fn get_project_paths(ccx: &AtCommandsContext) -> Vec<PathBuf> {
    let cx = ccx.global_context.read().await;
    let workspace_folders = cx.documents_state.workspace_folders.lock().unwrap();
    workspace_folders.iter().cloned().collect::<Vec<_>>()
}

fn text_on_clip(args: &Vec<AtCommandMember>) -> String {
    let text = match args.len() { 
        0 => "executed: git diff".to_string(),
        1 => format!("executed: git diff {}", args[0].text),
        2 => format!("executed: diff {} {}", args[0].text, args[1].text),
        _ => "".to_string(),
    };
    text
}

#[async_trait]
impl AtCommand for AtDiff {
    fn params(&self) -> &Vec<Arc<AMutex<dyn AtParam>>> {
        &self.params
    }

    async fn execute(&self, ccx: &mut AtCommandsContext, cmd: &mut AtCommandMember, args: &mut Vec<AtCommandMember>) -> Result<(Vec<ContextEnum>, String), String> {
        let project_path = match get_project_paths(ccx).await.get(0) {
            Some(path) => path.to_str().unwrap().to_string(),
            None => {
                cmd.ok = false; cmd.reason = Some("Project path is empty".to_string());
                args.clear();
                return Err("Project path is empty".to_string());
            }
        };
        info!("project_path: {}", project_path);
        let output_mb = match args.len() {
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
            2 => {
                // 2 arguments: diff between two files
                args.truncate(2);
                // TODO: if file_paths are rel, complete them
                let file1 = &args[0].text;
                let file2 = &args[1].text;
                execute_diff(file1, file2).await.map_err(|e|format!("Couldn't execute diff {} {}.\nError: {}", file1, file2, e))
            },
            _ => {
                cmd.ok = false; cmd.reason = Some("Invalid number of arguments".to_string());
                args.clear();
                return Err("Invalid number of arguments".to_string()); 
            },
        };
        let (stdout, stderr) = output_mb?;
        
        let chat_message = ChatMessage::new(
            "@diff".to_string(),
            format!("{}{}", stdout, stderr),
        );

        info!("executed @diff {:?}", args);
        Ok((vec![ContextEnum::ChatMessage(chat_message)], text_on_clip(args)))
    }

    fn depends_on(&self) -> Vec<String> {
        vec![]
    }
}
