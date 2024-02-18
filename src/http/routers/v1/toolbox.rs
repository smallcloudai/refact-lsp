use axum::response::Result;
use axum::Extension;
use hyper::{Body, Response, StatusCode};

use std::sync::Arc;

use tokio::sync::RwLock as ARwLock;


use crate::at_commands::at_commands::AtCommandsContext;


use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;


pub async fn handle_v1_toolbox_config(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    _body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let _context = AtCommandsContext::new(global_context.clone()).await;

    let tconfig = crate::toolbox::toolbox_config::load_and_mix_with_users_config();



    // let response = CommandCompletionResponse {
    //     completions: completions.clone(),
    //     replace: (pos1, pos2),
    //     is_cmd_executable,
    // };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string_pretty(&tconfig).unwrap()))
        .unwrap())
}
