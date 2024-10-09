use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::DiffChunk;
use crate::tools::tool_patch_aux::diff_structs::chunks_from_diffs;
use tracing::error;
use crate::global_context::GlobalContext;
use crate::tools::tool_patch_aux::fs_utils::read_file;

fn get_edit_sections(content: &str) -> Option<Vec<String>> {
    fn process_fenced_block(
        lines: &[&str],
        start_line_num: usize,
    ) -> Vec<String> {
        let mut line_num = start_line_num;
        while line_num < lines.len() {
            if lines[line_num].starts_with("```") {
                break;
            }
            line_num += 1;
        }
        lines[start_line_num..line_num].iter().map(|x| x.to_string()).collect()
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut line_num = 0;
    while line_num < lines.len() {
        let line = lines[line_num];
        if line.contains("Modified file") {
            return Some(process_fenced_block(&lines, line_num + 2));
        }
        line_num += 1;
    }
    None
}

async fn modified_code_to_diff_blocks(
    gcx: Arc<ARwLock<GlobalContext>>,
    modified_code: &Vec<String>,
    filename: &PathBuf,
) -> Result<Vec<DiffChunk>, String> {
    let context_file = read_file(gcx.clone(), filename.to_string_lossy().to_string()).await
        .map_err(|e| format!("cannot read file to modify: {:?}.\nError: {e}", filename))?;
    let file_path = PathBuf::from(&context_file.file_name);
    let line_ending = if context_file.file_content.contains("\r\n") { "\r\n" } else { "\n" };
    let code = modified_code.join(line_ending);
    let diffs = diff::lines(&context_file.file_content, &code);
    chunks_from_diffs(file_path, diffs)
}

pub struct WholeFileParser {}

impl WholeFileParser {
    pub fn prompt() -> String {
        let prompt = r#"You will receive a file containing code and one or more modified sections of this file. 
Modified sections could contain unfinished code, could be out of order of the original file, or could constitute complete functions. 
Your task is to modify the original file according all modified sections. 
You must introduce all listed changes event if they are minor or contain errors! 
Before the modification, you must list all the changes you must introduce to the file
Output Format:
# Modified file
```
[code]
```"#.to_string();
        prompt
    }

    pub async fn parse_message(
        gcx: Arc<ARwLock<GlobalContext>>,
        content: &str,
        filename: &PathBuf,
    ) -> Result<Vec<DiffChunk>, String> {
        let modified_code = get_edit_sections(content);
        if let Some(code) = modified_code {
            modified_code_to_diff_blocks(gcx.clone(), &code, &filename).await
        } else {
            error!("no code block found");
            Err("no code block found".to_string())
        }
    }
}
