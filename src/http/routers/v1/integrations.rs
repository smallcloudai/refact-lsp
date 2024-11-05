use std::sync::Arc;
use axum::Extension;
use axum::http::Response;
use tokio::sync::RwLock as ARwLock;
use hyper::Body;
use serde::Serialize;
use serde_json::{json, Value};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::load_integration_schema_and_json;


#[derive(Serialize)]
struct IntegrationItem {
    name: String,
    schema: Value,
    value: Value,
}

pub async fn handle_v1_integrations(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {

    let schemas_and_json_dict = load_integration_schema_and_json(gcx.clone()).await;
    
    let mut items = vec![];
    for (name, (schema, value)) in schemas_and_json_dict {
        let item = IntegrationItem {
            name,
            schema,
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
