use std::sync::{Arc, RwLock};

use serde_json::Value;
use tokenizers::Tokenizer;
use tracing::{info, warn};

use crate::{cached_tokenizers, scratchpads};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::execute_at_file;
use crate::at_tools::att_patch::args_parser::PatchArguments;
use crate::at_tools::att_patch::ast_interaction::{get_signatures_by_imports_traversal, get_signatures_by_symbol_names};
use crate::at_tools::att_patch::tool::DefaultToolPatch;
use crate::call_validation::{ChatMessage, ChatPost, ChatUsage, SamplingParameters};
use crate::scratchpads::chat_utils_rag::count_tokens;


async fn make_chat_history(
    args: &PatchArguments,
    ccx: &mut AtCommandsContext,
    tokenizer: Arc<RwLock<Tokenizer>>,
) -> Result<Vec<ChatMessage>, String> {
    let system_prompt = DefaultToolPatch::prompt();
    // TODO: use budget for extra context construction
    let maybe_extra_context = if let Some(symbols_names) = args.symbol_names.clone() {
        get_signatures_by_symbol_names(&symbols_names, ccx.global_context.clone()).await
    } else {
        get_signatures_by_imports_traversal(&args.paths, ccx.global_context.clone()).await

    };
    let mut tokens: usize = 0;
    let max_tokens: usize = crate::at_tools::att_patch::tool::MAX_TOKENS - crate::at_tools::att_patch::tool::MAX_NEW_TOKENS;
    let tokenizer_ref = tokenizer.read().unwrap().clone();
    let task_message = format!("The task is:\n{}", args.todo).to_string();
    let mut chat_messages = vec![
        ChatMessage::new(
            "system".to_string(),
            system_prompt.to_string(),
        )
    ];
    tokens += 3 + count_tokens(&tokenizer_ref, &system_prompt);
    tokens += 3 + count_tokens(&tokenizer_ref, &task_message);
    if tokens > max_tokens {
        return Err(format!("too many tokens: {tokens} > {max_tokens}"));
    }

    let has_single_file = args.paths.len() == 1;
    for (idx, file) in args.paths.iter().enumerate() {
        match execute_at_file(ccx, file.clone()).await {
            Ok(res) => {
                let message = format!("{}\n```\n{}```\n\n", res.file_name, res.file_content).to_string();
                tokens += 3 + count_tokens(&tokenizer_ref, &message);
                if tokens > max_tokens {
                    let err_message = if has_single_file || idx == 0 {
                        format!("the provided file {file} is too large for the patch tool: {tokens} > {max_tokens}")
                    } else {
                        format!("too many files are provided: {tokens} ctx > {max_tokens} max available ctx, use the tool for each file separately")
                    };
                    return Err(err_message);
                }
                chat_messages.push(ChatMessage::new("user".to_string(), message));
            }
            Err(err) => {
                warn!("cannot find a `{file}`: {err}, be sure that the input file exists");
            }
        }
    }
    if let Some(extra_context) = maybe_extra_context {
        let message = format!("Extra context for the files:\n{}", extra_context).to_string();
        tokens += 3 + count_tokens(&tokenizer_ref, &message);
        if tokens > max_tokens {
            warn!("Too many tokens for the extra context, skipping it: {tokens} > {max_tokens}");
        } else {
            chat_messages.push(ChatMessage::new("user".to_string(), message));
        }
    }

    chat_messages.push(ChatMessage::new("user".to_string(), task_message));
    info!("tokens num: {tokens}");
    Ok(chat_messages)
}

pub async fn execute_chat_model(
    args: &PatchArguments,
    ccx: &mut AtCommandsContext,
) -> Result<(Vec<String>, Option<ChatUsage>), String> {
    let gx = ccx.global_context.clone();
    let n_choices: Option<usize> = Some(12);
    let mut chat_post = ChatPost {
        messages: vec![],
        parameters: SamplingParameters {
            max_new_tokens: crate::at_tools::att_patch::tool::MAX_NEW_TOKENS,
            temperature: Some(crate::at_tools::att_patch::tool::TEMPERATURE),
            top_p: None,
            stop: vec![],
            n: n_choices,
        },
        model: crate::at_tools::att_patch::tool::DEFAULT_MODEL_NAME.to_string(),
        scratchpad: "".to_string(),
        stream: Some(false),
        temperature: Some(crate::at_tools::att_patch::tool::TEMPERATURE),
        max_tokens: crate::at_tools::att_patch::tool::MAX_TOKENS,
        n: n_choices,
        tools: None,
        tool_choice: None,
        only_deterministic_messages: false,
        chat_id: "".to_string(),
    };
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(
        gx.clone(), 0,
    )
        .await
        .map_err(|e| {
            warn!("no caps: {:?}", e);
            "network error communicating with the model (1)".to_string()
        })?;

    let (model_name, scratchpad_name, scratchpad_patch, n_ctx, _) = crate::http::routers::v1::chat::lookup_chat_scratchpad(
        caps.clone(),
        &chat_post,
    ).await?;
    let (client, api_key) = {
        let cx_locked = gx.write().await;
        (cx_locked.http_client.clone(), cx_locked.cmdline.api_key.clone())
    };

    let tokenizer = cached_tokenizers::cached_tokenizer(
        caps.clone(), gx.clone(), model_name.clone(),
    ).await?;

    chat_post.messages = make_chat_history(
        args, ccx, tokenizer,
    ).await?;

    let mut scratchpad = scratchpads::create_chat_scratchpad(
        gx.clone(),
        caps.clone(),
        model_name.clone(),
        &chat_post.clone(),
        &scratchpad_name,
        &scratchpad_patch,
        false,
        false,
    ).await?;
    let prompt = scratchpad.prompt(
        n_ctx,
        &mut chat_post.parameters,
    ).await?;

    let t1 = std::time::Instant::now();
    let messages = crate::restream::scratchpad_interaction_not_stream_json(
        gx.clone(),
        scratchpad,
        "chat".to_string(),
        &prompt,
        model_name,
        client,
        api_key,
        &chat_post.parameters,
        chat_post.only_deterministic_messages,
    ).await.map_err(|e| {
        warn!("network error communicating with the (2): {:?}", e);
        "network error communicating with the model (2)".to_string()
    })?;
    info!("patch generation took {:?}ms", t1.elapsed().as_millis() as i32);

    let usage_mb = match messages.get("usage") {
        Some(Value::Object(o)) => {
            match serde_json::from_value::<ChatUsage>(Value::Object(o.clone())) {
                Ok(usage) => Some(usage),
                Err(e) => {
                    warn!("Failed to parse usage object: {:?}; Metering is lost", e);
                    None
                }
            }
        },
        Some(v) => {
            warn!("usage is not a dict: {:?}; Metering is lost", v);
            None
        },
        None => {
            warn!("no usage object in the JSON output. Metering is lost");
            None
        }
    };
    
    let choices_array = match messages["choices"].as_array() {
        Some(array) => array,
        None => return Err("unable to get choices array from JSON".to_string()),
    };

    let mut choices = Vec::new();
    for item in choices_array {
        if let Some(content) = item.get("message")
            .and_then(|msg| msg.get("content"))
            .and_then(|content| content.as_str()) {
            choices.push(content.to_string());
        }
    }

    Ok((choices, usage_mb.clone()))
}