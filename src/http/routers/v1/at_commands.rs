use axum::response::Result;
use axum::Extension;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;
use strsim::jaro_winkler;
use itertools::Itertools;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::query::QueryLine;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Serialize, Deserialize, Clone)]
struct CommandCompletionPost {
    query: String,
    cursor: i64,
    top_n: usize,
}
#[derive(Serialize, Deserialize, Clone)]
struct CommandCompletionResponse {
    completions: Vec<String>,
    replace: (i64, i64),
    is_cmd_executable: bool,
}

#[derive(Serialize, Deserialize, Clone)]
struct CommandPreviewPost {
    query: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct CommandPreviewResponse {
    messages: Vec<Value>,
}

pub async fn handle_v1_command_completion(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let context = AtCommandsContext::new(global_context.clone()).await;
    let post = serde_json::from_slice::<CommandCompletionPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let mut completions: Vec<String> = vec![];
    let mut pos1 = -1; let mut pos2 = -1;
    let mut is_cmd_executable = false;

    if let Ok((query_line_val, cursor_rel, cursor_line_start)) = get_line_with_cursor(&post.query, post.cursor) {
        let query_line_val = query_line_val.chars().take(cursor_rel as usize).collect::<String>();
        let query_line = QueryLine::new(query_line_val, cursor_rel, cursor_line_start);
        (completions, is_cmd_executable, pos1, pos2) = command_completion(&query_line, &context, post.cursor, post.top_n).await;
    }

    let response = CommandCompletionResponse {
        completions: completions.clone(),
        replace: (pos1, pos2),
        is_cmd_executable,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

pub async fn handle_v1_command_preview(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let context = AtCommandsContext::new(global_context.clone()).await;
    let post = serde_json::from_slice::<CommandPreviewPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    let mut query = post.query.clone();
    let valid_commands = crate::at_commands::utils::find_valid_at_commands_in_query(&mut query, &context).await;

    let mut preview_msgs = vec![];
    for cmd in valid_commands {
        match cmd.command.lock().await.execute(&post.query, &cmd.args, 5, &context).await {
            Ok(msg) => {
                preview_msgs.push(json!(msg));
            },
            Err(_) => {}
        }
    }

    let response = CommandPreviewResponse {
        messages: preview_msgs,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

fn get_line_with_cursor(query: &String, cursor: i64) -> Result<(String, i64, i64), ScratchError> {
    let mut cursor_rel = cursor;
    for line in query.lines() {
        let line_length = line.len() as i64;
        if cursor_rel <= line_length {
            if !line.starts_with("@") {
                return Err(ScratchError::new(StatusCode::OK, "no command provided".to_string()));
            }
            return Ok((line.to_string(), cursor_rel, cursor - cursor_rel));
        }
        cursor_rel -= line_length + 1; // +1 to account for the newline character
    }
    return Err(ScratchError::new(StatusCode::EXPECTATION_FAILED, "incorrect cursor provided".to_string()));
}

async fn command_completion(
    query_line: &QueryLine,
    context: &AtCommandsContext,
    cursor_abs: i64,
    top_n: usize,
) -> (Vec<String>, bool, i64, i64) {    // returns ([possible, completions], good_as_it_is)
    let q_cmd = match query_line.command() {
        Some(x) => x,
        None => { return (vec![], false, -1, -1)}
    };

    let (_, cmd) = match context.at_commands.iter().find(|&(k, _v)| k == &q_cmd.value) {
        Some(x) => x,
        None => {
            return if !q_cmd.focused {
                (vec![], false, -1, -1)
            } else {
                (command_completion_options(&q_cmd.value, &context, top_n).await, false, q_cmd.pos1, q_cmd.pos2)
            }
        }
    };

    let can_execute = cmd.lock().await.can_execute(&query_line.get_args().iter().map(|x|x.value.clone()).collect(), context).await;

    for (arg, param) in query_line.get_args().iter().zip(cmd.lock().await.params()) {
        let param_locked = param.lock().await;
        let is_valid = param_locked.is_value_valid(&arg.value, context).await;
        if !is_valid {
            return if arg.focused {
                (param_locked.complete(&arg.value, context, top_n).await, can_execute, arg.pos1, arg.pos2)
            } else {
                (vec![], false, -1, -1)
            }
        }
        if is_valid && arg.focused && param_locked.complete_if_valid() {
            return (param_locked.complete(&arg.value, context, top_n).await, can_execute, arg.pos1, arg.pos2);
        }
    }

    if can_execute {
        return (vec![], true, -1, -1);
    }

    // if command is not focused, and the argument is empty we should make suggestions
    if !q_cmd.focused {
        match cmd.lock().await.params().get(query_line.get_args().len()) {
            Some(param) => {
                return (param.lock().await.complete(&"".to_string(), context, top_n).await, false, cursor_abs, cursor_abs);
            },
            None => {}
        }
    }

    (vec![], false, -1, -1)
}


async fn command_completion_options(
    q_cmd: &String,
    context: &AtCommandsContext,
    top_n: usize,
) -> Vec<String> {
    let at_commands_names = context.at_commands.iter().map(|(name, _cmd)| name.clone()).collect::<Vec<String>>();
    at_commands_names
        .iter()
        .filter(|command| command.starts_with(q_cmd))
        .map(|command| {
            (command, jaro_winkler(&command, q_cmd))
        })
        .sorted_by(|(_, dist1), (_, dist2)| dist1.partial_cmp(dist2).unwrap())
        .rev()
        .take(top_n)
        .map(|(command, _)| command.clone())
        .collect()
}
