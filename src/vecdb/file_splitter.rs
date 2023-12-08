use std::path::PathBuf;
use tokio::fs::read_to_string;
use crate::vecdb::structs::SplitResult;
use md5;

fn str_hash(s: &String) -> String {
    let digest = md5::compute(s);
    format!("{:x}", digest)
}

pub struct FileSplitter {
    soft_window: usize,
    hard_window: usize,
}

impl FileSplitter {
    pub fn new(window_size: usize, soft_limit: usize) -> Self {
        Self {
            soft_window: window_size,
            hard_window: window_size + soft_limit,
        }
    }

    pub async fn split(&self, file_path: &PathBuf) -> Result<Vec<SplitResult>, String> {
        let text = match read_to_string(file_path).await {
            Ok(s) => s,
            Err(e) => return Err(e.to_string())
        };

        let mut chunks = Vec::new();
        let mut batch = Vec::new();
        let mut batch_size = 0;
        let mut soft_batch = Vec::new();
        let mut current_line_number: u64 = 0;
        for line in text.lines() {
            batch_size += line.len();
            if batch_size > self.soft_window {
                soft_batch.push(line.to_string());
            } else {
                batch.push(line.to_string());
            }

            if batch_size >= self.hard_window {
                let best_break_line_n = soft_batch.iter()
                    .rposition(|l| l.trim().is_empty())
                    .unwrap_or(soft_batch.len());

                let (remaining, to_next_batch) = soft_batch.split_at(best_break_line_n);
                batch.extend_from_slice(remaining);

                let start_line = current_line_number;
                let end_line = start_line + batch.len() as u64;
                current_line_number += batch.len() as u64;

                chunks.push(SplitResult {
                    file_path: file_path.clone(),
                    window_text: batch.join("\n"),
                    window_text_hash: str_hash(&batch.join("\n")),
                    start_line,
                    end_line,
                });

                batch = to_next_batch.to_vec();
                soft_batch.clear();
                batch_size = batch.iter().map(|s| s.len()).sum();
            }
        }

        if !batch.is_empty() || !soft_batch.is_empty() {
            batch.extend(soft_batch);
            let start_line = current_line_number;
            let end_line = start_line + batch.len() as u64;

            chunks.push(SplitResult {
                file_path: file_path.clone(),
                window_text: batch.join("\n"),
                window_text_hash: str_hash(&batch.join("\n")),
                start_line,
                end_line,
            });
        }

        Ok(chunks)
    }
}
