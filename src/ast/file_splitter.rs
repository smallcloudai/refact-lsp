use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::sync::RwLock as StdRwLock;
use itertools::Itertools;
use tokenizers::Tokenizer;
use tokio::sync::RwLock;
use tracing::info;
use std::cell::RefCell;
use uuid::Uuid;

use crate::ast::chunk_utils::get_chunks;
use crate::ast::treesitter::ast_instance_structs::SymbolInformation;
use crate::ast::treesitter::parsers::get_ast_parser_by_filename;
use crate::ast::treesitter::skeletonizer::make_formatter;
use crate::ast::treesitter::structs::SymbolType;
use crate::ast::treesitter::file_ast_markup::FileASTMarkup;
use crate::files_in_workspace::Document;
use crate::global_context::GlobalContext;
use crate::vecdb::vdb_file_splitter::FileSplitter;
use crate::vecdb::vdb_structs::SplitResult;

pub(crate) const LINES_OVERLAP: usize = 3;


pub struct AstBasedFileSplitter {
    fallback_file_splitter: FileSplitter,
}

pub fn lowlevel_file_markup(
    doc: &Document,
    symbols: &Vec<SymbolInformation>,
) -> Result<FileASTMarkup, String> {
    let t0 = std::time::Instant::now();
    assert!(doc.doc_text.is_some());
    let mut symbols4export: Vec<Arc<RefCell<SymbolInformation>>> = symbols.iter().map(|s| {
        Arc::new(RefCell::new(s.clone()))
    }).collect();
    let guid_to_symbol: HashMap<Uuid, Arc<RefCell<SymbolInformation>>> = symbols4export.iter().map(
        |s| (s.borrow().guid.clone(), s.clone())
    ).collect();
    fn recursive_path_of_guid(guid_to_symbol: &HashMap<Uuid, Arc<RefCell<SymbolInformation>>>, guid: &Uuid) -> String
    {
        return match guid_to_symbol.get(guid) {
            Some(x) => {
                let pname = if !x.borrow().name.is_empty() { x.borrow().name.clone() } else { x.borrow().guid.to_string()[..8].to_string() };
                let pp = recursive_path_of_guid(&guid_to_symbol, &x.borrow().parent_guid);
                format!("{}::{}", pp, pname)
            }
            None => {
                // FIXME:
                // info!("parent_guid {} not found, maybe outside of this file", guid);
                "UNK".to_string()
            }
        };
    }
    for s in symbols4export.iter_mut() {
        let symbol_path = recursive_path_of_guid(&guid_to_symbol, &s.borrow().guid);
        s.borrow_mut().symbol_path = symbol_path.clone();
    }
    // longer symbol path at the bottom => parent always higher than children
    symbols4export.sort_by(|a, b| {
        a.borrow().symbol_path.len().cmp(&b.borrow().symbol_path.len())
    });
    let x = FileASTMarkup {
        file_path: doc.doc_path.clone(),
        file_content: doc.doc_text.as_ref().unwrap().to_string(),
        symbols_sorted_by_path_len: symbols4export.iter().map(|s| {
            s.borrow().clone()
        }).collect(),
    };
    tracing::info!("file_markup {:>4} symbols in {:.3}ms for {}",
        x.symbols_sorted_by_path_len.len(),
        t0.elapsed().as_secs_f32(),
        crate::nicer_logs::last_n_chars(&doc.doc_path.to_string_lossy().to_string(),
        30));
    Ok(x)
}

impl AstBasedFileSplitter {
    pub fn new(window_size: usize) -> Self {
        Self {
            fallback_file_splitter: FileSplitter::new(window_size),
        }
    }

    pub async fn vectorization_split(
        &self,
        doc: &Document,
        tokenizer: Arc<StdRwLock<Tokenizer>>,
        _gcx_weak: Weak<RwLock<GlobalContext>>,
        tokens_limit: usize,
    ) -> Result<Vec<SplitResult>, String> {
        assert!(doc.doc_text.is_some());
        let doc_text: String = doc.text_as_string().unwrap();
        let doc_lines: Vec<String> = doc_text.split("\n").map(|x| x.to_string()).collect();
        let path = doc.doc_path.clone();

        let (mut parser, language) = match get_ast_parser_by_filename(&path) {
            Ok(parser) => parser,
            Err(_e) => {
                // info!("cannot find a parser for {:?}, using simple file splitter: {}", crate::nicer_logs::last_n_chars(&path.display().to_string(), 30), e.message);
                return self.fallback_file_splitter.vectorization_split(&doc, tokenizer.clone(), tokens_limit).await;
            }
        };

        let mut guid_to_children: HashMap<Uuid, Vec<Uuid>> = Default::default();
        let mut symbols_struct: Vec<SymbolInformation> = Default::default();
        {
            let symbols = parser.parse(doc.text_as_string().unwrap().as_str(), &path);
            let _ = symbols.into_iter().for_each(|s| {
                let s = s.read();
                guid_to_children.insert(s.guid().clone(), s.childs_guid().clone());
                symbols_struct.push(s.symbol_info_struct());
            });
        }

        let ast_markup: FileASTMarkup = match lowlevel_file_markup(&doc, &symbols_struct) {
            Ok(x) => x,
            Err(e) => {
                info!("lowlevel_file_markup failed for {:?}, using simple file splitter: {}", crate::nicer_logs::last_n_chars(&path.display().to_string(), 30), e);
                return self.fallback_file_splitter.vectorization_split(&doc, tokenizer.clone(), tokens_limit).await;
            }
        };

        let guid_to_info: HashMap<Uuid, &SymbolInformation> = ast_markup.symbols_sorted_by_path_len.iter().map(|s| (s.guid.clone(), s)).collect();
        let guids: Vec<_> = guid_to_info.iter()
            .sorted_by(|a, b| a.1.full_range.start_byte.cmp(&b.1.full_range.start_byte))
            .map(|(s, _)| s.clone()).collect();

        let mut chunks: Vec<SplitResult> = Vec::new();
        let mut unused_symbols_cluster_accumulator: Vec<&SymbolInformation> = Default::default();

        let flush_accumulator = |
            unused_symbols_cluster_accumulator_: &mut Vec<&SymbolInformation>,
            chunks_: &mut Vec<SplitResult>,
        | {
            if !unused_symbols_cluster_accumulator_.is_empty() {
                let top_row = unused_symbols_cluster_accumulator_.first().unwrap().full_range.start_point.row;
                let bottom_row = unused_symbols_cluster_accumulator_.last().unwrap().full_range.end_point.row;
                let content = doc_lines[top_row..bottom_row + 1].join("\n");
                let chunks__ = get_chunks(&content, &path, &"".to_string(),
                                          (top_row, bottom_row),
                                          tokenizer.clone(), tokens_limit, LINES_OVERLAP, false);
                chunks_.extend(chunks__);
                unused_symbols_cluster_accumulator_.clear();
            }
        };


        for guid in &guids {
            let symbol = guid_to_info.get(&guid).unwrap();
            let need_in_vecdb_at_all = match symbol.symbol_type {
                SymbolType::StructDeclaration | SymbolType::FunctionDeclaration |
                SymbolType::TypeAlias | SymbolType::ClassFieldDeclaration => true,
                _ => false,
            };
            if !need_in_vecdb_at_all {
                let mut is_flushed = false;
                let mut parent_guid = &symbol.parent_guid;
                while let Some(_parent_sym) = guid_to_info.get(parent_guid) {
                    if vec![SymbolType::StructDeclaration, SymbolType::FunctionDeclaration].contains(&_parent_sym.symbol_type) {
                        flush_accumulator(&mut unused_symbols_cluster_accumulator, &mut chunks);
                        is_flushed = true;
                        break;
                    }
                    parent_guid = &_parent_sym.parent_guid;
                }
                if !is_flushed {
                    unused_symbols_cluster_accumulator.push(symbol);
                }
                continue;
            }
            flush_accumulator(&mut unused_symbols_cluster_accumulator, &mut chunks);

            let formatter = make_formatter(&language);
            if symbol.symbol_type == SymbolType::StructDeclaration {
                if let Some(children) = guid_to_children.get(&symbol.guid) {
                    if !children.is_empty() {
                        let skeleton_line = formatter.make_skeleton(&symbol, &doc_text, &guid_to_children, &guid_to_info);
                        let chunks_ = get_chunks(&skeleton_line, &symbol.file_path,
                                                 &symbol.symbol_path,
                                                 (symbol.full_range.start_point.row, symbol.full_range.end_point.row),
                                                 tokenizer.clone(), tokens_limit, LINES_OVERLAP, true);
                        chunks.extend(chunks_);
                    }
                }
            }

            let (declaration, top_bottom_rows) = formatter.get_declaration_with_comments(&symbol, &doc_text, &guid_to_children, &guid_to_info);
            if !declaration.is_empty() {
                let chunks_ = get_chunks(&declaration, &symbol.file_path,
                                         &symbol.symbol_path, top_bottom_rows, tokenizer.clone(), tokens_limit, LINES_OVERLAP, true);
                chunks.extend(chunks_);
            }
        }

        flush_accumulator(&mut unused_symbols_cluster_accumulator, &mut chunks);

        Ok(chunks)
    }
}
