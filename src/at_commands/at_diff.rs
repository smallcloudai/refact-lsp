use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use async_trait::async_trait;
use crate::at_commands::at_commands::{AtCommand, AtResponse, AtCommandsContext, AtParam};
use tokio::sync::Mutex as AMutex;
use tracing::info;
use crate::at_commands::at_file::{AtParamFilePath, parameter_repair_candidates};
use crate::call_validation::ContextDiff;

pub struct AtDiff {
    pub name: String,
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtDiff {
    pub fn new() -> Self {
        AtDiff {
            name: "@diff".to_string(),
            params: vec![
                Arc::new(AMutex::new(AtParamFilePath::new()))
            ],
        }
    }
}

async fn exec_git_diff_with_path(path: &String) -> Result<String, String> {
    info!("EXEC: git diff {}", path);
    let path_buf = PathBuf::from(path);
    if let Some(parent_dir) = path_buf.clone().parent() {
        let command = Command::new("git")
            .current_dir(parent_dir)
            .arg("diff")
            .arg(path)
            .output()
            .map_err(|e|e.to_string())?;

        let output = String::from_utf8(command.stdout).map_err(|e|e.to_string())?;

        Ok(output)
    } else {
        Err("Failed to get parent directory".to_string())
    }
}

#[async_trait]
impl AtCommand for AtDiff {
    fn name(&self) -> &String {
        &self.name
    }

    fn params(&self) -> &Vec<Arc<AMutex<dyn AtParam>>>
    {
        &self.params
    }
    
    async fn can_execute(&self, args: &Vec<String>, _context: &AtCommandsContext) -> bool {
        args.len() == 1
    }
    async fn execute(&self, _query: &String, args: &Vec<String>, top_n: usize, context: &AtCommandsContext) -> Result<Vec<AtResponse>, String> {
        let can_execute = self.can_execute(args, context).await;
        if !can_execute {
            return Err("incorrect arguments".to_string());
        }
        let correctable_file_path = args.get(0).unwrap().clone();
        let candidates = parameter_repair_candidates(&correctable_file_path, context, top_n).await;
        if candidates.len() == 0 {
            let msg = format!("parameter {:?} is uncorrectable :/", &correctable_file_path);
            info!(msg);
            return Err(msg);
        }
        let file_path = candidates[0].clone();
        let res = exec_git_diff_with_path(&file_path).await?;
        
        let context_diff = ContextDiff {
            file_name: file_path.clone(),
            content: res.clone(),
        };
        
        Ok(vec![context_diff].into_iter().map(AtResponse::ContextDiff).collect())
    }
}
