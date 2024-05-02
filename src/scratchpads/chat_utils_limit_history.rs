use std::io::Cursor;
use image::io::Reader as ImageReader;
use tracing::{error, info};

use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::call_validation::ChatMessage;


fn calculate_image_tokens_openai(image_string: &String) -> Result<i32, String> {
    // as per https://platform.openai.com/docs/guides/vision
    const SMALL_CHUNK_SIZE: u32 = 512;
    const COST_PER_SMALL_CHUNK: i32 = 170;
    const BIG_CHUNK_SIZE: u32 = 2048;
    const CONST_COST: i32 = 85;

    let image_bytes = base64::decode(image_string).map_err(|_| "base64 decode failed".to_string())?;
    let cursor = Cursor::new(image_bytes);
    let reader = ImageReader::new(cursor).with_guessed_format().map_err(|e| e.to_string())?;
    let (mut width, mut height) = reader.into_dimensions().map_err(|_| "Failed to get dimensions".to_string())?;

    let shrink_factor = (width.max(height) as f64) / (BIG_CHUNK_SIZE as f64);
    if shrink_factor > 1.0 {
        width = (width as f64 / shrink_factor) as u32;
        height = (height as f64 / shrink_factor) as u32;
    }

    let width_chunks = (width as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    let height_chunks = (height as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    let small_chunks_needed = width_chunks * height_chunks;
    
    Ok(small_chunks_needed as i32 * COST_PER_SMALL_CHUNK + CONST_COST)
}

fn calculate_t_cnt(msg: &ChatMessage, t: &HasTokenizerAndEot) -> Result<i32, String> {
    return if msg.kind == "text" {
        Ok(3 + t.count_tokens(msg.content.as_str())?)
    }
    else if msg.kind == "image" {
        let t_cnt = calculate_image_tokens_openai(&msg.content).unwrap_or_else(|e| {
            error!("calculate_image_tokens_openai failed: {}; applying max value: 2805", e);
            2805
        });
        Ok(t_cnt)
    }
    else {
        Err(format!("unknown msg kind: {}", msg.kind))
    }
}

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
    info!("limit_messages_history tokens_limit={} <= context_size={} - max_new_tokens={}", tokens_limit, context_size, max_new_tokens);
    let mut tokens_used: i32 = 0;
    let mut message_token_count: Vec<i32> = vec![0; messages.len()];
    let mut message_take: Vec<bool> = vec![false; messages.len()];
    let mut have_system = false;
    for (i, msg) in messages.iter().enumerate() {
        let t_cnt = calculate_t_cnt(msg, t)?;
        message_token_count[i] = t_cnt;
        if i==0 && msg.role == "system" {
            message_take[i] = true;
            tokens_used += t_cnt;
            have_system = true;
        }
        if i >= last_user_msg_starts {
            message_take[i] = true;
            tokens_used += t_cnt;
        }
    }
    let need_default_system_msg = !have_system && default_system_message.len() > 0;
    if need_default_system_msg {
        let tcnt = t.count_tokens(default_system_message.as_str())? as i32;
        tokens_used += tcnt;
    }
    for i in (0..messages.len()).rev() {
        let t_cnt = 3 + message_token_count[i];
        if !message_take[i] {
            if tokens_used + t_cnt < tokens_limit {
                message_take[i] = true;
                tokens_used += t_cnt;
                info!("take {:?}, tokens_used={} < {}", crate::nicer_logs::first_n_chars(&messages[i].content, 30), tokens_used, tokens_limit);
            } else {
                info!("drop {:?} with {} tokens, quit", crate::nicer_logs::first_n_chars(&messages[i].content, 30), t_cnt);
                break;
            }
        } else {
            info!("not allowed to drop {:?}, tokens_used={} < {}", crate::nicer_logs::first_n_chars(&messages[i].content, 30), tokens_used, tokens_limit);
        }
    }
    let mut messages_out: Vec<ChatMessage> = messages.iter().enumerate().filter(|(i, _)| message_take[*i]).map(|(_, x)| x.clone()).collect();
    if need_default_system_msg {
        messages_out.insert(0, ChatMessage {
            role: "system".to_string(),
            content: default_system_message.clone(),
            kind: "text".to_string(),
        });
    }
    Ok(messages_out)
}
