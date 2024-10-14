use std::sync::Arc;
use std::sync::RwLock;
use tokenizers::Tokenizer;

use crate::call_validation::ChatMessage;


pub async fn postprocess_plain_text(
    plain_text_messages: Vec<&ChatMessage>,
    tokenizer: Arc<RwLock<Tokenizer>>,
    tokens_limit: usize,
) -> (Vec<ChatMessage>, usize) {
    if plain_text_messages.is_empty() {
        return (vec![], tokens_limit);
    }
    let mut messages_sorted = plain_text_messages.clone();
    messages_sorted.sort_by(|a, b| a.content.size_estimate().cmp(&b.content.size_estimate()));

    let mut tok_used_global = 0;
    let mut tok_per_m = tokens_limit / messages_sorted.len();
    let mut results = vec![];

    let tokenizer_guard = tokenizer.read().unwrap();
    for (idx, msg) in messages_sorted.iter().cloned().enumerate() {
        let mut out = vec![];
        let mut tok_used = 0;
        let text = match &msg.content {
            crate::call_validation::ChatContent::SimpleText(text) => text,
            _ => unreachable!(),
        };
        for line in text.lines() {
            let line_tokens = crate::scratchpads::scratchpad_utils::count_tokens_text_only(&tokenizer_guard, &line);
            if tok_used + line_tokens > tok_per_m {
                if out.is_empty() {
                    out.push("No content: tokens limit reached");
                }
                out.push("Truncated: too many tokens\n");
                break;
            }
            tok_used += line_tokens;
            out.push(line);
        }
        if idx != messages_sorted.len() - 1 {
            // distributing non-used rest of tokens among the others
            tok_per_m += (tok_per_m - tok_used) / (messages_sorted.len() - idx - 1);
        }
        tok_used_global += tok_used;
        let mut m_cloned = msg.clone();
        m_cloned.content = crate::call_validation::ChatContent::SimpleText(out.join("\n"));

        // TODO: find a good way to tell the model how much lines were omitted

        results.push(m_cloned);
    }

    let tok_unused = tokens_limit.saturating_sub(tok_used_global);
    (results, tok_unused)
}

