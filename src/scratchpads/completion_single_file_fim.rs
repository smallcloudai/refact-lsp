use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::time::Instant;
use std::vec;

use async_trait::async_trait;
use log::warn;
use ropey::Rope;
use serde_json::{Value, json};
use tokenizers::Tokenizer;
use tokio::sync::RwLock as ARwLock;
use tracing::{info, error};
use tree_sitter::Point;

use crate::ast::ast_module::AstModule;
use crate::ast::comments_wrapper::{get_language_id_by_filename, wrap_comments};
use crate::ast::treesitter::language_id::LanguageId;
use crate::at_commands::at_ast_lookup_symbols::results2message;
use crate::call_validation::{CodeCompletionPost, ChatMessage, ContextFile, SamplingParameters};
use crate::global_context::GlobalContext;
use crate::completion_cache;
use crate::files_in_workspace::Document;
use crate::nicer_logs::last_n_chars;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::scratchpads::chat_utils_rag::context_to_fim_debug_page;
use crate::telemetry::snippets_collection;
use crate::telemetry::telemetry_structs;


const DEBUG: bool = true;


pub struct SingleFileFIM {
    pub t: HasTokenizerAndEot,
    pub post: CodeCompletionPost,
    pub order: String,
    pub fim_prefix: String,
    pub fim_suffix: String,
    pub fim_middle: String,
    pub context_used: Value,
    pub data4cache: completion_cache::CompletionSaveToCache,
    pub data4snippet: snippets_collection::SaveSnippet,
    pub ast_module: Option<Arc<ARwLock<AstModule>>>,
    pub global_context: Arc<ARwLock<GlobalContext>>,
}

impl SingleFileFIM {
    pub fn new(
        tokenizer: Arc<StdRwLock<Tokenizer>>,
        post: CodeCompletionPost,
        order: String,
        cache_arc: Arc<StdRwLock<completion_cache::CompletionCache>>,
        tele_storage: Arc<StdRwLock<telemetry_structs::Storage>>,
        ast_module: Option<Arc<ARwLock<AstModule>>>,
        global_context: Arc<ARwLock<GlobalContext>>,
    ) -> Self {
        let data4cache = completion_cache::CompletionSaveToCache::new(cache_arc, &post);
        let data4snippet = snippets_collection::SaveSnippet::new(tele_storage, &post);
        SingleFileFIM { t: HasTokenizerAndEot::new(tokenizer), post, order, fim_prefix: String::new(),
            fim_suffix: String::new(), fim_middle: String::new(),
            context_used: json!({}),
            data4cache,
            data4snippet,
            ast_module,
            global_context,
        }
    }

    fn cleanup_prompt(&mut self, text: &String) -> String {
        text.replace(&self.fim_prefix, "")
            .replace(&self.fim_middle, "")
            .replace(&self.fim_suffix, "")
            .replace(&self.t.eos, "")
            .replace(&self.t.eot, "")
    }
}

fn add_context_to_prompt(context_format: &String, prompt: &String, postprocessed_messages: &Vec<ContextFile>, language_id: &LanguageId) -> String {
    let mut context_files = vec![];
    if context_format == "starcoder" {
        for m in postprocessed_messages {
            let s = format!(
                "{}{}{}{}",
                "<file_sep>",
                m.file_name,
                "\n",
                m.file_content
            );
            context_files.push(s);
        }
        if !context_files.is_empty() {
            context_files.insert(0, "<repo_name>default_repo".to_string());
            context_files.push("<file_sep>".to_string())
        }
    } else if context_format == "default" {
        for m in postprocessed_messages {
            context_files.push(wrap_comments(&format!(
                "{}\n{}",
                m.file_name,
                m.file_content
            ), language_id));
        }
    } else {
        warn!("context_format \"{}\" not recognized", context_format);
    }
    format!(
        "{}{}",
        context_files.join(""),
        prompt,
    )
}

#[async_trait]
impl ScratchpadAbstract for SingleFileFIM {
    fn apply_model_adaptation_patch(
        &mut self,
        patch: &Value,
    ) -> Result<(), String> {
        // That will work for some models (starcoder) without patching
        self.fim_prefix = patch.get("fim_prefix").and_then(|x| x.as_str()).unwrap_or("<fim_prefix>").to_string();
        self.fim_suffix = patch.get("fim_suffix").and_then(|x| x.as_str()).unwrap_or("<fim_suffix>").to_string();
        self.fim_middle = patch.get("fim_middle").and_then(|x| x.as_str()).unwrap_or("<fim_middle>").to_string();
        self.t.eot = patch.get("eot").and_then(|x| x.as_str()).unwrap_or("<|endoftext|>").to_string();
        self.t.eos = patch.get("eos").and_then(|x| x.as_str()).unwrap_or("").to_string();
        self.t.context_format = patch.get("context_format").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        self.t.rag_ratio = patch.get("rag_ratio").and_then(|x| x.as_f64()).unwrap_or(0.0);
        self.t.assert_one_token(&self.fim_prefix.as_str())?;
        self.t.assert_one_token(&self.fim_suffix.as_str())?;
        self.t.assert_one_token(&self.fim_middle.as_str())?;
        self.t.assert_one_token(&self.t.eot.as_str())?;
        if !self.t.eos.is_empty() {
            self.t.assert_one_token(&self.t.eos.as_str())?;
        }
        Ok(())
    }

    async fn prompt(
        &mut self,
        context_size: usize,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        let rag_tokens_n = if self.post.rag_tokens_n > 0 {
            self.post.rag_tokens_n.min(4096).max(1024)
        } else {
            ((context_size as f64 * self.t.rag_ratio) as usize).min(4096).max(1024)
        };
        let limit: i32 = (context_size as i32) - (self.post.parameters.max_new_tokens as i32) - (rag_tokens_n as i32);
        if limit < 512 {
            let msg = format!("context_size={} - max_new_tokens={} - rag_tokens_n={} leaves too little {} space for completion to work",
                context_size, self.post.parameters.max_new_tokens, rag_tokens_n, limit);
            warn!("{}", msg);
            return Err(msg);
        }

        let supports_stop = true; // some hf models do not support stop, but it's a thing of the past?
        if supports_stop {
            let mut stop_list = vec![self.t.eot.clone(), "\n\n".to_string()];
            if !self.post.inputs.multiline {
                stop_list.push("\n".to_string());  // This doesn't stop hf inference, only whole tokens do
            }
            sampling_parameters_to_patch.stop = Some(stop_list);
        }
        let mut source = self.post.inputs.sources.get(
                &self.post.inputs.cursor.file
            ).ok_or("Cursor is in file not found in sources".to_string())?.clone();
        source = self.cleanup_prompt(&source);

        let text = Rope::from_str(&*source);

        let pos = &self.post.inputs.cursor;
        let file_path = PathBuf::from(self.post.inputs.cursor.file.clone());
        let mut before_iter = text.lines_at(pos.line as usize).reversed();
        let mut after_iter = text.lines_at(pos.line as usize + 1);
        let mut tokens_used = 0;

        let mut before_line = before_iter.next();

        let cursor_line1: String;
        let col = pos.character as usize;
        cursor_line1 = text.line(pos.line as usize).slice(0..col).to_string();
        // UNFINISHED LI|

        let mut after_line = after_iter.next();

        let cursor_line2: String;
        if self.post.inputs.multiline {
            cursor_line2 = text.line(pos.line as usize).slice(col..).to_string();
        } else {
            cursor_line2 = "".to_string();
        }

        let mut before = vec![];
        let mut after = String::new();
        let mut fim_line1: i32 = i32::MAX;
        let mut fim_line2: i32 = i32::MIN;
        tokens_used += self.t.count_tokens(
            (cursor_line1.clone() + &cursor_line2).as_str()
        )?;
        let mut rel_line_n: i32 = 0;
        while before_line.is_some() || after_line.is_some() {
            if let Some(before_line) = before_line {
                let before_line = before_line.to_string();
                let tokens = self.t.count_tokens(before_line.as_str())?;
                if tokens_used + tokens > limit {
                    break;
                }
                tokens_used += tokens;
                before.push(before_line);
                fim_line1 = pos.line - rel_line_n as i32;
            }
            if let Some(after_line) = after_line {
                let after_line = after_line.to_string();
                let tokens = self.t.count_tokens(after_line.as_str())?;
                if tokens_used + tokens > limit {
                    break;
                }
                tokens_used += tokens;
                after.push_str(&after_line);
                fim_line2 = pos.line + rel_line_n as i32;
            }
            before_line = before_iter.next();
            after_line = after_iter.next();
            rel_line_n += 1;
        }

        let before = before.into_iter().rev().collect::<Vec<_>>().join("");
        info!("single file FIM prompt {} tokens used < limit {}", tokens_used, limit);
        let mut prompt: String;
        if self.order == "PSM" {
            prompt = format!(
                "{}{}{}{}{}{}{}{}",
                self.t.eos,
                self.fim_prefix,
                before,
                cursor_line1,
                self.fim_suffix,
                cursor_line2,
                after,
                self.fim_middle
            );
        } else if self.order == "SPM" {
            prompt = format!(
                "{}{}{}{}{}{}{}{}",
                self.t.eos,
                self.fim_suffix,
                cursor_line2,
                after,
                self.fim_prefix,
                before,
                cursor_line1,
                self.fim_middle,
            );
        } else {
            return Err(format!("order \"{}\" not recognized", self.order));
        }

        if !self.t.context_format.is_empty() && self.post.use_ast && rag_tokens_n > 0 && self.ast_module.is_some() {
            let t0 = Instant::now();
            let language_id = get_language_id_by_filename(&PathBuf::from(&self.post.inputs.cursor.file)).unwrap_or(LanguageId::Unknown);
            let (mut ast_messages, was_looking_for) = {
                let doc = Document::new(&file_path, None);
                match self.ast_module.clone().unwrap().write().await.retrieve_cursor_symbols_by_declarations(
                    &doc, &source, Point { row: pos.line as usize, column: pos.character as usize },
                    5, 5
                ).await {
                    Ok(res) => {
                        let mut was_looking_for = HashMap::new();
                        let cursor_symbols = res.cursor_symbols.iter().map(|x| last_n_chars(&x.symbol_declaration.name, 30)).collect::<Vec<_>>();
                        let declarations = res.declaration_symbols.iter().map(|x| last_n_chars(&x.symbol_declaration.name, 30)).collect::<Vec<_>>();
                        let usages = res.declaration_usage_symbols.iter().map(|x| last_n_chars(&x.symbol_declaration.name, 30)).collect::<Vec<_>>();
                        let matched_by_name_symbols = res.matched_by_name_symbols.iter().map(|x| last_n_chars(&x.symbol_declaration.name, 30)).collect::<Vec<_>>();
                        info!("near cursor cursor_symbols: {:?}", cursor_symbols);
                        info!("near cursor declarations: {:?}", declarations);
                        info!("near cursor usages: {:?}", usages);
                        info!("near cursor matched_by_name_symbols: {:?}", matched_by_name_symbols);
                        was_looking_for.insert("cursor_symbols".to_string(), cursor_symbols);
                        was_looking_for.insert("declarations".to_string(), declarations);
                        was_looking_for.insert("usages".to_string(), usages);
                        was_looking_for.insert("matched_by_name_symbols".to_string(), matched_by_name_symbols);
                        (vec![results2message(&res).await], was_looking_for)
                    },
                    Err(err) => {
                        error!("can't fetch ast results: {}", err);
                        (vec![], HashMap::new())
                    }
                }
            };

            let fim_ban = ContextFile {
                file_name: self.post.inputs.cursor.file.clone(),
                file_content: "".to_string(),
                line1: (fim_line1 + 1) as usize,
                line2: (fim_line2 + 1) as usize,
                symbol: "".to_string(),
                usefulness: -1.0,
            };
            ast_messages.push(ChatMessage { role: "context_file".to_string(), content: serde_json::json!([fim_ban]).to_string() });

            let postprocessed_messages = crate::scratchpads::chat_utils_rag::postprocess_at_results2(
                self.global_context.clone(),
                ast_messages,
                self.t.tokenizer.clone(),
                rag_tokens_n,
            ).await;

            prompt = add_context_to_prompt(&self.t.context_format, &prompt, &postprocessed_messages, &language_id);
            self.context_used = context_to_fim_debug_page(&t0, &postprocessed_messages, &was_looking_for);
        } else {
            info!("will not use ast {}{}{}{}", self.t.context_format.is_empty() as i32, self.post.use_ast as i32, (rag_tokens_n > 0) as i32, self.ast_module.is_some() as i32);
        }

        if DEBUG {
            info!("cursor position\n{:?}", self.post.inputs.cursor);
            info!("prompt\n{}", prompt);
            info!("re-encode whole prompt again gives {} tokens", self.t.count_tokens(prompt.as_str())?);
        }
        Ok(prompt)
    }

    fn response_n_choices(
        &mut self,
        choices: Vec<String>,
        stopped: Vec<bool>
    ) -> Result<Value, String> {
        let json_choices = choices.iter().enumerate().map(|(i, x)| {
            let (mut cc, mut finished) = cut_result(&x, self.t.eot.as_str(), self.post.inputs.multiline);
            finished |= stopped[i];
            let finish_reason = if finished {
                cc = cc.trim_end().to_string();
                "stop"
            } else {
                "length"
            }.to_string();
            if i==0 {
                self.data4cache.completion0_text = cc.clone();
                self.data4cache.completion0_finish_reason = finish_reason.clone();
            }
            json!({
                "index": i,
                "code_completion": cc,
                "finish_reason": finish_reason.clone(),
            })
        }).collect::<Vec<_>>();
        if DEBUG {
            info!("response_n_choices\n{:?}", json_choices);
        }

        snippets_collection::snippet_register_from_data4cache(&self.data4snippet, &mut self.data4cache);
        return Ok(json!(
            {
                "choices": json_choices,
                "snippet_telemetry_id": self.data4cache.completion0_snippet_telemetry_id,
                "model": self.post.model.clone(),
                "context": self.context_used,
            }
        ));
    }

    fn response_streaming(
        &mut self,
        delta: String,
        stop_toks: bool,
        stop_length: bool,
    ) -> Result<(Value, bool), String> {
        let mut finished;
        let json_choices;
        // info!("XXXXX delta: {:?}", delta);
        // info!("XXXXX stop_toks: {:?}", stop_toks);
        // info!("XXXXX stop_length: {:?}", stop_length);
        if !delta.is_empty() || stop_toks {
            let mut s: String;
            (s, finished) = cut_result(&delta, self.t.eot.as_str(), self.post.inputs.multiline);
            finished |= stop_toks;
            if finished {
                // can stay consistent with trim() only if that's the final iteration
                s = s.trim_end().to_string();
                self.data4cache.completion0_finish_reason = if finished { "stop".to_string() } else { "".to_string() };
            }
            self.data4cache.completion0_text.push_str(&s);
            json_choices = json!([{
                "index": 0,
                "code_completion": s,
                "finish_reason": if finished { serde_json::Value::String("stop".to_string()) } else { serde_json::Value::Null },
            }]);
        } else {
            assert!(stop_length);
            json_choices = json!([{
                "index": 0,
                "code_completion": "",
                "finish_reason": "length"
            }]);
            self.data4cache.completion0_finish_reason = "length".to_string();
            finished = true;
        }
        snippets_collection::snippet_register_from_data4cache(&self.data4snippet, &mut self.data4cache);
        let ans = json!({
            "choices": json_choices,
            "snippet_telemetry_id": self.data4cache.completion0_snippet_telemetry_id,
        });
        Ok((ans, finished))
    }

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String>  {
        return Err("".to_string());
    }
}


fn cut_result(text: &str, eot_token: &str, multiline: bool) -> (String, bool) {
    let mut cut_at = vec![];
    if let Some(x) = text.find(eot_token) {
        cut_at.push(x);
    }
    if let Some(x) = text.find("\n\n") {
        cut_at.push(x);
    }
    if let Some(x) = text.find("\r\n\r\n") {
        cut_at.push(x);
    }
    if !multiline {
        if let Some(x) = text.find("\n") {
            cut_at.push(x);
        }
    }
    if cut_at.is_empty() {
        return (text.to_string().replace("\r", ""), false);
    }
    let cut_at = cut_at.into_iter().min().unwrap_or(text.len());
    let ans = text.split_at(cut_at).0.to_string();
    return (ans.replace("\r", ""), true);
}

