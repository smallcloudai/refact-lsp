use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction, ChatUsage};

pub async fn update_usage(usage: Arc<AMutex<ChatUsage>>, usage_collector: &mut ChatUsage) {
    let mut usage_lock = usage.lock().await;
    usage_lock.prompt_tokens += usage_collector.prompt_tokens;
    usage_lock.completion_tokens += usage_collector.completion_tokens;
    usage_lock.total_tokens += usage_collector.total_tokens;
}

pub fn pretend_tool_call(tool_name: &str, tool_arguments: &str) -> ChatMessage {
    let tool_call = ChatToolCall {
        id: format!("{tool_name}_123"),
        function: ChatToolFunction {
            arguments: tool_arguments.to_string(),
            name: tool_name.to_string()
        },
        tool_type: "function".to_string(),
    };
    ChatMessage {
        role: "assistant".to_string(),
        content: "".to_string(),
        tool_calls: Some(vec![tool_call]),
        tool_call_id: "".to_string(),
        ..Default::default()
    }
}

pub fn reduce_by_counter<I>(values: I, top_n: usize) -> Vec<String>
where
    I: Iterator<Item = String>,
{
    let mut counter = HashMap::new();
    for s in values {
        *counter.entry(s).or_insert(0) += 1;
    }
    let mut counts_vec: Vec<(String, usize)> = counter.into_iter().collect();
    counts_vec.sort_by(|a, b| b.1.cmp(&a.1));
    let top_n: Vec<(String, usize)> = counts_vec.into_iter().take(top_n).collect();
    top_n.into_iter().map(|x|x.0).collect()
}
