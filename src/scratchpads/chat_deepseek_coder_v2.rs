use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use async_trait::async_trait;
use serde_json::Value;
use tokenizers::Tokenizer;
use tokio::sync::RwLock as ARwLock;
use tracing::{info, error};
use crate::at_commands::execute_at::run_at_commands;

use crate::call_validation::{ChatMessage, ChatPost, ContextFile, SamplingParameters};
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::scratchpads::chat_generic::default_system_message_from_patch;
use crate::scratchpads::chat_utils_deltadelta::DeltaDeltaChatStreamer;
use crate::scratchpads::chat_utils_limit_history::limit_messages_history;
use crate::scratchpads::chat_utils_rag::HasRagResults;

const DEBUG: bool = true;


pub struct ChatDeepSeekCoderV2 {
    pub t: HasTokenizerAndEot,
    pub dd: DeltaDeltaChatStreamer,
    pub post: ChatPost,
    pub keyword_syst: String,
    pub keyword_user: String,
    pub keyword_asst: String,
    pub token_eos: String,
    pub token_eot: String,
    pub default_system_message: String,
    pub has_rag_results: HasRagResults,
    pub global_context: Arc<ARwLock<GlobalContext>>,
    pub allow_at: bool,
}


impl ChatDeepSeekCoderV2 {
    pub fn new(
        tokenizer: Arc<StdRwLock<Tokenizer>>,
        post: ChatPost,
        global_context: Arc<ARwLock<GlobalContext>>,
        allow_at: bool,
    ) -> Self {
        ChatDeepSeekCoderV2 {
            t: HasTokenizerAndEot::new(tokenizer),
            dd: DeltaDeltaChatStreamer::new(),
            post,
            keyword_syst: "".to_string(),
            keyword_user: "User: ".to_string(),
            keyword_asst: "Assistant: ".to_string(),
            token_eos: "<｜end▁of▁sentence｜>".to_string(),
            token_eot: "<|EOT|>".to_string(),
            default_system_message: "".to_string(),
            has_rag_results: HasRagResults::new(),
            global_context,
            allow_at,
        }
    }
}

#[async_trait]
impl ScratchpadAbstract for ChatDeepSeekCoderV2 {
    async fn apply_model_adaptation_patch(
        &mut self,
        patch: &Value,
    ) -> Result<(), String> {
        self.default_system_message = default_system_message_from_patch(&patch, self.global_context.clone()).await;
        self.t.assert_one_token(&self.token_eot.as_str())?;
        self.t.assert_one_token(&self.token_eos.as_str())?;
        self.t.eot = self.token_eot.clone();
        self.dd.stop_list.clear();
        self.dd.stop_list.push(self.token_eot.clone());
        self.dd.stop_list.push(self.token_eos.clone());
        Ok(())
    }

    async fn prompt(
        &mut self,
        context_size: usize,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        let top_n: usize = 7;
        let (messages, undroppable_msg_n, _any_context_produced) = if self.allow_at {
            run_at_commands(self.global_context.clone(), self.t.tokenizer.clone(), sampling_parameters_to_patch.max_new_tokens, context_size, &self.post.messages, top_n, &mut self.has_rag_results).await
        } else {
            (self.post.messages.clone(), self.post.messages.len(), false)
        };
        let limited_msgs: Vec<ChatMessage> = limit_messages_history(&self.t, &messages, undroppable_msg_n, sampling_parameters_to_patch.max_new_tokens, context_size, &self.default_system_message)?;
        sampling_parameters_to_patch.stop = self.dd.stop_list.clone();

        let mut prompt = "".to_string();
        let mut last_role = "assistant".to_string();
        for msg in limited_msgs {
            if msg.role == "system" {
                prompt.push_str(self.keyword_syst.as_str());
                prompt.push_str(msg.content.as_str());
                prompt.push_str("\n\n");
            } else if msg.role == "user" {
                prompt.push_str(self.keyword_user.as_str());
                prompt.push_str(msg.content.as_str());
                prompt.push_str("\n\n");
            } else if msg.role == "assistant" {
                prompt.push_str(self.keyword_asst.as_str());
                prompt.push_str(msg.content.as_str());
                prompt.push_str(self.token_eos.as_str());
            } else if msg.role == "context_file" {
                let vector_of_context_files: Vec<ContextFile> = serde_json::from_str(&msg.content).map_err(|e|error!("parsing context_files has failed: {}; content: {}", e, &msg.content)).unwrap_or(vec![]);
                for context_file in vector_of_context_files {
                    prompt.push_str(format!("{}\n```\n{}```\n\n", context_file.file_name, context_file.file_content).as_str());
                }
            } else {
                return Err(format!("role \"{}\"not recognized", msg.role));
            }
            last_role = msg.role.clone();
        }

        if last_role == "assistant" || last_role == "system" {
            self.dd.role = "user".to_string();
            prompt.push_str(self.keyword_user.as_str());
        } else if last_role == "user" || last_role == "context_file" {
            self.dd.role = "assistant".to_string();
            prompt.push_str(self.keyword_asst.as_str());
        }
        if DEBUG {
            info!("chat prompt\n{}", prompt);
            info!("chat re-encode whole prompt again gives {} tokens", self.t.count_tokens(prompt.as_str())?);
        }
        Ok(prompt)
    }

    fn response_n_choices(
        &mut self,
        choices: Vec<String>,
        stopped: Vec<bool>,
    ) -> Result<Value, String> {
        self.dd.response_n_choices(choices, stopped)
    }

    fn response_streaming(
        &mut self,
        delta: String,
        stop_toks: bool,
        _stop_length: bool,
    ) -> Result<(Value, bool), String> {
        self.dd.response_streaming(delta, stop_toks)
    }

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String> {
        return self.has_rag_results.response_streaming();
    }
}

