use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;

#[derive(Serialize, Deserialize, Clone)]
struct AstPost {
    filename: String,
    row: usize,
    column: usize,
    top_n: usize,
}

pub async fn handle_v1_ast_search(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<AstPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cx_locked = global_context.read().await;
    let ast = cx_locked.ast_module.lock().await;
    // let search_res = ast.search(
    //     PathBuf::from(post.filename),
    //     Point::new(post.row, post.column),
    //     post.top_n,
    // ).await;
    let search_res: Result<Vec<usize>, String> = Ok(vec![]);
    match search_res {
        Ok(search_res) => {
            let json_string = serde_json::to_string_pretty(&search_res).map_err(|e| {
                ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON serialization problem: {}", e))
            })?;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(json_string))
                .unwrap())
        }
        Err(e) => {
            Err(ScratchError::new(StatusCode::BAD_REQUEST, e))
        }
    }
}
