use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;

use crate::vecdb::structs::SplitResult;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileSplitter {
    window_size: usize,
}

fn guess_newline_delimiter(file_content: &String) -> Result<String, String> {
    // Check for CRLF first, as it contains LF
    if file_content.contains("\r\n") {
        Ok("\r\n".to_string())
    } else if file_content.contains("\n") {
        Ok("\n".to_string())
    } else {
        Err("Unknown".to_string())
    }
}

fn str_hash(s: &String) -> String {
    let digest = md5::compute(s);
    format!("{:x}", digest)
}

impl FileSplitter {
    pub fn new(window_size: usize) -> Self {
        FileSplitter { window_size }
    }

    pub async fn split(&self, file_path: &PathBuf) -> Result<Vec<SplitResult>, String> {
        let file_content = match read_to_string(file_path).await {
            Ok(s) => s,
            Err(e) => return Err(e.to_string())
        };

        let mut chunks = Vec::new();
        let mut delimiter = self.max_empty_lines(&file_content);
        chunks = self.split_by_empty_lines(&file_content, file_path, delimiter, 0);

        delimiter -= 1;
        while delimiter > 1 {
            chunks = self.split_large_chunks(chunks, file_path, delimiter);
            if chunks.iter().all(|chunk| chunk.window_text.len() <= self.window_size) {
                break;
            }
            delimiter -= 1;
        }

        Ok(chunks.iter()
            .filter(|s| !s.window_text.is_empty())
            .map(
                |s| {
                    let text_stripped = s.window_text.trim().to_string();
                    SplitResult {
                        window_text_hash: str_hash(&text_stripped),
                        window_text: text_stripped,
                        start_line: s.start_line,
                        end_line: s.end_line,
                        file_path: s.file_path.clone(),
                    }
                }
            )
            .collect())
    }

    fn max_empty_lines(&self, content: &str) -> i32 {
        let mut max_count = 0;
        let mut current_count = 0;
        let mut prev_was_newline = false;

        let mut chars = content.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\n' || (c == '\r' && chars.peek() == Some(&'\n')) {
                if prev_was_newline {
                    current_count += 1;
                } else {
                    current_count = 1;
                    prev_was_newline = true;
                }
                max_count = max_count.max(current_count);
            } else {
                prev_was_newline = false;
            }

            // Skip the '\n' character if the current character is '\r'
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
        }

        max_count
    }

    fn split_by_empty_lines(&self, content: &String, file_path: &PathBuf, delimiter: i32, offset: u64) -> Vec<SplitResult> {
        let delimiter_str = guess_newline_delimiter(&content);

        if delimiter == 0 || delimiter_str.is_err() {
            return vec![SplitResult {
                window_text: content.to_string(),
                window_text_hash: str_hash(&content.to_string()),
                start_line: offset,
                end_line: offset + (content.lines().count() as i64 - 1).max(0) as u64,
                file_path: file_path.clone(),
            }];
        }

        let mut offset_sum = offset;
        content
            .split(&delimiter_str.unwrap().repeat(delimiter as usize))
            .map(
                |s| {
                    let end_line = offset_sum + (s.lines().count() as i64 - 1).max(0) as u64;
                    let res = SplitResult {
                        window_text: s.to_string(),
                        window_text_hash: str_hash(&s.to_string()),
                        start_line: offset_sum,
                        end_line: end_line,
                        file_path: file_path.clone(),
                    };
                    offset_sum = end_line + delimiter as u64;
                    return res;
                }
            )
            .collect()
    }

    fn split_large_chunks(&self, chunks: Vec<SplitResult>, file_path: &PathBuf, delimiter: i32) -> Vec<SplitResult> {
        chunks
            .into_iter()
            .flat_map(|chunk| {
                if chunk.window_text.len() > self.window_size {
                    self.split_by_empty_lines(&chunk.window_text, file_path, delimiter - 1, chunk.start_line)
                } else {
                    vec![chunk]
                }
            }).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    fn create_temp_file_with_content(content: &str) -> (NamedTempFile, PathBuf) {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{}", content).unwrap();  // Using `write!` to add content without an extra newline at the end.
        let file_path = file.path().to_owned(); // Directly assign the PathBuf to a variable
        (file, file_path)
    }

    #[tokio::test]
    async fn test_empty_file() {
        let splitter = FileSplitter::new(10);
        let (temp_file, file_path) = create_temp_file_with_content("");
        let result = splitter.split(&file_path).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_file_smaller_than_window_size() {
        let splitter = FileSplitter::new(3);
        let (temp_file, file_path) = create_temp_file_with_content("Hello");
        let result = splitter.split(&file_path).await.unwrap();
        assert_eq!(result, vec![
            SplitResult {
                window_text: "Hello".to_string(),
                window_text_hash: str_hash(&"Hello".to_string()),
                start_line: 0,
                end_line: 0,
                file_path: file_path
            }
        ]);
    }

    #[tokio::test]
    async fn test_file_with_exact_window_size() {
        let splitter = FileSplitter::new(5);
        let (temp_file, file_path) = create_temp_file_with_content("Hello");
        let result = splitter.split(&file_path).await.unwrap();
        assert_eq!(result, vec![
            SplitResult {
                window_text: "Hello".to_string(),
                window_text_hash: str_hash(&"Hello".to_string()),
                start_line: 0,
                end_line: 0,
                file_path: file_path
            },
        ]);
    }

    #[tokio::test]
    async fn test_file_with_a_single_empty_line() {
        let splitter = FileSplitter::new(10);
        let (temp_file, file_path) = create_temp_file_with_content("Hello\nWorld");
        let result = splitter.split(&file_path).await.unwrap();
        assert_eq!(result, vec![
            SplitResult {
                window_text: "Hello".to_string(),
                window_text_hash: str_hash(&"Hello".to_string()),
                start_line: 0,
                end_line: 0,
                file_path: file_path.clone()
            },
            SplitResult {
                window_text: "World".to_string(),
                window_text_hash: str_hash(&"World".to_string()),
                start_line: 1,
                end_line: 1,
                file_path: file_path
            },
        ]);
    }

    #[tokio::test]
    async fn test_splitting_by_maximum_empty_lines() {
        let splitter = FileSplitter::new(30);
        let (temp_file, file_path) = create_temp_file_with_content("Hello\n\n\nWorld");
        let result = splitter.split(&file_path).await.unwrap();
        assert_eq!(result, vec![
            SplitResult {
                window_text: "Hello".to_string(),
                window_text_hash: str_hash(&"Hello".to_string()),
                start_line: 0,
                end_line: 0,
                file_path: file_path.clone()
            },
            SplitResult {
                window_text: "World".to_string(),
                window_text_hash: str_hash(&"World".to_string()),
                start_line: 3,
                end_line: 3,
                file_path: file_path
            },
        ]);
    }

    #[tokio::test]
    async fn test_splitting_enough_win_size() {
        let splitter = FileSplitter::new(20);
        let content = "Hello\n\nWorld\n\n\nThis is a test\n";
        let (temp_file, file_path) = create_temp_file_with_content(content);
        let result = splitter.split(&file_path).await.unwrap();
        assert_eq!(result, vec![
            SplitResult {
                window_text: "Hello\n\nWorld".to_string(),
                window_text_hash: str_hash(&"Hello\n\nWorld".to_string()),
                start_line: 0,
                end_line: 2,
                file_path: file_path.clone()
            },
            SplitResult {
                window_text: "This is a test".to_string(),
                window_text_hash: str_hash(&"This is a test".to_string()),
                start_line: 5,
                end_line: 5,
                file_path: file_path
            },
        ]);
    }

    #[tokio::test]
    async fn test_splitting_not_enough_win_size() {
        let splitter = FileSplitter::new(5);
        let content = "Hello\n\nWorld\n\n\nThis is a test\n";
        let (temp_file, file_path) = create_temp_file_with_content(content);
        let result = splitter.split(&file_path).await.unwrap();
        assert_eq!(result, vec![
            SplitResult {
                window_text: "Hello".to_string(),
                window_text_hash: str_hash(&"Hello".to_string()),
                start_line: 0,
                end_line: 0,
                file_path: file_path.clone()
            },
            SplitResult {
                window_text: "World".to_string(),
                window_text_hash: str_hash(&"World".to_string()),
                start_line: 2,
                end_line: 2,
                file_path: file_path.clone()
            },
            SplitResult {
                window_text: "This is a test".to_string(),
                window_text_hash: str_hash(&"This is a test".to_string()),
                start_line: 5,
                end_line: 5,
                file_path: file_path
            },
        ]);
    }
}
