use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use async_trait::async_trait;
use serde_json::Value;
use tokenizers::Tokenizer;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;
use tracing::{error, info};

use crate::at_commands::execute_at::run_at_commands;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatPost, SamplingParameters};
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::scratchpads::chat_utils_limit_history::limit_messages_history;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::scratchpads::chat_utils_prompts::{get_default_system_prompt, get_default_system_prompt_from_remote, system_prompt_add_workspace_info};
use crate::scratchpads::passthrough_convert_messages::convert_messages_to_openai_format;
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

    pub fn feed_delta(&mut self, role: &str, delta: &str, finish_reason: &str, tool_calls: Option<Value>) -> Value {
        let x = serde_json::json!([{
            "index": 0,
            "delta": {
                "role": if role != self.role_sent.as_str() { serde_json::Value::String(role.to_string()) } else { serde_json::Value::Null },
                "content": delta,
                "tool_calls": tool_calls.unwrap_or(serde_json::Value::Null),
            },
            "finish_reason": if finish_reason == "" { serde_json::Value::Null } else { serde_json::Value::String(finish_reason.to_string()) }
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
    pub default_system_message: String,
    pub has_rag_results: HasRagResults,
    pub delta_sender: DeltaSender,
    pub global_context: Arc<ARwLock<GlobalContext>>,
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
        global_context: Arc<ARwLock<GlobalContext>>,
        allow_at: bool,
        supports_tools: bool,
        supports_clicks: bool,
        endpoint_style: &str,
    ) -> Self {
        ChatPassthrough {
            t: HasTokenizerAndEot::new(tokenizer),
            post: post.clone(),
            messages: messages.clone(),
            default_system_message: "".to_string(),
            has_rag_results: HasRagResults::new(),
            delta_sender: DeltaSender::new(),
            global_context,
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
        exploration_tools: bool,
        agentic_tools: bool,
        should_execute_remotely: bool,
    ) -> Result<(), String> {
        self.default_system_message = if should_execute_remotely {
            get_default_system_prompt_from_remote(self.global_context.clone(), exploration_tools, agentic_tools, &self.post.chat_id).await?
        } else {
            get_default_system_prompt(self.global_context.clone(), exploration_tools, agentic_tools).await
        };
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

        // TODO? Maybe we should execute at commands remotely.
        let (mut messages, undroppable_msg_n, _any_context_produced) = if self.allow_at && !should_execute_remotely {
            run_at_commands(ccx.clone(), self.t.tokenizer.clone(), sampling_parameters_to_patch.max_new_tokens, &self.messages, &mut self.has_rag_results).await
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
        let mut limited_msgs = limit_messages_history(&self.t, &messages, undroppable_msg_n, sampling_parameters_to_patch.max_new_tokens, n_ctx, &self.default_system_message).unwrap_or_else(|e| {
            error!("error limiting messages: {}", e);
            vec![]
        });
        if let Some(first_msg) = limited_msgs.first_mut() {
            if first_msg.role == "system" {
                first_msg.content = ChatContent::SimpleText(system_prompt_add_workspace_info(gcx.clone(), &first_msg.content.content_text_only()).await);
            }
            if self.post.model == "o1-mini" && first_msg.role == "system" {
                limited_msgs.remove(0);
            }
        }
        if DEBUG {
            info!("chat passthrough {} messages -> {} messages after applying at-commands and limits, possibly adding the default system message", messages.len(), limited_msgs.len());
        }

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

    fn response_n_choices(  // result of old-school OpenAI with text (not messages) which is not possible when using passthrough (means messages)
        &mut self,
        _choices: Vec<String>,
        _stopped: Vec<bool>,
    ) -> Result<serde_json::Value, String> {
        todo!();
    }

    fn response_streaming(
        &mut self,
        delta: String,
        stop_toks: bool,
        stop_length: bool,
    ) -> Result<(serde_json::Value, bool), String> {
        let finished = stop_toks || stop_length;
        let finish_reason = if finished {
            if stop_toks { "stop".to_string() } else { "length".to_string() }
        } else {
            "".to_string()
        };
        let json_choices = self.delta_sender.feed_delta("assistant", &delta, &finish_reason, None);
        let ans = serde_json::json!({
            "choices": json_choices,
            "object": "chat.completion.chunk",
        });
        Ok((ans, finished))
    }

    fn response_spontaneous(&mut self) -> Result<Vec<Value>, String>  {
        self.has_rag_results.response_streaming()
    }
}

// for anthropic:
// tool answers must be located in the same message.content (if tools executed in parallel)
fn format_messages_anthropic(messages: Vec<Value>) -> Vec<Value> {
    let mut res: Vec<Value> = vec![];
    for m in messages {
        match m.get("content") {
            Some(Value::Array(cont)) => {
                if let Some(prev_el) = res.last_mut() {
                    if let Some(Value::Array(prev_cont)) = prev_el.get_mut("content") {
                        if cont.iter().any(|c| c.get("type") == Some(&Value::String("tool_result".to_string())))
                            && prev_cont.iter().any(|p| p.get("type") == Some(&Value::String("tool_result".to_string())))
                        {
                            prev_cont.extend(cont.iter().cloned());
                            continue;
                        }
                    }
                }
                res.push(m);
            }
            _ => res.push(m),
        }
    }
    res
}
