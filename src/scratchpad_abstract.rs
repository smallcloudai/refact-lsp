use serde_json;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::Mutex as AMutex;
use tokenizers::Tokenizer;
use async_trait::async_trait;
use serde_json::Value;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::SamplingParameters;


#[async_trait]
pub trait TextScratchpadAbstract: Send {
    async fn apply_model_adaptation_patch(
        &mut self,
        patch: &Value,
        exploration_tools: bool,
        agentic_tools: bool,
        should_execute_remotely: bool,
    ) -> Result<(), String>;

    async fn prompt(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String>;

    fn response_n_choices(   // Not streaming, convert what model says (choices) to final result
                             &mut self,
                             choices: Vec<String>,
                             stopped: Vec<bool>,
    ) -> Result<Value, String>;

    fn response_streaming(   // Only 1 choice, but streaming. Returns delta the user should see, and finished flag
                             &mut self,
                             delta: String,       // if delta is empty, there is no more input, add final fields if needed
                             stop_toks: bool,
                             stop_length: bool,
    ) -> Result<(Value, bool), String>;

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String>;
}

#[async_trait]
pub trait MessagesScratchpadAbstract: Send {
    async fn apply_model_adaptation_patch(
        &mut self,
        patch: &Value,
        exploration_tools: bool,
        agentic_tools: bool,
        should_execute_remotely: bool,
    ) -> Result<(), String>;

    async fn prompt(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String>;

    fn response_n_choices(   // Not streaming, convert what model says (choices) to final result
                             &mut self,
                             choices: Vec<String>,
                             stopped: Vec<bool>,
    ) -> Result<Value, String>;

    fn response_streaming(   // Only 1 choice, but streaming. Returns delta the user should see, and finished flag
                             &mut self,
                             delta: &Value,       // if delta is empty, there is no more input, add final fields if needed
                             stop_toks: bool,
                             stop_length: bool,
    ) -> Result<(Value, bool), String>;

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String>;
}


pub enum ScratchpadAbstract {
    Text(Box<dyn TextScratchpadAbstract>),
    Messages(Box<dyn MessagesScratchpadAbstract>),
}

impl ScratchpadAbstract {
    pub async fn apply_model_adaptation_patch(
        &mut self,
        patch: &Value,
        exploration_tools: bool,
        agentic_tools: bool,
        should_execute_remotely: bool,
    ) -> Result<(), String> {
        match self {
            ScratchpadAbstract::Text(text_scratchpad) => {
                text_scratchpad.apply_model_adaptation_patch(patch, exploration_tools, agentic_tools, should_execute_remotely).await
            }
            ScratchpadAbstract::Messages(messages_scratchpad) => {
                messages_scratchpad.apply_model_adaptation_patch(patch, exploration_tools, agentic_tools, should_execute_remotely).await
            }
        }
    }

    pub async fn prompt(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        match self {
            ScratchpadAbstract::Text(text_scratchpad) => {
                Box::pin(text_scratchpad.prompt(ccx, sampling_parameters_to_patch)).await
            }
            ScratchpadAbstract::Messages(messages_scratchpad) => {
                Box::pin(messages_scratchpad.prompt(ccx, sampling_parameters_to_patch)).await
            }
        }
    }

    pub fn response_spontaneous(&mut self) -> Result<Vec<Value>, String> {
        match self {
            ScratchpadAbstract::Text(text_scratchpad) => {
                text_scratchpad.response_spontaneous()
            }
            ScratchpadAbstract::Messages(messages_scratchpad) => {
                messages_scratchpad.response_spontaneous()
            }
        }
    }
}

// aggregate this struct to make scratchpad implementation easier
#[derive(Debug, Clone)]
pub struct HasTokenizerAndEot {
    pub tokenizer: Arc<RwLock<Tokenizer>>,
    pub eot: String,
    pub eos: String,
    pub context_format: String,
    pub rag_ratio: f64,
}

impl HasTokenizerAndEot {
    pub fn new(tokenizer: Arc<RwLock<Tokenizer>>) -> Self {
        HasTokenizerAndEot { tokenizer, eot: String::new(), eos: String::new(), context_format: String::new(), rag_ratio: 0.5}
    }

    pub fn count_tokens(
        &self,
        text: &str,
    ) -> Result<i32, String> {
        let tokenizer = self.tokenizer.write().unwrap();
        let tokens = tokenizer.encode(text, false).map_err(|err| {
            return format!("Encoding error: {}", err);
        })?;
        Ok(tokens.len() as i32)
    }

    pub fn assert_one_token(
        &self,
        text: &str
    ) -> Result<(), String> {
        let tokenizer = self.tokenizer.write().unwrap();
        let tokens = tokenizer.encode(text, false).map_err(|err| {
            format!("assert_one_token: {}", err)
        })?;
        if tokens.len() != 1 {
            return Err(format!("assert_one_token: expected 1 token for \"{}\", got {}", text, tokens.len()));
        }
        Ok(())
    }
}
