use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;


// copy-paste from treesitter-vecdb-new
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextFile {
    pub file_name: String,
    pub file_content: String,
    pub line1: i32,
    pub line2: i32,
    #[serde(default)]
    pub usefulness: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}


pub async fn debug_fim_data(
    Extension(global_context): Extension<SharedGlobalContext>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    // let debug_data = global_context.read().await.debug_handler_data.lock().await.clone();

    let mut vector_of_context_file: Vec<ContextFile> = vec![];
    vector_of_context_file.push(ContextFile {
        file_name: "test.txt".to_string(),
        file_content: "test".to_string(),
        line1: 1,
        line2: 1,
        usefulness: 100.0,
    });
    vector_of_context_file.push(ContextFile {
        file_name: "test.py".to_string(),
        file_content: "test\ntest".to_string(),
        line1: 1,
        line2: 2,
        usefulness: 73.0,
    });
    vector_of_context_file.push(ContextFile {
        file_name: "test.js".to_string(),
        file_content: "test\ntest\ntest".to_string(),
        line1: 3,
        line2: 6,
        usefulness: 99.0,
    });

    let chat_message_mb = serde_json::to_string_pretty(&ChatMessage{
        role: "context_file".to_string(),
        content: json!(&vector_of_context_file).to_string(),
    });

    let body = match chat_message_mb{
        Ok(body) => body,
        Err(err) => {
            return Err(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY ,format!("Error serializing data: {}", err)))
        },
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(body))
        .unwrap())
}
