use std::sync::{Arc, RwLockReadGuard};
use std::sync::RwLock;
use tokenizers::Tokenizer;
use crate::scratchpads::chat_message::{ChatContent, ChatMessage, ChatMultimodalElement, MultimodalElementTextOpenAI};
use crate::scratchpads::scratchpad_utils::{count_tokens_text_only, multimodal_image_count_tokens};


fn limit_text_content(
    tokenizer_guard: &RwLockReadGuard<Tokenizer>,
    text: &String,
    tok_used: &mut usize,
    tok_per_m: usize,
) -> String {
    let mut new_text_lines = vec![];
    for line in text.lines() {
        let line_tokens = count_tokens_text_only(tokenizer_guard, &line);
        if tok_used.clone() + line_tokens > tok_per_m {
            if new_text_lines.is_empty() {
                new_text_lines.push("No content: tokens limit reached");
            }
            new_text_lines.push("Truncated: too many tokens\n");
            break;
        }
        *tok_used += line_tokens;
        new_text_lines.push(line);
    }
    new_text_lines.join("\n")
}

pub async fn postprocess_plain_text(
    plain_text_messages: Vec<&ChatMessage>,
    tokenizer: Arc<RwLock<Tokenizer>>,
    tokens_limit: usize,
) -> (Vec<ChatMessage>, usize) {
    if plain_text_messages.is_empty() {
        return (vec![], tokens_limit);
    }
    let mut messages_sorted = plain_text_messages.clone();
    messages_sorted.sort_by(|a, b| a.content.size_estimate(tokenizer.clone()).cmp(&b.content.size_estimate(tokenizer.clone())));

    let mut tok_used_global = 0;
    let mut tok_per_m = tokens_limit / messages_sorted.len();
    let mut new_messages = vec![];

    let tokenizer_guard = tokenizer.read().unwrap();
    for (idx, msg) in messages_sorted.iter().cloned().enumerate() {
        let mut tok_used = 0;
        let mut m_cloned = msg.clone();
        
        m_cloned.content = match &msg.content {
            ChatContent::SimpleText(text) => {
                let new_content = limit_text_content(&tokenizer_guard, text, &mut tok_used, tok_per_m);
                ChatContent::SimpleText(new_content)
            },
            ChatContent::Multimodal(elements) => {
                let mut new_content = vec![];
                
                for element in elements {
                    match element {
                        ChatMultimodalElement::MultimodalElementTextOpenAI(text_el) => {
                            new_content.push(ChatMultimodalElement::MultimodalElementTextOpenAI(MultimodalElementTextOpenAI {
                                content_type: text_el.content_type.clone(),
                                text: limit_text_content(&tokenizer_guard, &text_el.text, &mut tok_used, tok_per_m)
                            }));
                        },
                        ChatMultimodalElement::MultiModalImageURLElementOpenAI(image_el) => {
                            let tokens = multimodal_image_count_tokens(image_el);
                            if tok_used + tokens > tok_per_m {
                                new_content.push(ChatMultimodalElement::MultimodalElementTextOpenAI(MultimodalElementTextOpenAI {
                                    content_type: "text".to_string(),
                                    text: "Image truncated: too many tokens".to_string()
                                }));
                            } else {
                                new_content.push(ChatMultimodalElement::MultiModalImageURLElementOpenAI(image_el.clone()));
                                tok_used += tokens;
                            }
                        }
                    };
                }
                ChatContent::Multimodal(new_content)
            }
        };

        if idx != messages_sorted.len() - 1 {
            // distributing non-used rest of tokens among the others
            tok_per_m += (tok_per_m - tok_used) / (messages_sorted.len() - idx - 1);
        }
        tok_used_global += tok_used;

        new_messages.push(m_cloned);
    }

    let tok_unused = tokens_limit.saturating_sub(tok_used_global);
    (new_messages, tok_unused)
}
