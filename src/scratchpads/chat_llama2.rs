use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use async_trait::async_trait;
use serde_json::Value;
use tokenizers::Tokenizer;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;
use tracing::{info, error};

use crate::at_commands::execute_at::run_at_commands;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatPost, ContextFile, SamplingParameters};
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::{FinishReason, HasTokenizerAndEot, ScratchpadAbstract};
use crate::scratchpads::chat_utils_deltadelta::DeltaDeltaChatStreamer;
use crate::scratchpads::chat_utils_limit_history::limit_messages_history;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::scratchpads::chat_utils_prompts::{get_default_system_prompt, system_prompt_add_workspace_info};


const DEBUG: bool = true;


// #[derive(Debug)]
pub struct ChatLlama2 {
    pub t: HasTokenizerAndEot,
    pub dd: DeltaDeltaChatStreamer,
    #[allow(dead_code)]
    pub post: ChatPost,
    pub messages: Vec<ChatMessage>,
    pub keyword_s: String, // "SYSTEM:" keyword means it's not one token
    pub keyword_slash_s: String,
    pub default_system_message: String,
    pub has_rag_results: HasRagResults,
    pub global_context: Arc<ARwLock<GlobalContext>>,
    pub allow_at: bool,
}


impl ChatLlama2 {
    pub fn new(
        tokenizer: Arc<StdRwLock<Tokenizer>>,
        post: &ChatPost,
        messages: &Vec<ChatMessage>,
        global_context: Arc<ARwLock<GlobalContext>>,
        allow_at: bool,
    ) -> Self {
        ChatLlama2 {
            t: HasTokenizerAndEot::new(tokenizer),
            dd: DeltaDeltaChatStreamer::new(),
            post: post.clone(),
            messages: messages.clone(),
            keyword_s: "<s>".to_string(),
            keyword_slash_s: "</s>".to_string(),
            default_system_message: "".to_string(),
            has_rag_results: HasRagResults::new(),
            global_context,
            allow_at,
        }
    }
}

#[async_trait]
impl ScratchpadAbstract for ChatLlama2 {
    async fn apply_model_adaptation_patch(
        &mut self,
        patch: &Value,
        exploration_tools: bool,
        agentic_tools: bool,
        _should_execute_remotely: bool,
    ) -> Result<(), String> {
        self.keyword_s = patch.get("s").and_then(|x| x.as_str()).unwrap_or("<s>").to_string();
        self.keyword_slash_s = patch.get("slash_s").and_then(|x| x.as_str()).unwrap_or("</s>").to_string();
        self.default_system_message = get_default_system_prompt(self.global_context.clone(), exploration_tools, agentic_tools).await;
        self.t.eot = self.keyword_s.clone();
        info!("llama2 chat model adaptation patch applied {:?}", self.keyword_s);
        self.t.assert_one_token(&self.t.eot.as_str())?;
        self.dd.stop_list.clear();
        self.dd.stop_list.push(self.t.eot.clone());
        self.dd.stop_list.push(self.keyword_slash_s.clone());
        Ok(())
    }

    async fn prompt(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        let (n_ctx, gcx) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.n_ctx, ccx_locked.global_context.clone())
        };
        let (messages, undroppable_msg_n, _any_context_produced) = if self.allow_at {
            run_at_commands(ccx.clone(), self.t.tokenizer.clone(), sampling_parameters_to_patch.max_new_tokens, &self.messages, &mut self.has_rag_results).await
        } else {
            (self.messages.clone(), self.messages.len(), false)
        };
        let mut limited_msgs: Vec<ChatMessage> = limit_messages_history(&self.t, &messages, undroppable_msg_n, sampling_parameters_to_patch.max_new_tokens, n_ctx, &self.default_system_message)?;
        if let Some(first_msg) = limited_msgs.first_mut() {
            if first_msg.role == "system" {
                first_msg.content = ChatContent::SimpleText(system_prompt_add_workspace_info(gcx.clone(), &first_msg.content.content_text_only()).await);
            }
        }
        sampling_parameters_to_patch.stop = self.dd.stop_list.clone();
        // loosely adapted from https://huggingface.co/spaces/huggingface-projects/llama-2-13b-chat/blob/main/model.py#L24
        let mut prompt = "".to_string();
        prompt.push_str(self.keyword_s.as_str());
        prompt.push_str("[INST] ");
        let mut do_strip = false;
        for msg in limited_msgs {
            let msg_content = msg.content.content_text_only();
            if msg.role == "system" {
                if !do_strip {
                    prompt.push_str("<<SYS>>\n");
                    prompt.push_str(self.default_system_message.as_str());
                    prompt.push_str("\n<</SYS>>\n");
                }
            } else {
                // prompt.push_str("\n\n");
            }
            if msg.role == "context_file" {
                let vector_of_context_files: Vec<ContextFile> = serde_json::from_str(&msg_content)
                    .map_err(|e|error!("parsing context_files has failed: {}; content: {}", e, &msg.content.content_text_only())).unwrap_or_default();
                for context_file in vector_of_context_files {
                    prompt.push_str(format!("{}\n```\n{}```\n\n", context_file.file_name, context_file.file_content).as_str());
                }
            }
            if msg.role == "cd_instruction" {
                prompt.push_str(msg_content.trim());
                prompt.push_str("");
            }
            if msg.role == "user" {
                let user_input = if do_strip { msg_content.trim().to_string() } else { msg_content.clone() };
                prompt.push_str(user_input.as_str());
                prompt.push_str(" [/INST]");
                do_strip = true;
            }

            if msg.role == "assistant" {
                prompt.push_str(msg_content.trim());
                prompt.push_str(" ");
                prompt.push_str(&self.keyword_slash_s.as_str());
                prompt.push_str(&self.keyword_s.as_str());
                prompt.push_str("[INST]");
            }
        }
        // This only supports assistant, not suggestions for user
        self.dd.role = "assistant".to_string();
        if DEBUG {
            // info!("llama2 chat vdb_suggestion {:?}", vdb_suggestion);
            info!("llama2 chat prompt\n{}", prompt);
            info!("llama2 chat re-encode whole prompt again gives {} tokens", self.t.count_tokens(prompt.as_str())?);
        }
        Ok(prompt)
    }

    fn response_n_choices(
        &mut self,
        choices: Vec<String>,
        finish_reasons: Vec<FinishReason>,
    ) -> Result<Value, String> {
        self.dd.response_n_choices(choices, finish_reasons)
    }

    fn response_streaming(
        &mut self,
        delta: String,
        finish_reason: FinishReason
    ) -> Result<(Value, FinishReason), String> {
        self.dd.response_streaming(delta, finish_reason)
    }
    fn response_message_n_choices(
        &mut self,
        _choices: Vec<String>,
        _finish_reasons: Vec<FinishReason>
    ) -> Result<Value, String> {
        Err("not implemented".to_string())
    }

    fn response_message_streaming(
        &mut self,
        _delta: &Value,
        _finish_reason: FinishReason
    ) -> Result<(Value, FinishReason), String> {
        Err("not implemented".to_string())
    }

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String>  {
        self.has_rag_results.response_streaming()
    }

    fn streaming_finished(&mut self, finish_reason: FinishReason) -> Result<Value, String> {
        self.dd.streaming_finished(finish_reason)
    }
}

