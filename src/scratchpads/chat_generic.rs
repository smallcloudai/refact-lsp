use std::sync::Arc;
use std::sync::RwLock;

use async_trait::async_trait;
use tokenizers::Tokenizer;
use tokio::sync::Mutex as AMutex;
use tracing::info;

use crate::call_validation::{ChatMessage, ChatPost, ContextFile, SamplingParameters};
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::scratchpads::chat_utils_deltadelta::DeltaDeltaChatStreamer;
use crate::scratchpads::chat_utils_limit_history::limit_messages_history;
use crate::scratchpads::chat_utils_rag::embed_vecdb_results;
use crate::vecdb::structs::VecdbSearch;

const DEBUG: bool = true;


pub struct GenericChatScratchpad<T> {
    pub t: HasTokenizerAndEot,
    pub dd: DeltaDeltaChatStreamer,
    pub post: ChatPost,
    pub token_esc: String,    // for models that switch between sections using <esc>SECTION
    pub keyword_syst: String, // "SYSTEM:" keyword means it's not one token
    pub keyword_user: String,
    pub keyword_asst: String,
    pub default_system_message: String,
    pub vecdb_search: Arc<AMutex<Option<T>>>,
}

impl<T: Send + Sync + VecdbSearch> GenericChatScratchpad<T> {
    pub fn new(
        tokenizer: Arc<RwLock<Tokenizer>>,
        post: ChatPost,
        vecdb_search: Arc<AMutex<Option<T>>>,
    ) -> Self where T: VecdbSearch + 'static + Sync {
        GenericChatScratchpad {
            t: HasTokenizerAndEot::new(tokenizer),
            dd: DeltaDeltaChatStreamer::new(),
            post,
            token_esc: "".to_string(),
            keyword_syst: "".to_string(),
            keyword_user: "".to_string(),
            keyword_asst: "".to_string(),
            default_system_message: "".to_string(),
            vecdb_search
        }
    }
}

#[async_trait]
impl<T: Send + Sync + VecdbSearch> ScratchpadAbstract for GenericChatScratchpad<T> {
    fn apply_model_adaptation_patch(
        &mut self,
        patch: &serde_json::Value,
    ) -> Result<(), String> {
        self.token_esc = patch.get("token_esc").and_then(|x| x.as_str()).unwrap_or("").to_string();
        self.keyword_syst = patch.get("keyword_system").and_then(|x| x.as_str()).unwrap_or("SYSTEM:").to_string();
        self.keyword_user = patch.get("keyword_user").and_then(|x| x.as_str()).unwrap_or("USER:").to_string();
        self.keyword_asst = patch.get("keyword_assistant").and_then(|x| x.as_str()).unwrap_or("ASSISTANT:").to_string();
        self.default_system_message = patch.get("default_system_message").and_then(|x| x.as_str()).unwrap_or("").to_string();
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
        self.dd.stop_list.retain(|x|!x.is_empty());

        Ok(())
    }

    async fn prompt(
        &mut self,
        context_size: usize,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        let augmented_msgs = match *self.vecdb_search.lock().await {
            Some(ref db) => embed_vecdb_results(db, &self.post.messages, 6).await,
            None => { self.post.messages.clone() }
        };
        let limited_msgs: Vec<ChatMessage> = limit_messages_history(&self.t, &augmented_msgs, self.post.parameters.max_new_tokens, context_size, &self.default_system_message)?;
        sampling_parameters_to_patch.stop = Some(self.dd.stop_list.clone());
        // adapted from https://huggingface.co/spaces/huggingface-projects/llama-2-13b-chat/blob/main/model.py#L24
        let mut prompt = "".to_string();
        let mut last_role = "assistant".to_string();
        for msg in limited_msgs {
            prompt.push_str(self.token_esc.as_str());
            if msg.role == "system" {
                prompt.push_str(self.keyword_syst.as_str());
                prompt.push_str(msg.content.as_str());
                prompt.push_str("\n");
            } else if msg.role == "user" {
                prompt.push_str(self.keyword_user.as_str());
                prompt.push_str(msg.content.as_str());
                prompt.push_str("\n");
            } else if msg.role == "assistant" {
                prompt.push_str(self.keyword_asst.as_str());
                prompt.push_str(msg.content.as_str());
                prompt.push_str("\n");
            } else if msg.role == "context_file" {
                let vector_of_context_files: Vec<ContextFile> = serde_json::from_str(&msg.content).unwrap(); // FIXME unwrap
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
            info!("chat re-encode whole prompt again gives {} tokes", self.t.count_tokens(prompt.as_str())?);
        }
        Ok(prompt)
    }

    fn response_n_choices(
        &mut self,
        choices: Vec<String>,
        stopped: Vec<bool>,
    ) -> Result<serde_json::Value, String> {
        self.dd.response_n_choices(choices, stopped)
    }

    fn response_streaming(
        &mut self,
        delta: String,
        stop_toks: bool,
        stop_length: bool,
    ) -> Result<(serde_json::Value, bool), String> {
        self.dd.response_streaming(delta, stop_toks)
    }
}

