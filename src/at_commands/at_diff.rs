use std::process::Command;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::json;
use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use tokio::sync::Mutex as AMutex;
use tracing::info;
use crate::at_commands::at_file::{AtParamFilePath, parameter_repair_candidates};
use crate::call_validation::{ChatMessage, ContextFile};

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
    info!("EXEC: git diff");
    let command = Command::new("git").arg("diff").arg(path).output().map_err(|e|e.to_string())?;

    let output = String::from_utf8(command.stdout).map_err(|e|e.to_string())?;

    Ok(output)
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
    async fn execute(&self, _query: &String, args: &Vec<String>, top_n: usize, context: &AtCommandsContext) -> Result<ChatMessage, String> {
        let can_execute = self.can_execute(args, context).await;
        if !can_execute {
            return Err("incorrect arguments".to_string());
        }
        let correctable_file_path = args[0].clone();
        let candidates = parameter_repair_candidates(&correctable_file_path, context, top_n).await;
        if candidates.len() == 0 {
            info!("parameter {:?} is uncorrectable :/", &correctable_file_path);
            return Err(format!("parameter {:?} is uncorrectable :/", &correctable_file_path));
        }
        let file_path = candidates[0].clone();
        let res = exec_git_diff_with_path(&file_path).await?;
        
        let context_file = ContextFile {
            file_name: file_path.clone(),
            file_content: res.clone(),
            line1: 0,
            line2: res.lines().count(),
            symbol: "".to_string(),
            usefulness: 100.
        };
        Ok(ChatMessage {
            role: "context_file".to_string(),
            content: json!(vec![context_file]).to_string(),
        })
    }
}
