use std::sync::Arc;
use std::sync::RwLock;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{info, warn};
use serde_json::{json, Value};
use tokenizers::Tokenizer;
use tokio::sync::RwLock as ARwLock;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::ast::treesitter::ast_instance_structs::SymbolInformation;

use crate::call_validation::{ChatMessage, ChatPost, ContextFile};
use crate::global_context::GlobalContext;
use crate::ast::structs::FileASTMarkup;
use crate::files_in_workspace::{canonical_path, correct_to_nearest_filename, Document, get_file_text_from_memory_or_disk};
use crate::nicer_logs::{first_n_chars, last_n_chars};


const RESERVE_FOR_QUESTION_AND_FOLLOWUP: usize = 1024;  // tokens
const DEBUG: bool = true;


#[derive(Debug)]
pub struct File {
    pub markup: FileASTMarkup,
    pub cpath: PathBuf,
}

#[derive(Debug)]
pub struct FileLine {
    pub fref: Arc<ARwLock<File>>,
    pub line_n: usize,
    pub line_content: String,
    pub useful: f32,
    pub color: String,
    pub take: bool,
}

pub fn context_to_fim_debug_page(t0: &Instant, postprocessed_messages: &[ContextFile], was_looking_for: &HashMap<String, Vec<String>>) -> Value {
    let attached_files: Vec<_> = postprocessed_messages.iter().map(|x| {
        json!({
            "file_name": x.file_name,
            "file_content": x.file_content,
            "line1": x.line1,
            "line2": x.line2,
        })
    }).collect();

    let was_looking_for_vec: Vec<_> = was_looking_for.iter().flat_map(|(k, v)| {
        v.iter().map(move |i| {
            json!({
                "from": k,
                "symbol": i,
            })
        })
    }).collect();
    let elapsed = t0.elapsed().as_secs_f32();
    json!({
        "elapsed": elapsed,
        "was_looking_for": was_looking_for_vec,
        "attached_files": attached_files,
    })
}

pub async fn omsgs_from_paths(
    global_context: Arc<ARwLock<GlobalContext>>,
    files_set: HashSet<String>
) -> Vec<ContextFile> {
    let mut omsgs = vec![];
    for file_name in files_set {
        let path = canonical_path(&file_name.clone());
        let text = get_file_text_from_memory_or_disk(global_context.clone(), &path).await.unwrap_or_default();
        omsgs.push(ContextFile {
            file_name: file_name.clone(),
            file_content: text.clone(),
            line1: 0,
            line2: text.lines().count(),
            symbol: "".to_string(),
            usefulness: 0.,
        });
    }
    omsgs
}

fn msg2doc(msg: &ContextFile) -> Document {
    let mut doc = Document::new(&PathBuf::from(&msg.file_name), None);
    doc.update_text(&msg.file_content);
    doc
}

async fn colorize_if_more_useful(linevec: &Vec<Arc<ARwLock<FileLine>>>, line1: usize, line2: usize, color: &String, useful: f32) {
    if DEBUG {
        info!("    colorize_if_more_useful {}..{} <= color {:?} useful {}", line1, line2, color, useful);
    }
    for i in line1 .. line2 {
        let u = useful - (i as f32) * 0.001;
        match linevec.get(i) {
            Some(line) => {
                let mut line_lock = line.write().await;
                if line_lock.useful < u || line_lock.color.is_empty() {
                    line_lock.useful = u;
                    line_lock.color = color.clone();
                }
            },
            None => warn!("    {} has faulty range {}..{}", color, line1, line2),
        }
    }
}

async fn colorize_minus_one(linevec: &Vec<Arc<ARwLock<FileLine>>>, line1: usize, line2: usize) {
    for i in line1 .. line2 {
        match linevec.get(i) {
            Some(line) => {
                let mut line_lock = line.write().await;
                line_lock.useful = -1.;
                line_lock.color = "disabled".to_string();
            },
            None => {}
        }
    }
}

async fn downgrade_lines_if_subsymbol(linevec: &Vec<Arc<ARwLock<FileLine>>>, line1_base0: usize, line2_base0: usize, subsymbol: &String, downgrade_coef: f32) {
    let mut changes_cnt = 0;
    for i in line1_base0 .. line2_base0 {
        assert!(i < linevec.len());
        match linevec.get(i) {
            Some(line) => {
                let mut line_lock = line.write().await;
                if subsymbol.starts_with(&line_lock.color) {
                    if i == line2_base0-1 || i == line1_base0 {
                        if line_lock.line_content.trim().len() == 1 {
                            // HACK: closing brackets at the end, leave it alone without downgrade
                            continue;
                        }
                    }
                }
                line_lock.useful *= downgrade_coef;
                line_lock.color = subsymbol.clone();
                changes_cnt += 1;
            },
            None => { continue; }
        }
    }
    if DEBUG {
        info!("        {}..{} ({} affected) <= subsymbol {:?} downgrade {}", changes_cnt, line1_base0, line2_base0, subsymbol, downgrade_coef);
    }
}

pub async fn postprocess_rag_stage1(
    global_context: Arc<ARwLock<GlobalContext>>,
    origmsgs: Vec<ContextFile>,
    close_small_gaps: bool,
    force_read_text: bool,
) -> (HashMap<PathBuf, Vec<Arc<ARwLock<FileLine>>>>, Vec<Arc<ARwLock<FileLine>>>){
    // 1. Load files, with ast or not
    let mut files = HashMap::new();
    let ast_module = global_context.read().await.ast_module.clone();
    
    for msg in origmsgs.iter() {
        let mut f: Option<File> = None;
        let mut doc = msg2doc(&msg);
        if force_read_text {
            match doc.get_text_or_read_from_disk().await {
                Ok(text) => doc.update_text(&text),
                Err(_) => {}
            }
        }

        if let Some(ast) = &ast_module {
            match ast.write().await.file_markup(&doc).await {
                Ok(markup) => {
                    if markup.file_content == doc.text.clone().unwrap_or_default().to_string() {
                        f = Some(File { markup, cpath: doc.path.clone() });
                    }
                },
                Err(err) => {
                    warn!("postprocess_rag_stage1 query file {:?} markup problem: {}", doc.path.display(), err);
                }
            }
        }
        if f.is_none() {
            f = Some(File {
                markup: FileASTMarkup {
                    file_path: doc.path.clone(),
                    file_content: doc.text.unwrap_or_default().to_string(),
                    symbols_sorted_by_path_len: Vec::new(),
                },
                cpath: doc.path.clone(),
            });
        }
        files.insert(msg.file_name.clone(), Arc::new(ARwLock::new(f.unwrap())));
    }
    
    // 2. Generate line refs, fill background scopes found in a file (not search results yet)
    let mut lines_by_useful = vec![];
    let mut lines_in_files = HashMap::new();
    for fref in files.values() {
        for (line_n, line) in fref.read().await.markup.file_content.lines().enumerate() {
            let file_line = FileLine {
                fref: fref.clone(),
                line_n,
                line_content: line.to_string(),
                useful: 0.0,
                color: "".to_string(),
                take: false,
            };
            let file_line_arc = Arc::new(ARwLock::new(file_line));
            lines_by_useful.push(file_line_arc.clone());
            lines_in_files.entry(fref.read().await.cpath.clone()).or_insert(vec![]).push(file_line_arc.clone());
        }
    }
    
    for linevec in lines_in_files.values() {
        if let Some(line) = linevec.get(0) {
            for s in line.read().await.fref.read().await.markup.symbols_sorted_by_path_len.iter() {
                let useful = 10.;  // depends on symbol type?
                colorize_if_more_useful(linevec, s.full_range.start_point.row, s.full_range.end_point.row+1, &format!("{}", s.symbol_path), useful).await;
            }
        }
        colorize_if_more_useful(linevec, 0, linevec.len(), &"".to_string(), 5.).await;
    }

    // 3. Fill in usefulness from search results
    for omsg in origmsgs.iter() {
        // Do what we can to match omsg.file_name to something real
        let nearest = correct_to_nearest_filename(global_context.clone(), &omsg.file_name, false, 1).await;
        let cpath = canonical_path(&nearest.get(0).unwrap_or(&omsg.file_name));
        
        let linevec= match lines_in_files.get(&cpath) {
            Some(x) => x,
            None => {
                warn!("postprocess_rag_stage1: file not found {:?} or transformed to canonical path {:?}", omsg.file_name, cpath);
                continue;
            }
        };
        let mut symbol_mb: Option<SymbolInformation> = None;
        match linevec.get(0) {
            Some(line) => {
                if !omsg.symbol.is_empty() {
                    for sym in line.read().await.fref.read().await.markup.symbols_sorted_by_path_len.iter() {
                        if sym.guid == omsg.symbol {
                            symbol_mb = Some(sym.clone());
                            break;
                        }
                    }
                    if symbol_mb.is_none() {
                        warn!("postprocess_rag_stage1: cannot find symbol {} in file {}", omsg.symbol, omsg.file_name);
                    }
                }
            }
            None => { continue; }
        }
        if omsg.usefulness < 0. {
            colorize_minus_one(linevec, omsg.line1-1, omsg.line2).await;
            continue;
        }
        
        match symbol_mb {
            Some(s) => {
                info!("    search result {} {:?} {:.2}", s.symbol_path, s.symbol_type, omsg.usefulness);
                colorize_if_more_useful(linevec, s.full_range.start_point.row, s.full_range.end_point.row+1, &format!("{}", s.symbol_path), omsg.usefulness).await;
            }
            None => {
                if omsg.line1 == 0 || omsg.line2 == 0 || omsg.line1 > omsg.line2 || omsg.line1 > linevec.len() || omsg.line2 > linevec.len() {
                    warn!("postprocess_rag_stage1: cannot use range {}:{}..{}", omsg.file_name, omsg.line1, omsg.line2);
                    continue;
                }
                colorize_if_more_useful(linevec, omsg.line1-1, omsg.line2, &"nosymb".to_string(), omsg.usefulness).await;
            }
        }
    }

    // 4. Downgrade sub-symbols and uninteresting regions
    for linevec in lines_in_files.values() {
        match linevec.get(0) {
            Some(line) => {
                let line_lock = line.read().await;
                let fref = line_lock.fref.read().await;
                if DEBUG {
                    info!("degrading body of symbols in {:?}", fref.cpath);
                }
                for sym in fref.markup.symbols_sorted_by_path_len.iter() {
                    if DEBUG {
                        info!("    {} {:?} {}-{}", sym.symbol_path, sym.symbol_type, sym.full_range.start_point.row, sym.full_range.end_point.row);
                    }
                    if sym.definition_range.end_byte != 0 {
                        // decl  void f() {
                        // def      int x = 5;
                        // def   }
                        let (def0, def1) = (
                            sym.definition_range.start_point.row.max(sym.declaration_range.end_point.row + 1),   // definition must stay clear of declaration
                            sym.definition_range.end_point.row + 1
                        );
                        if def1 > def0 {
                            downgrade_lines_if_subsymbol(linevec, def0, def1, &format!("{}::body", sym.symbol_path), 0.8).await;
                            // NOTE: this will not downgrade function body of a function that is a search result, because it's not a subsymbol it's the symbol itself (equal path)
                        }
                    }

                }

            },
            None => { continue; }
        }
    }

    // 5. A-la mathematical morphology, removes one-line holes
    if close_small_gaps {
        for linevec in lines_in_files.values() {
            let mut useful_copy = vec![];
            for line in linevec.iter() {
                useful_copy.push(line.read().await.useful);
            }
            for i in 1 .. linevec.len() - 1 {
                let (l, m, r) = (
                    linevec.get(i-1).unwrap().read().await.useful,
                    linevec.get(i).unwrap().read().await.useful,
                    linevec.get(i+1).unwrap().read().await.useful,
                );
                
                let both_l_and_r_support = l.min(r);
                useful_copy[i] = m.max(both_l_and_r_support);
            }
            for i in 0 .. linevec.len() {
                if let Some(line) = linevec.get(i) {
                    line.write().await.useful = useful_copy[i];
                }
            }
        }
    }

    (lines_in_files, lines_by_useful)
}

pub async fn _postprocess2_from_chat_messages(
    global_context: Arc<ARwLock<GlobalContext>>,
    messages: Vec<ChatMessage>,
    tokenizer: Arc<RwLock<Tokenizer>>,
    tokens_limit: usize,
    force_read_text: bool,
) -> Vec<ContextFile> {
    let mut origmsgs: Vec<ContextFile> = vec![];
    for msg in messages {
        match serde_json::from_str::<Vec<ContextFile>>(&msg.content) {
            Ok(decoded) => {
                origmsgs.extend(decoded.clone());
            },
            Err(err) => {
                warn!("postprocess_at_results2 decoding results problem: {}", err);
                continue;
            }
        }
    }
    postprocess_at_results2(global_context, origmsgs, tokenizer, tokens_limit, force_read_text).await
}

pub async fn postprocess_at_results2(
    global_context: Arc<ARwLock<GlobalContext>>,
    messages: Vec<ContextFile>,
    tokenizer: Arc<RwLock<Tokenizer>>,
    tokens_limit: usize,
    force_read_text: bool
) -> Vec<ContextFile> {
    // 1-5
    let (lines_in_files, lines_by_useful) = postprocess_rag_stage1(
        global_context, messages, true, force_read_text
    ).await;

    // 6. Sort
    let mut useful_values = vec![];
    for v in lines_by_useful.iter() {
        useful_values.push(v.read().await.useful);
    }
    let mut indices: Vec<_> = (0..lines_by_useful.len()).collect();
    indices.sort_by(|&a, &b| {
        let a_useful = useful_values[a];
        let b_useful = useful_values[b];
        b_useful.partial_cmp(&a_useful).unwrap_or(Ordering::Equal)
    });
    let lines_by_useful: Vec<_> = indices.into_iter().map(|i| lines_by_useful[i].clone()).collect();
    
    // 7. Convert line_content to tokens up to the limit
    let mut tokens_count: usize = 0;
    let mut lines_take_cnt: usize = 0;
    for lineref in lines_by_useful.iter() {
        let mut line_lock = lineref.write().await;
        if line_lock.useful < 0.0 {
            continue;
        }
        let n_tokens = count_tokens(&tokenizer.read().unwrap(), &line_lock.line_content);
        if tokens_count + n_tokens > tokens_limit {
            break;
        }
        tokens_count += n_tokens;
        line_lock.take = true;
        lines_take_cnt += 1;
    }
    info!("{} lines in {} files  =>  tokens {} < {} tokens limit  =>  {} lines", lines_by_useful.len(), lines_in_files.len(), tokens_count, tokens_limit, lines_take_cnt);
    if DEBUG {
        for linevec in lines_in_files.values() {
            for lineref in linevec.iter() {
                let line_lock = lineref.read().await;
                info!("{} {}:{:04} {:>7.3} {}",
                if line_lock.take { "take" } else { "dont" },
                last_n_chars(&line_lock.fref.read().await.cpath.to_string_lossy().to_string(), 30),
                line_lock.line_n,
                line_lock.useful,
                first_n_chars(&line_lock.line_content, 20)
            );
            }
        }
    }

    // 8. Generate output
    let mut merged: Vec<ContextFile> = vec![];
    for linevec in lines_in_files.values() {
        if let Some(line) = linevec.get(0) {
            let cpath = line.read().await.fref.read().await.cpath.clone();
            let mut out = String::new();
            let mut first_line: usize = 0;
            let mut last_line: usize = 0;
            let mut prev_line: usize = 0;
            let mut anything = false;
            
            for (i, line_i) in linevec.iter().enumerate() {
                let line_lock = line_i.read().await;
                last_line = i;
                if !line_lock.take {
                    continue;
                }
                anything = true;
                if first_line == 0 { 
                    first_line = i; 
                }
                if i > prev_line + 1 {
                    out.push_str(format!("...{} lines\n", i - prev_line - 1).as_str());
                }
                out.push_str(&line_lock.line_content);
                out.push_str("\n");
                prev_line = i;
            }
            if last_line > prev_line + 1 {
                out.push_str("...\n");
            }
            if DEBUG {
                info!("file {:?}\n{}", cpath, out);
            }
            if !anything {
                continue;
            }
            merged.push(ContextFile {
                file_name: cpath.to_string_lossy().to_string(),
                file_content: out,
                line1: first_line,
                line2: last_line,
                symbol: "".to_string(),
                usefulness: 0.0,
            });
        }
    }
    merged
}

pub fn count_tokens(
    tokenizer: &Tokenizer,
    text: &str,
) -> usize {
    match tokenizer.encode(text, false) {
        Ok(tokens) => tokens.len(),
        Err(_) => 0,
    }
}

pub async fn run_at_commands(
    global_context: Arc<ARwLock<GlobalContext>>,
    tokenizer: Arc<RwLock<Tokenizer>>,
    maxgen: usize,
    n_ctx: usize,
    post: &mut ChatPost,
    top_n: usize,
    stream_back_to_user: &mut HasVecdbResults,
) -> usize {
    // TODO: don't operate on `post`, return a copy of the messages
    let context = AtCommandsContext::new(global_context.clone()).await;

    let mut user_msg_starts = post.messages.len();
    let mut user_messages_with_at: usize = 0;
    while user_msg_starts > 0 {
        let message = post.messages.get(user_msg_starts - 1).unwrap().clone();
        let role = message.role.clone();
        let content = message.content.clone();
        info!("user_msg_starts {} {}", user_msg_starts - 1, role);
        if role == "user" {
            user_msg_starts -= 1;
            if content.contains("@") {
                user_messages_with_at += 1;
            }
        } else {
            break;
        }
    }
    user_messages_with_at = user_messages_with_at.max(1);
    let reserve_for_context = n_ctx - maxgen - RESERVE_FOR_QUESTION_AND_FOLLOWUP;
    info!("reserve_for_context {} tokens", reserve_for_context);

    // Token limit works like this:
    // - if there's only 1 user message at the bottom, it receives ntokens_minus_maxgen tokens for context
    // - if there are N user messages, they receive ntokens_minus_maxgen/N tokens each (and there's no taking from one to give to the other)
    // This is useful to give prefix and suffix of the same file precisely the position necessary for FIM-like operation of a chat model

    let mut rebuilt_messages: Vec<ChatMessage> = post.messages.iter().take(user_msg_starts).map(|m| m.clone()).collect();
    for msg_idx in user_msg_starts..post.messages.len() {
        let mut user_posted = post.messages[msg_idx].content.clone();
        let user_posted_ntokens = count_tokens(&tokenizer.read().unwrap(), &user_posted);
        let mut context_limit = reserve_for_context / user_messages_with_at;
        if context_limit <= user_posted_ntokens {
            context_limit = 0;
        } else {
            context_limit -= user_posted_ntokens;
        }
        info!("msg {} user_posted {:?} that's {} tokens", msg_idx, user_posted, user_posted_ntokens);
        info!("that leaves {} tokens for context of this message", context_limit);

        let valid_commands = crate::at_commands::utils::find_valid_at_commands_in_query(&mut user_posted, &context).await;
        let mut messages_for_postprocessing = vec![];
        for cmd in valid_commands {
            match cmd.command.lock().await.execute(&user_posted, &cmd.args, top_n, &context).await {
                Ok(msgs) => {
                    messages_for_postprocessing.extend(msgs);
                },
                Err(e) => {
                    warn!("can't execute command that indicated it can execute: {}", e);
                }
            }
        }
        let t0 = Instant::now();
        let processed = postprocess_at_results2(
            global_context.clone(),
            messages_for_postprocessing,
            tokenizer.clone(),
            context_limit,
            true,
        ).await;
        info!("postprocess_at_results2 {:.3}s", t0.elapsed().as_secs_f32());
        if processed.len() > 0 {
            let message = ChatMessage {
                role: "context_file".to_string(),
                content: serde_json::to_string(&processed).unwrap(),
            };
            rebuilt_messages.push(message.clone());
            stream_back_to_user.push_in_json(json!(message));
        }
        if user_posted.trim().len() > 0 {
            let msg = ChatMessage {
                role: "user".to_string(),
                content: user_posted,  // stream back to the user, without commands
            };
            rebuilt_messages.push(msg.clone());
            stream_back_to_user.push_in_json(json!(msg));
        }
    }
    post.messages = rebuilt_messages;
    user_msg_starts
}


pub struct HasVecdbResults {
    pub was_sent: bool,
    pub in_json: Vec<Value>,
}

impl HasVecdbResults {
    pub fn new() -> Self {
        HasVecdbResults {
            was_sent: false,
            in_json: vec![],
        }
    }
}

impl HasVecdbResults {
    pub fn push_in_json(&mut self, value: Value) {
        self.in_json.push(value);
    }

    pub fn response_streaming(&mut self) -> Result<Vec<Value>, String> {
        if self.was_sent == true || self.in_json.is_empty() {
            return Ok(vec![]);
        }
        self.was_sent = true;
        Ok(self.in_json.clone())
    }
}
