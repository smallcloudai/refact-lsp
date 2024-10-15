use std::io::Cursor;
use image::ImageReader;

use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::call_validation::ChatMessage;
use std::collections::HashSet;
use tracing::info;

pub fn limit_messages_history(
    t: &HasTokenizerAndEot,
    messages: &Vec<ChatMessage>,
    last_user_msg_starts: usize,
    max_new_tokens: usize,
    context_size: usize,
    default_system_message: &String,
) -> Result<Vec<ChatMessage>, String>
{
    let tokens_limit: i32 = context_size as i32 - max_new_tokens as i32;
    tracing::info!("limit_messages_history tokens_limit={} because context_size={} and max_new_tokens={}", tokens_limit, context_size, max_new_tokens);
    let mut tokens_used: i32 = 0;
    let mut message_token_count: Vec<i32> = vec![0; messages.len()];
    let mut message_take: Vec<bool> = vec![false; messages.len()];
    let mut have_system = false;
    for (i, msg) in messages.iter().enumerate() {
        info!("counting tokens");
        let tcnt = 3 + msg.content.count_tokens(t.tokenizer.clone())?;
        info!("tokens_count={}", tcnt);
        message_token_count[i] = tcnt;
        if i==0 && msg.role == "system" {
            message_take[i] = true;
            tokens_used += tcnt;
            have_system = true;
        }
        if i >= last_user_msg_starts {
            message_take[i] = true;
            tokens_used += tcnt;
        }
    }
    let need_default_system_msg = !have_system && default_system_message.len() > 0;
    if need_default_system_msg {
        let tcnt = t.count_tokens(default_system_message.as_str())? as i32;
        tokens_used += tcnt;
    }
    let mut log_buffer = Vec::new();
    let mut dropped = false;

    for i in (0..messages.len()).rev() {
        let tcnt = 3 + message_token_count[i];
        if !message_take[i] {
            if tokens_used + tcnt < tokens_limit {
                message_take[i] = true;
                tokens_used += tcnt;
                log_buffer.push(format!("take {:?}, tokens_used={} < {}", crate::nicer_logs::first_n_chars(&messages[i].content.content_text_only(), 30), tokens_used, tokens_limit));
            } else {
                log_buffer.push(format!("DROP {:?} with {} tokens, quit", crate::nicer_logs::first_n_chars(&messages[i].content.content_text_only(), 30), tcnt));
                dropped = true;
                break;
            }
        } else {
            message_take[i] = true;
            log_buffer.push(format!("not allowed to drop {:?}, tokens_used={} < {}", crate::nicer_logs::first_n_chars(&messages[i].content.content_text_only(), 30), tokens_used, tokens_limit));
        }
    }

    if dropped {
        tracing::info!("\n{}", log_buffer.join("\n"));
    }

    // additinally, drop tool results if we drop the calls
    let mut tool_call_id_drop = HashSet::new();
    for i in 0..messages.len() {
        if message_take[i] {
            continue;
        }
        if let Some(tool_calls) = &messages[i].tool_calls {
            for call in tool_calls {
                tool_call_id_drop.insert(call.id.clone());
            }
        }
    }
    for i in 0..messages.len() {
        if !message_take[i] {
            continue;
        }
        if tool_call_id_drop.contains(messages[i].tool_call_id.as_str()) {
            message_take[i] = false;
            tracing::info!("drop {:?} because of drop tool result rule", crate::nicer_logs::first_n_chars(&messages[i].content.content_text_only(), 30));
        }
    }

    let mut messages_out: Vec<ChatMessage> = messages.iter().enumerate().filter(|(i, _)| message_take[*i]).map(|(_, x)| x.clone()).collect();
    if need_default_system_msg {
        messages_out.insert(0, ChatMessage::new("system".to_string(), default_system_message.clone()));
    }
    // info!("messages_out: {:?}", messages_out);
    Ok(messages_out)
}

fn calculate_image_tokens_by_dimensions(mut width: u32, mut height: u32) -> i32 {
    // as per https://platform.openai.com/docs/guides/vision
    const SMALL_CHUNK_SIZE: u32 = 512;
    const COST_PER_SMALL_CHUNK: i32 = 170;
    const BIG_CHUNK_SIZE: u32 = 2048;
    const CONST_COST: i32 = 85;

    let shrink_factor = (width.max(height) as f64) / (BIG_CHUNK_SIZE as f64);
    if shrink_factor > 1.0 {
        width = (width as f64 / shrink_factor) as u32;
        height = (height as f64 / shrink_factor) as u32;
    }

    let width_chunks = (width as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    let height_chunks = (height as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    let small_chunks_needed = width_chunks * height_chunks;

    small_chunks_needed as i32 * COST_PER_SMALL_CHUNK + CONST_COST
}

// for detail = high. all images w detail = low cost 85 tokens (independent of image size)
pub fn calculate_image_tokens_openai(image_string: &String, detail: &String) -> Result<i32, String> {
    #[allow(deprecated)]
    let image_bytes = base64::decode(image_string).map_err(|_| "base64 decode failed".to_string())?;
    let cursor = Cursor::new(image_bytes);
    let reader = ImageReader::new(cursor).with_guessed_format().map_err(|e| e.to_string())?;
    let (width, height) = reader.into_dimensions().map_err(|_| "Failed to get dimensions".to_string())?;

    match detail.as_str() {
        "high" => Ok(calculate_image_tokens_by_dimensions(width, height)),
        "low" => Ok(85),
        _ => Err("detail must be one of high or low".to_string()),
    }
}

// cargo test scratchpads::chat_utils_limit_history
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_image_tokens_by_dimensions() {
        let width = 1024;
        let height = 1024;
        let expected_tokens = 765;
        let tokens = calculate_image_tokens_by_dimensions(width, height);
        assert_eq!(tokens, expected_tokens, "Expected {} tokens, but got {}", expected_tokens, tokens);
    }
}
