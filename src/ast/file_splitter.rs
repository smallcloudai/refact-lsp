use md5;
use tracing::info;
use crate::ast::treesitter::parsers::get_ast_parser_by_filename;

use crate::files_in_workspace::Document;
use crate::vecdb::file_splitter::FileSplitter;
use crate::vecdb::structs::SplitResult;

fn str_hash(s: &String) -> String {
    let digest = md5::compute(s);
    format!("{:x}", digest)
}

pub struct AstBasedFileSplitter {
    soft_window: usize,
    // hard_window: usize,
    fallback_file_splitter: FileSplitter,
}

impl AstBasedFileSplitter {
    pub fn new(window_size: usize, soft_limit: usize) -> Self {
        Self {
            soft_window: window_size,
            // hard_window: window_size + soft_limit,
            fallback_file_splitter: FileSplitter::new(window_size, soft_limit),
        }
    }

    pub async fn split(&self, doc: &Document) -> Result<Vec<SplitResult>, String> {
        let mut doc = doc.clone();
        let path = doc.path.clone();
        let mut parser = match get_ast_parser_by_filename(&path) {
            Ok(parser) => parser,
            Err(_) => {
                info!("cannot find a parser for {:?}, using simple file splitter", path);
                return self.fallback_file_splitter.split(&doc).await;
            }
        };
        let text = match doc.get_text_or_read_from_disk().await {
            Ok(s) => s,
            Err(err) => {
                return Err(err.to_string());
            }
        };
        let symbols = parser.parse(text.as_str(), &path);
        let mut chunks = Vec::new();
        let mut split_normally: usize = 0;
        let mut split_using_fallback: usize = 0;
        let mut split_errors: usize = 0;
        for symbol in symbols.iter().map(|s| s.read()
            .expect("cannot read symbol")
            .symbol_info_struct()) {
            let content = match symbol.get_content().await {
                Ok(content) => content,
                Err(err) => {
                    split_errors += 1;
                    info!("cannot retrieve symbol's content {}", err);
                    continue;
                }
            };
            if content.len() > self.soft_window {
                let mut temp_doc = Document::new(&doc.path, Some("unknown".to_string()));
                temp_doc.update_text(&content);
                match self.fallback_file_splitter.split(&temp_doc).await {
                    Ok(mut res) => {
                        for r in res.iter_mut() {
                            r.start_line += symbol.full_range.start_point.row as u64;
                            r.end_line += symbol.full_range.start_point.row as u64;
                        }
                        chunks.extend(res)
                    }
                    Err(err) => {
                        info!("{}", err);
                    }
                }
                split_using_fallback += 1;
                continue;
            } else {
                split_normally += 1;
                chunks.push(SplitResult {
                    file_path: doc.path.clone(),
                    window_text: content.clone(),
                    window_text_hash: str_hash(&content),
                    start_line: symbol.full_range.start_point.row as u64,
                    end_line: symbol.full_range.end_point.row as u64,
                });
            }
        }
        let last_30_chars = crate::nicer_logs::last_n_chars(&doc.path.display().to_string(), 30);
        let message = format!("split {last_30_chars} by definitions {split_normally}, fallback {split_using_fallback}, errors {split_errors}");
        info!(message);

        Ok(chunks)
    }
}
