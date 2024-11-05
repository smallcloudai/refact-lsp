use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use tokio::sync::RwLock as ARwLock;
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::{integrations_paths, load_integration_schema_and_json, validate_integration_value};


#[derive(Serialize, Deserialize)]
struct IntegrationItem {
    name: String,
    schema: Option<Value>,
    value: Value,
}

pub async fn handle_v1_integrations(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {

    let schemas_and_json_dict = load_integration_schema_and_json(gcx).await.map_err(|e|{
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load integrations: {}", e))
    })?;
    
    let mut items = vec![];
    for (name, (schema, value)) in schemas_and_json_dict {
        let item = IntegrationItem {
            name,
            schema: Some(schema),
            value,
        };
        
        items.push(item);
    }
    
    let payload = serde_json::to_string_pretty(&json!(items)).expect("Failed to serialize items");
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}


pub async fn handle_v1_integrations_save(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<IntegrationItem>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let yaml_value: serde_yaml::Value = serde_json::to_string(&post.value).map_err(|e|e.to_string())
        .and_then(|s|serde_yaml::from_str(&s).map_err(|e|e.to_string()))
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("ERROR converting JSON to YAML: {}", e)))?;

    let yaml_value = validate_integration_value(&post.name, yaml_value)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("ERROR validating integration value: {}", e)))?;
    
    let integr_paths = integrations_paths(gcx.clone()).await;
    
    let path = integr_paths.get(&post.name)
        .ok_or(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("Integration {} not found", post.name)))?;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file: {}", e)))?;

    let yaml_string = serde_yaml::to_string(&yaml_value)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to convert YAML to string: {}", e)))?;

    tokio::io::AsyncWriteExt::write_all(&mut file, yaml_string.as_bytes()).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write to file: {}", e)))?;

    Ok(Response::builder()
       .status(StatusCode::OK)
       .header("Content-Type", "application/json")
       .body(Body::from(format!("Integration {} saved", post.name)))
       .unwrap())
}
