use tracing::info;
use std::sync::Arc;
// use std::sync::RwLock as StdRwLock;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::call_validation::ChatPost;
use crate::call_validation::ChatMessage;
use crate::call_validation::SamplingParameters;
use crate::scratchpads::chat_utils_limit_history::limit_messages_history_in_bytes;
use crate::vecdb_search::{VecdbSearch};
use crate::scratchpads::chat_utils_vecdb::{HasVecdb, HasVecdbResults};

const DEBUG: bool = true;


// #[derive(Debug)]
pub struct ChatPassthrough {
    pub post: ChatPost,
    pub default_system_message: String,
    pub limit_bytes: usize,
    pub limited_msgs: Vec<ChatMessage>,
    pub vecdb_search: Arc<AMutex<Box<dyn VecdbSearch + Send>>>,
    pub has_vecdb_results: HasVecdbResults,
}

impl ChatPassthrough {
    pub fn new(
        post: ChatPost,
        vecdb_search: Arc<AMutex<Box<dyn VecdbSearch + Send>>>,
    ) -> Self {
        ChatPassthrough {
            post,
            default_system_message: "".to_string(),
            limit_bytes: 4096*3,  // one token translates to 3 bytes (not unicode chars)
            limited_msgs: Vec::new(),
            vecdb_search,
            has_vecdb_results: HasVecdbResults::new(),
        }
    }
}

#[async_trait]
impl ScratchpadAbstract for ChatPassthrough {
    fn apply_model_adaptation_patch(
        &mut self,
        patch: &serde_json::Value,
    ) -> Result<(), String> {
        self.default_system_message = patch.get("default_system_message").and_then(|x| x.as_str()).unwrap_or("").to_string();
        self.limit_bytes = patch.get("limit_bytes").and_then(|x| x.as_u64()).unwrap_or(4096*3) as usize;
        Ok(())
    }

    async fn prompt(
        &mut self,
        context_size: usize,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        // embedding vecdb into messages
        {
            let latest_msg_cont = &self.post.messages.last().unwrap().content;
            let vdb_result_mb = self.vecdb_search.lock().await.search(latest_msg_cont).await;
            self.has_vecdb_results.add2messages(vdb_result_mb, &mut self.post.messages).await;
        }

        let limited_msgs: Vec<ChatMessage> = limit_messages_history_in_bytes(&self.post, context_size, self.limit_bytes, &self.default_system_message)?;
        info!("chat passthrough {} messages -> {} messages after applying limits and possibly adding the default system message", &limited_msgs.len(), &self.limited_msgs.len());
        Ok("".to_string())
    }

    fn response_n_choices(
        &mut self,
        choices: Vec<String>,
        stopped: Vec<bool>,
    ) -> Result<serde_json::Value, String> {
        unimplemented!()
    }

    fn response_streaming(
        &mut self,
        delta: String,
        stop_toks: bool,
        stop_length: bool,
    ) -> Result<(serde_json::Value, bool), String> {
        unimplemented!()
    }

    fn response_spontaneous(&mut self) -> Result<serde_json::Value, String> {
        return self.has_vecdb_results.response_streaming();
    }

}

