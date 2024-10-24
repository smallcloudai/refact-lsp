use std::path::PathBuf;
use std::sync::Arc;

use crate::call_validation::DiffChunk;
use crate::tools::tool_patch_aux::diff_structs::{diff_blocks_to_diff_chunks, DiffBlock, DiffLine, LineType};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use tracing::error;

use crate::global_context::GlobalContext;
use crate::tools::tool_patch_aux::fs_utils::read_file;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub enum SectionType {
    Original,
    Modified,
}

#[derive(Clone, Debug)]
pub struct EditSection {
    hunk: Vec<String>,
    type_: SectionType,
}

fn process_fenced_block(
    lines: &[&str],
    start_line_num: usize,
    is_original: bool,
) -> (usize, EditSection) {
    let mut line_num = start_line_num;
    while line_num < lines.len() {
        if lines[line_num].starts_with("```") {
            break;
        }
        line_num += 1;
    }
    (
        line_num + 1,
        EditSection {
            hunk: lines[start_line_num..line_num].iter().map(|x| x.to_string()).collect(),
            type_: if is_original { SectionType::Original } else { SectionType::Modified },
        }
    )
}

fn get_edit_sections(content: &str) -> Vec<EditSection> {
    let lines: Vec<&str> = content.lines().collect();
    let mut line_num = 0;
    let mut sections: Vec<EditSection> = vec![];
    while line_num < lines.len() {
        while line_num < lines.len() {
            let line = lines[line_num];
            if line.contains("Original Section") {
                let (new_line_num, section) = process_fenced_block(&lines, line_num + 2, true);
                line_num = new_line_num;
                sections.push(section);
                break;
            }
            if line.contains("Modified Section") {
                let (new_line_num, section) = process_fenced_block(&lines, line_num + 2, false);
                line_num = new_line_num;
                sections.push(section);
                break;
            }
            line_num += 1;
        }
    }
    sections
}

fn search_block_line_by_line(file_text: &Vec<String>, block_to_find: &Vec<String>) -> Result<Vec<(usize, usize, Vec<String>)>, String> {
    let mut found: Vec<(usize, usize, Vec<String>)> = vec![];
    let mut block_index = 0;
    let mut current_start = None;
    let mut current_block = vec![];

    for (file_index, file_line) in file_text.iter().enumerate() {
        if file_line.trim_start() == block_to_find[block_index].trim_start() {
            if current_start.is_none() {
                current_start = Some(file_index);
            }
            current_block.push(file_line.clone());
            block_index += 1;

            if block_index == block_to_find.len() {
                break;
            }
        } else {
            if !current_block.is_empty() {
                found.push((
                    current_start.unwrap(),
                    file_index,
                    std::mem::take(&mut current_block),
                ));
                current_start = None;
                current_block.clear();
            }
        }
    }
    if !current_block.is_empty() {
        found.push((
            current_start.unwrap(),
            file_text.len(),
            std::mem::take(&mut current_block),
        ));
    }

    if found.is_empty() {
        Err(format!("Block not found in the file text: {:?}", block_to_find))
    } else {
        Ok(found)
    }
}

async fn sections_to_diff_blocks(
    gcx: Arc<ARwLock<GlobalContext>>,
    sections: &Vec<EditSection>,
    filename: &PathBuf,
) -> Result<Vec<DiffBlock>, String> {
    let mut diff_blocks = vec![];
    let file_lines = read_file(gcx.clone(), filename.to_string_lossy().to_string())
        .await
        .map(|x| x.file_content.lines().into_iter()
            .map(|x| {
                if let Some(stripped_row) = x.to_string()
                    .replace("\r\n", "\n")
                    .strip_suffix("\n") {
                    stripped_row.to_string()
                } else {
                    x.to_string()
                }
            })
            .collect::<Vec<_>>()
        )?;
    let mut errors: Vec<String> = vec![];
    for (idx, sections) in sections.iter().chunks(2).into_iter()
        .map(|x| x.collect::<Vec<_>>()).enumerate() {
        let orig_section = sections.get(0).ok_or("No original section found")?;
        let modified_section = sections.get(1).ok_or("No modified section found")?;
        if orig_section.type_ != SectionType::Original || modified_section.type_ != SectionType::Modified {
            return Err("section types are messed up, try to regenerate the diff".to_string());
        }
        let orig_section_span = orig_section.hunk.iter()
            .map(|x| x.trim_start().to_string())
            .collect::<Vec<_>>();
        let mut start_offset = None;
        for file_line_idx in 0..=file_lines.len() - orig_section.hunk.len() {
            let file_lines_span = file_lines[file_line_idx..file_line_idx + orig_section.hunk.len()]
                .iter()
                .map(|x| x.trim_start().to_string())
                .collect::<Vec<_>>();
            if file_lines_span == orig_section_span {
                start_offset = Some(file_line_idx);
                break;
            }
        }
        if let Some(start_offset) = start_offset {
            diff_blocks.push(DiffBlock {
                file_name_before: filename.clone(),
                file_name_after: filename.clone(),
                action: "edit".to_string(),
                diff_lines: file_lines
                    [start_offset..start_offset + orig_section.hunk.len()]
                    .iter()
                    .enumerate()
                    .map(|(idx, x)| DiffLine {
                        line: x.clone(),
                        line_type: LineType::Minus,
                        file_line_num_idx: Some(start_offset + idx),
                        correct_spaces_offset: None,
                    })
                    .chain(modified_section
                        .hunk
                        .iter()
                        .map(|x| DiffLine {
                            line: x.clone(),
                            line_type: LineType::Plus,
                            file_line_num_idx: Some(start_offset),
                            correct_spaces_offset: None,
                        }))
                    .collect::<Vec<_>>(),
                hunk_idx: idx,
                file_lines: Arc::new(vec![]),
            })
        } else {
            match search_block_line_by_line(&file_lines, &orig_section.hunk) {
                Ok(res) => {
                    let mut err = format!("This section wasn't found in the original file content:\n```\n{}\n```\n", orig_section.hunk.iter().join("\n"));
                    err += "Split it into multiple sections like this:\n";
                    for (_, _, found_block) in res {
                        err += &format!("### Original Section (to be replaced)\n```\n{}\n```\n", found_block.join("\n"));
                        err += &"### Modified Section (to replace with)\n```\n[Modified code section]\n```\n".to_string();
                    }
                    errors.push(err.clone());
                    error!("{}", err);
                    continue;
                }
                Err(_) => {
                    let err = format!("This section wasn't found in the original file content:\n```\n{}\n```\n", orig_section.hunk.iter().join("\n"));
                    errors.push(err.clone());
                    error!("{}", err);
                    continue;
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(diff_blocks)
    } else {
        Err(errors.join("\n"))
    }
}

pub struct BlocksOfCodeParser {}

impl BlocksOfCodeParser {
    pub fn prompt() -> String {
        let prompt = r#"You will receive a file containing code with one or more modified sections. Your task is to identify, describe, and extract all sections of the original code that correspond to the modified sections provided. Follow the steps below to ensure accuracy and clarity in your response. Carefully read the hints if they're given, they contain important information about the changes (i.e. exact spots where to paste new code).

## Steps
1. **Locate Modified Sections:** Carefully review the provided code file and identify all sections that differ between the original and modified versions.
2. **Output Modifications:** Prepare the output using the format specified below. Ensure the original formatting is preserved for both the original and modified sections.

## Output Format
### Original Section (to be replaced)
```
[Original code section]
```
### Modified Section (to replace with)
```
[Modified code section]
```

## Notes
- Where possible, replace entire functions instead of making multiple small changes within them for better clarity.
- Split a single modified section into multiple if changes are located in different parts of the original file.
- Preserve the original indentation and formatting to avoid introducing errors during code replacement.
- Do not skip any modification, even if they are invalid or insufficient!
- If there is new code added without any modifications, use this format:
### Original Section (to be replaced)
```
[an old section of the code where you need to insert new code]
```
### Modified Section (to replace with)
```
[an old section of the code + new code]
```"#.to_string();
        prompt
    }

    pub fn followup_prompt(error_message: &String) -> String {
        let prompt = r#"{error_message}
1. Provide your thoughts why these sections couldn't be found. 
2. Rewrite those sections. The best way to do that correctly is to split them into smaller pieces. 
I.e., if there are many functions in a single section - make a separate section for each function
3. Copy other correct sections without any changes
4. Follow the hints to find the spot where to paste the code
5. Keep the output format the same is in the initial prompt and don't forget to replace [Modified code section] with the real modified code:
## Output Format
### Original Section (to be replaced)
```
[Original code section]
```
### Modified Section (to replace with)
```
[Modified code section]
```"#.to_string();
        prompt.replace("{error_message}", error_message)
    }

    pub async fn parse_message(
        gcx: Arc<ARwLock<GlobalContext>>,
        content: &str,
        filename: &PathBuf,
    ) -> Result<Vec<DiffChunk>, String> {
        let sections = get_edit_sections(content);
        if sections.is_empty() {
            return Err("no sections found, probably an empty diff".to_string());
        }
        let diff_blocks = sections_to_diff_blocks(gcx, &sections, &filename).await?;
        let chunks = diff_blocks_to_diff_chunks(&diff_blocks)
            .into_iter()
            .unique()
            .collect::<Vec<_>>();
        if chunks.is_empty() {
            return Err("no chunks found, probably an empty diff".to_string());
        }
        Ok(chunks)
    }
}
