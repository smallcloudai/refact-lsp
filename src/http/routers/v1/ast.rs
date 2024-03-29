use std::path::PathBuf;

use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use tree_sitter::Point;

use crate::custom_error::ScratchError;
use crate::files_in_workspace::Document;
use crate::global_context::SharedGlobalContext;

#[derive(Serialize, Deserialize, Clone)]
struct AstCursorSearchPost {
    filename: String,
    text: String,
    row: usize,
    column: usize,
    top_n: usize,
}

#[derive(Serialize, Deserialize, Clone)]
struct AstQuerySearchPost {
    query: String,
    top_n: usize,
}


#[derive(Serialize, Deserialize, Clone)]
struct AstReferencesPost {
    filename: String,
}


#[derive(Serialize, Deserialize, Clone)]
struct AstIndexFilePost {
    text: String,
    filename: String,
}


pub async fn handle_v1_ast_declarations_cursor_search(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<AstCursorSearchPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cx_locked = global_context.read().await;
    let search_res = match *cx_locked.ast_module.lock().await {
        Some(ref mut ast) => {
            let mut doc = Document::new(&PathBuf::from(post.filename), None);
            doc.update_text_from_disk().await;
            let text = match doc.get_text_or_read_from_disk().await {
                Ok(text) => text,
                Err(e) => return Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("Document has no text: {e}"))) 
            };
            ast.search_declarations_by_cursor(
                &doc, &text, Point::new(post.row, post.column), post.top_n, false
            ).await
        }
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Ast module is not available".to_string(),
            ));
        }
    };
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

pub async fn handle_v1_ast_declarations_query_search(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<AstQuerySearchPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cx_locked = global_context.read().await;
    let search_res = match *cx_locked.ast_module.lock().await {
        Some(ref ast) => {
            ast.search_declarations_by_symbol_path(
                post.query,
                post.top_n,
            ).await
        }
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Ast module is not available".to_string(),
            ));
        }
    };

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

pub async fn handle_v1_ast_references_cursor_search(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<AstCursorSearchPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cx_locked = global_context.read().await;
    let search_res = match *cx_locked.ast_module.lock().await {
        Some(ref mut ast) => {
            let mut doc = Document::new(&PathBuf::from(post.filename), None);
            doc.text = Some(Rope::from(post.text.clone()));
            if doc.text.is_none() {
                return Err(ScratchError::new(StatusCode::BAD_REQUEST, "Document has no text".to_string()));
            }
            ast.search_references_by_cursor(
                &doc.clone(), &doc.text.unwrap_or_default().to_string(), Point::new(post.row, post.column), post.top_n, true
            ).await
        }
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Ast module is not available".to_string(),
            ));
        }
    };
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

pub async fn handle_v1_ast_references_query_search(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<AstQuerySearchPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cx_locked = global_context.read().await;
    let search_res = match *cx_locked.ast_module.lock().await {
        Some(ref ast) => {
            ast.search_references_by_symbol_path(
                post.query,
                post.top_n,
            ).await
        }
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Ast module is not available".to_string(),
            ));
        }
    };

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

pub async fn handle_v1_ast_file_symbols(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<AstReferencesPost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    let doc = Document::new(&PathBuf::from(post.filename), None);

    let cx_locked = global_context.read().await;
    let search_res = match *cx_locked.ast_module.lock().await {
        Some(ref ast) => {
            ast.get_file_symbols(&doc).await
        }
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Ast module is not available".to_string(),
            ));
        }
    };

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

pub async fn handle_v1_ast_index_file(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<AstIndexFilePost>(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    
    let mut doc = Document::new(&PathBuf::from(post.filename), None);
    doc.text = Some(Rope::from(post.text));
    if doc.text.is_none() {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "Document has no text".to_string()));
    }
    
    let cx_locked = global_context.read().await;
    let add_res = match *cx_locked.ast_module.lock().await {
        Some(ref ast) => {
            ast.ast_add_file_no_queue(&doc).await
        }
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Ast module is not available".to_string(),
            ));
        }
    };

    match add_res {
        Ok(()) => {
            Ok(Response::builder().status(StatusCode::OK)
                .body(Body::from("{}"))
                .unwrap())
        }
        Err(e) => {
            Err(ScratchError::new(StatusCode::BAD_REQUEST, e))
        }
    }
}


pub async fn handle_v1_ast_clear_index(
    Extension(global_context): Extension<SharedGlobalContext>,
    _: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let cx_locked = global_context.read().await;
    let x = match *cx_locked.ast_module.lock().await {
        Some(ref ast) => {
            ast.clear_index().await;
            Ok(Response::builder().status(StatusCode::OK)
                .body(Body::from("{}"))
                .unwrap())
        }
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Ast module is not available".to_string(),
            ));
        }
    };
    x
}
