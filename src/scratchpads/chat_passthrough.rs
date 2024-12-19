use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use serde_json::{json, Value};
use tokenizers::Tokenizer;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use tracing::{error, info};

use crate::at_commands::execute_at::run_at_commands;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatPost, SamplingParameters};
use crate::scratchpad_abstract::{FinishReason, HasTokenizerAndEot, ScratchpadAbstract};
use crate::scratchpads::chat_utils_limit_history::limit_messages_history;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::scratchpads::chat_utils_prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use crate::scratchpads::passthrough_convert_messages::{convert_messages_to_openai_format, format_messages_anthropic};
use crate::tools::tools_description::{tool_description_list_from_yaml, tools_merged_and_filtered};
use crate::tools::tools_execute::{run_tools_locally, run_tools_remotely};


const DEBUG: bool = false;


pub struct DeltaSender {
    pub role_sent: String,
}

impl DeltaSender {
    pub fn new() -> Self {
        DeltaSender {
            role_sent: "".to_string(),
        }
    }

    pub fn feed_delta(&mut self, role: &str, _json: &Value, finish_reason: &FinishReason, tool_calls: Option<Value>) -> Value {
        // TODO: correctly implement it
        let x = json!([{
            "index": 0,
            "delta": {
                "role": if role != self.role_sent.as_str() { Value::String(role.to_string()) } else { Value::Null },
                "content": "",
                "tool_calls": tool_calls.unwrap_or(Value::Null),
            },
            "finish_reason": finish_reason.to_json_val()
        }]);
        self.role_sent = role.to_string();
        x
    }
}


// #[derive(Debug)]
pub struct ChatPassthrough {
    pub t: HasTokenizerAndEot,
    pub post: ChatPost,
    pub messages: Vec<ChatMessage>,
    pub has_rag_results: HasRagResults,
    pub delta_sender: DeltaSender,
    pub allow_at: bool,
    pub supports_tools: bool,
    pub supports_clicks: bool,
    pub endpoint_style: String,
}

impl ChatPassthrough {
    pub fn new(
        tokenizer: Arc<StdRwLock<Tokenizer>>,
        post: &ChatPost,
        messages: &Vec<ChatMessage>,
        allow_at: bool,
        supports_tools: bool,
        supports_clicks: bool,
        endpoint_style: &str,
    ) -> Self {
        ChatPassthrough {
            t: HasTokenizerAndEot::new(tokenizer),
            post: post.clone(),
            messages: messages.clone(),
            has_rag_results: HasRagResults::new(),
            delta_sender: DeltaSender::new(),
            allow_at,
            supports_tools,
            supports_clicks,
            endpoint_style: endpoint_style.to_string(),
        }
    }
}

#[async_trait]
impl ScratchpadAbstract for ChatPassthrough {
    async fn apply_model_adaptation_patch(
        &mut self,
        _patch: &Value,
        _exploration_tools: bool,
        _agentic_tools: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn prompt(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        sampling_parameters_to_patch: &mut SamplingParameters,
    ) -> Result<String, String> {
        let (gcx, n_ctx, should_execute_remotely) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.n_ctx, ccx_locked.should_execute_remotely)
        };
        let style = self.endpoint_style.clone();
        let allow_experimental = gcx.read().await.cmdline.experimental;
        let at_tools = tools_merged_and_filtered(gcx.clone(), self.supports_clicks).await?;

        let messages = prepend_the_right_system_prompt_and_maybe_more_initial_messages(gcx.clone(), self.messages.clone(), &self.post, &mut self.has_rag_results).await;
        let (mut messages, undroppable_msg_n, _any_context_produced) = if self.allow_at && !should_execute_remotely {
            run_at_commands(ccx.clone(), self.t.tokenizer.clone(), sampling_parameters_to_patch.max_new_tokens, &messages, &mut self.has_rag_results).await
        } else {
            (self.messages.clone(), self.messages.len(), false)
        };
        if self.supports_tools {
            (messages, _) = if should_execute_remotely {
                run_tools_remotely(ccx.clone(), &self.post.model, sampling_parameters_to_patch.max_new_tokens, &messages, &mut self.has_rag_results, &style).await?
            } else {
                run_tools_locally(ccx.clone(), at_tools.clone(), self.t.tokenizer.clone(), sampling_parameters_to_patch.max_new_tokens, &messages, &mut self.has_rag_results, &style).await?
            }
        };
        let limited_msgs = limit_messages_history(&self.t, &messages, undroppable_msg_n, sampling_parameters_to_patch.max_new_tokens, n_ctx, &style).unwrap_or_else(|e| {
            error!("error limiting messages: {}", e);
            vec![]
        });

        let converted_messages = convert_messages_to_openai_format(limited_msgs, &style);
        let converted_messages = if style.as_str() == "anthropic" {
            format_messages_anthropic(converted_messages)
        } else {
            converted_messages
        };

        let mut big_json = serde_json::json!({
            "messages": converted_messages,
        });

        if self.supports_tools {
            let tools = if let Some(tools) = &self.post.tools {
                // if tools.is_empty() || any_context_produced {
                if tools.is_empty() {
                    None
                } else {
                    Some(tools)
                }
            } else {
                None
            };

            let tools_enabled = match tools {
                Some(tools) => {
                    tools.iter().map(|t|t["function"]["name"].as_str().unwrap().to_string()).collect::<Vec<_>>()
                },
                None => vec![]
            };

            let tools_desc_list = tool_description_list_from_yaml(at_tools, &tools_enabled, allow_experimental).await?;
            let tools_filtered = tools_desc_list.iter().filter(|t|tools_enabled.contains(&t.name)).cloned().collect::<Vec<_>>();

            if !tools_filtered.is_empty() {
                if self.endpoint_style == "anthropic" {
                    big_json["tools"] = serde_json::json!(tools_filtered.iter().map(|t|t.clone().into_anthropic_style()).collect::<Vec<_>>());
                } else {
                    big_json["tools"] = serde_json::json!(tools_filtered.iter().map(|t|t.clone().into_openai_style(false)).collect::<Vec<_>>());
                    big_json["tool_choice"] = serde_json::json!(self.post.tool_choice);
                }
            }

            if DEBUG {
                info!("PASSTHROUGH TOOLS ENABLED CNT: {:?}", tools.unwrap_or(&vec![]).len());
            }
        } else {
            if DEBUG {
                info!("PASSTHROUGH TOOLS NOT SUPPORTED");
            }
        }
        let prompt = "PASSTHROUGH ".to_string() + &serde_json::to_string(&big_json).unwrap();
        Ok(prompt.to_string())
    }

    fn response_n_choices(
        &mut self,
        _choices: Vec<String>,
        _finish_reasons: Vec<FinishReason>,
    ) -> Result<Value, String> {
        Err("not implemented".to_string())
    }

    fn response_streaming(
        &mut self,
        _delta: String,
        _finish_reason: FinishReason
    ) -> Result<(Value, FinishReason), String> {
        Err("not implemented".to_string())
    }

    fn response_message_streaming(
        &mut self,
        json: &Value,
        finish_reason: FinishReason,
    ) -> Result<(Value, FinishReason), String> {
        Ok((json.clone(), finish_reason))
    }

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String>  {
        // let mut deterministic: Vec<Value> = vec![];
        // let mut cursor = 0;
        // while cursor < self.messages.len() {

        // }



        // let have_system_prompt_in_post = !self.post.messages.is_empty() && self.post.messages[0].get("role") == Some(&serde_json::Value::String("system".to_string()));
        // let have_system_prompt_in_messages = !self.messages.is_empty() && self.messages[0].role == "system";
        // if !have_system_prompt_in_post && have_system_prompt_in_messages && self.post.messages.len() == 1 {  // only the user message present in request

        //     self.has_rag_results.in_json.insert(0, json!(self.messages[0]));

        // }
        // deterministic.extend(self.has_rag_results.response_streaming()?);
        // Ok(deterministic)
        self.has_rag_results.response_streaming()
    }

    fn streaming_finished(&mut self, finish_reason: FinishReason) -> Result<Value, String> {
        let json_choices = self.delta_sender.feed_delta("assistant", &json!({}), &finish_reason, None);
        Ok(json!({
            "choices": json_choices,
            "object": "chat.completion.chunk",
        }))
    }
}
