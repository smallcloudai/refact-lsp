use std::sync::Arc;
use std::sync::RwLock;

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
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::scratchpads::chat_utils_deltadelta::DeltaDeltaChatStreamer;
use crate::scratchpads::chat_utils_limit_history::limit_messages_history;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::scratchpads::chat_utils_prompts::{get_default_system_prompt, system_prompt_add_workspace_info};


const DEBUG: bool = true;


pub struct GenericChatScratchpad {
    pub t: HasTokenizerAndEot,
    pub dd: DeltaDeltaChatStreamer,
    pub post: ChatPost,
    pub token_esc: String,
    // for models that switch between sections using <esc>SECTION
    pub keyword_syst: String,
    // "SYSTEM:" keyword means it's not one token
    pub keyword_user: String,
    pub keyword_asst: String,
    pub default_system_message: String,
    pub has_rag_results: HasRagResults,
    pub global_context: Arc<ARwLock<GlobalContext>>,
    pub allow_at: bool,
}

impl GenericChatScratchpad {
    pub fn new(
        tokenizer: Arc<RwLock<Tokenizer>>,
        post: &ChatPost,
        global_context: Arc<ARwLock<GlobalContext>>,
        allow_at: bool,
    ) -> Self {
        GenericChatScratchpad {
            t: HasTokenizerAndEot::new(tokenizer),
            dd: DeltaDeltaChatStreamer::new(),
            post: post.clone(),
            token_esc: "".to_string(),
            keyword_syst: "".to_string(),
            keyword_user: "".to_string(),
            keyword_asst: "".to_string(),
            default_system_message: "".to_string(),
            has_rag_results: HasRagResults::new(),
            global_context,
            allow_at,
        }
    }
}

#[async_trait]
impl ScratchpadAbstract for GenericChatScratchpad {
    async fn apply_model_adaptation_patch(
        &mut self,
        patch: &Value,
        exploration_tools: bool,
        agentic_tools: bool,
    ) -> Result<(), String> {
        self.token_esc = patch.get("token_esc").and_then(|x| x.as_str()).unwrap_or("").to_string();
        self.keyword_syst = patch.get("keyword_system").and_then(|x| x.as_str()).unwrap_or("SYSTEM:").to_string();
        self.keyword_user = patch.get("keyword_user").and_then(|x| x.as_str()).unwrap_or("USER:").to_string();
        self.keyword_asst = patch.get("keyword_assistant").and_then(|x| x.as_str()).unwrap_or("ASSISTANT:").to_string();
        self.default_system_message = get_default_system_prompt(self.global_context.clone(), exploration_tools, agentic_tools).await;

        self.t.eot = patch.get("eot").and_then(|x| x.as_str()).unwrap_or("<|endoftext|>").to_string();

        self.dd.stop_list.clear();
        if !self.t.eot.is_empty() {
            self.t.assert_one_token(&self.t.eot.as_str())?;
            self.dd.stop_list.push(self.t.eot.clone());
        }
        if self.token_esc.len() > 0 {
            self.dd.stop_list.push(self.token_esc.clone());
        } else {
            self.dd.stop_list.push(self.keyword_syst.clone());
            self.dd.stop_list.push(self.keyword_user.clone());
            self.dd.stop_list.push(self.keyword_asst.clone());
        }
        self.dd.stop_list.retain(|x| !x.is_empty());

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
            run_at_commands(ccx.clone(), self.t.tokenizer.clone(), sampling_parameters_to_patch.max_new_tokens, &self.post.messages, &mut self.has_rag_results).await
        } else {
            (self.post.messages.clone(), self.post.messages.len(), false)
        };
        let mut limited_msgs: Vec<ChatMessage> = limit_messages_history(&self.t, &messages, undroppable_msg_n, self.post.parameters.max_new_tokens, n_ctx, &self.default_system_message)?;
        // if self.supports_tools {
        // };
        if let Some(first_msg) = limited_msgs.first_mut() {
            if first_msg.role == "system" {
                first_msg.content = ChatContent::SimpleText(
                    system_prompt_add_workspace_info(gcx.clone(), &first_msg.content.content_text_only()).await
                );
            }
        }
        sampling_parameters_to_patch.stop = self.dd.stop_list.clone();
        // adapted from https://huggingface.co/spaces/huggingface-projects/llama-2-13b-chat/blob/main/model.py#L24
        let mut prompt = "".to_string();
        let mut last_role = "assistant".to_string();
        for msg in limited_msgs {
            let content_text_only = msg.content.content_text_only();
            prompt.push_str(self.token_esc.as_str());
            if msg.role == "system" {
                prompt.push_str(self.keyword_syst.as_str());
                prompt.push_str(content_text_only.as_str());
                prompt.push_str("\n");
            } else if msg.role == "user" {
                prompt.push_str(self.keyword_user.as_str());
                prompt.push_str(content_text_only.as_str());
                prompt.push_str("\n");
            } else if msg.role == "cd_instruction" {
                prompt.push_str(self.keyword_user.as_str());
                prompt.push_str(content_text_only.as_str());
                prompt.push_str("\n");
            } else if msg.role == "assistant" {
                prompt.push_str(self.keyword_asst.as_str());
                prompt.push_str(content_text_only.as_str());
                prompt.push_str("\n");
            } else if msg.role == "context_file" {
                let vector_of_context_files: Vec<ContextFile> = serde_json::from_str(&content_text_only).map_err(|e|error!("parsing context_files has failed: {}; content: {}", e, &msg.content.content_text_only())).unwrap_or(vec![]);
                for context_file in vector_of_context_files {
                    prompt.push_str(format!("{}\n```\n{}```\n\n", context_file.file_name, context_file.file_content).as_str());
                }
            } else {
                return Err(format!("role \"{}\"not recognized", msg.role));
            }
            last_role = msg.role.clone();
        }
        prompt.push_str(self.token_esc.as_str());
        if last_role == "assistant" || last_role == "system" {
            self.dd.role = "user".to_string();
            prompt.push_str(self.keyword_user.as_str());
        } else if last_role == "user" {
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
