use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::call_validation::ChatMessage;


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
    tracing::info!("limit_messages_history tokens_limit={} <= context_size={} - max_new_tokens={}", tokens_limit, context_size, max_new_tokens);
    let mut tokens_used: i32 = 0;
    let mut message_token_count: Vec<i32> = vec![0; messages.len()];
    let mut message_take: Vec<bool> = vec![false; messages.len()];
    let mut have_system = false;
    for (i, msg) in messages.iter().enumerate() {
        let tcnt = (3 + t.count_tokens(msg.content.as_str())?) as i32;  // 3 for role "\n\nASSISTANT:" kind of thing
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
    for i in (0..messages.len()).rev() {
        let tcnt = 3 + message_token_count[i];
        if !message_take[i] {
            if tokens_used + tcnt < tokens_limit {
                message_take[i] = true;
                tokens_used += tcnt;
                tracing::info!("take {:?}, tokens_used={} < {}", crate::nicer_logs::first_n_chars(&messages[i].content, 30), tokens_used, tokens_limit);
            } else {
                tracing::info!("drop {:?} with {} tokens, quit", crate::nicer_logs::first_n_chars(&messages[i].content, 30), tcnt);
                break;
            }
        } else {
            tracing::info!("not allowed to drop {:?}, tokens_used={} < {}", crate::nicer_logs::first_n_chars(&messages[i].content, 30), tokens_used, tokens_limit);
        }
    }
    let mut messages_out: Vec<ChatMessage> = messages.iter().enumerate().filter(|(i, _)| message_take[*i]).map(|(_, x)| x.clone()).collect();
    if need_default_system_msg {
        messages_out.insert(0, ChatMessage {
            role: "system".to_string(),
            content: default_system_message.clone(),
        });
    }
    Ok(messages_out)
}
