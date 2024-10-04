use std::path::PathBuf;
use std::sync::Arc;
use strsim::normalized_damerau_levenshtein;

use crate::call_validation::DiffChunk;
use crate::files_in_workspace::read_file_from_disk;
use crate::privacy::PrivacySettings;
use crate::tools::tool_patch_aux::diff_structs::{diff_blocks_to_diff_chunks, DiffBlock, DiffLine, LineType};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tracing::warn;

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
            if line.starts_with("### Original Section (to be replaced)") {
                let (new_line_num, section) = process_fenced_block(&lines, line_num + 2, true);
                line_num = new_line_num;
                sections.push(section);
                break;
            }
            if line.starts_with("### Modified Section (to replace with)") {
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

async fn sections_to_diff_blocks(
    sections: &Vec<EditSection>,
    filename: &PathBuf,
    privacy_settings: Arc<PrivacySettings>,
) -> Result<Vec<DiffBlock>, String> {
    let mut diff_blocks = vec![];
    let file_lines = read_file_from_disk(privacy_settings.clone(), &filename)
        .await
        .map(|x| x
            .lines()
            .into_iter()
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
    for (idx, sections) in sections
        .iter()
        .chunks(2)
        .into_iter()
        .map(|x| x.collect::<Vec<_>>()).enumerate() {
        let orig_section = sections.get(0).ok_or("No original section found")?;
        let modified_section = sections.get(1).ok_or("No modified section found")?;
        if orig_section.type_ != SectionType::Original || modified_section.type_ != SectionType::Modified {
            return Err("section types are messed up, try to regenerate the diff".to_string());
        }
        let orig_section_span = orig_section.hunk
            .iter()
            .map(|x| x.trim_start().to_string())
            .collect::<Vec<_>>();
        let mut start_offset = None;
        let mut distances = vec![];
        for file_line_idx in 0..=file_lines.len() - orig_section.hunk.len() {
            let file_lines_span = file_lines[file_line_idx..file_line_idx + orig_section.hunk.len()]
                .iter()
                .map(|x| x.trim_start().to_string())
                .collect::<Vec<_>>();
            if file_lines_span == orig_section_span {
                start_offset = Some(file_line_idx);
                break;
            } else {
                let orig_section_span_str = orig_section_span.join("\n");
                let file_lines_span_str = file_lines_span.join("\n");
                distances.push(normalized_damerau_levenshtein(&orig_section_span_str, &file_lines_span_str));
            }
        }
        if let Some(start_offset) = start_offset {
            let lines = file_lines
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
                .collect::<Vec<_>>();
            diff_blocks.push(DiffBlock {
                file_name_before: filename.clone(),
                file_name_after: filename.clone(),
                action: "edit".to_string(),
                diff_lines: lines,
                hunk_idx: idx,
                file_lines: Arc::new(vec![]),
            })
        } else {
            warn!("section not found in file {}, distances: {:?}", filename.to_string_lossy(), distances);
            continue
        }
    }
    Ok(diff_blocks)
}

pub struct BlocksOfCodeParser {}

impl BlocksOfCodeParser {
    pub fn prompt(
        workspace_projects_dirs: Vec<String>
    ) -> String {
        assert_eq!(workspace_projects_dirs.is_empty(), false);
        let prompt = r#"You will receive a file containing code with one or several modified sections. Your task is to identify and extract all sections in the provided original code that need to be replaced based on the modified sections given. Follow the steps below to ensure accuracy and clarity in your response.

## Steps
1. **Locate Modified Sections:** Carefully review the provided code file and identify all sections that differ between the original and modified versions.
2. **Describe Modifications:** Clearly describe each identified modification, focusing on what has changed in terms of functionality, structure, or imports.
3. **Output Modifications:** After describing all modifications, prepare the output using the format specified below. Ensure the original formatting is preserved for both the original and modified sections.

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
- Prefer to rewrite entire functions instead of making many small changes within them.
- If new lines are added to the original file, include a few lines of the original code along with the new lines.
- Ensure that all modifications are extracted accurately without altering their structure.
- Retain the original indentation and formatting to ensure clarity and prevent errors when replacing code sections.
- Strictly follow the steps, and don't forget to describe modifications first!"#.to_string();
        prompt
    }

    pub async fn parse_message(
        content: &str,
        filename: &PathBuf,
        privacy_settings: Arc<PrivacySettings>,
    ) -> Result<Vec<DiffChunk>, String> {
        let edits = get_edit_sections(content);
        let diff_blocks = sections_to_diff_blocks(&edits, &filename, privacy_settings).await?;
        let chunks = diff_blocks_to_diff_chunks(&diff_blocks)
            .into_iter()
            .unique()
            .collect::<Vec<_>>();
        Ok(chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;
    use std::sync::Arc;
    use crate::privacy::PrivacySettings;

    #[tokio::test]
    async fn test_get_edit_sections() {
        let content = r#"
### Original Section (to be replaced)
```
let a = 1;
let b = 2;
```

### Modified Section (to replace with)
```
let a = 10;
let b = 20;
```
"#;
        let sections = get_edit_sections(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].type_, SectionType::Original);
        assert_eq!(sections[1].type_, SectionType::Modified);
        assert_eq!(sections[0].hunk, vec!["let a = 1;", "let b = 2;"]);
        assert_eq!(sections[1].hunk, vec!["let a = 10;", "let b = 20;"]);
    }

    #[tokio::test]
    async fn test_sections_to_diff_blocks() {
        let content = r#"
### Original Section (to be replaced)
```
let a = 1;
let b = 2;
```
### Modified Section (to replace with)
```
let a = 10;
let b = 20;
```
"#;
        let sections = get_edit_sections(content);
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "let a = 1;\nlet b = 2;").unwrap();
        let filename = temp_file.path().to_path_buf();
        let privacy_settings = Arc::new(PrivacySettings::allow_all());

        let diff_blocks = sections_to_diff_blocks(&sections, &filename, privacy_settings).await.unwrap();
        assert_eq!(diff_blocks.len(), 1);
        let diff_block = &diff_blocks[0];
        assert_eq!(diff_block.diff_lines.len(), 4);
        assert_eq!(diff_block.diff_lines[0].line_type, LineType::Minus);
        assert_eq!(diff_block.diff_lines[1].line_type, LineType::Minus);
        assert_eq!(diff_block.diff_lines[2].line_type, LineType::Plus);
        assert_eq!(diff_block.diff_lines[3].line_type, LineType::Plus);
    }

    #[tokio::test]
    async fn test_parse_message() {
        let content = r#"
### Original Section (to be replaced)
```
let a = 1;
let b = 2;
```
### Modified Section (to replace with)
```
let a = 10;
let b = 20;
```
"#;
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "let a = 1;\nlet b = 2;").unwrap();
        let filename = temp_file.path().to_path_buf();
        let privacy_settings = Arc::new(PrivacySettings::allow_all());

        let diff_chunks = BlocksOfCodeParser::parse_message(content, &filename, privacy_settings).await.unwrap();
        assert_eq!(diff_chunks.len(), 1);
    }
}