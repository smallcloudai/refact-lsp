use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde_json::json;
use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;

use reqwest::Client;
use tracing::info;
use tokio::io;
use tokio::io::AsyncBufReadExt;

#[derive(Debug, serde::Deserialize)]
struct RHData {
    id: i64,
    tenant_name: String,
    ts_reported: i64,
    ip: String,
    enduser_client_version: String,
    completions_cnt: i64,
    file_extension: String,
    human_characters: i64,
    model: String,
    robot_characters: i64,
    teletype: String,
    ts_start: i64,
    ts_end: i64,
}

#[derive(Debug, serde::Deserialize)]
struct RHResponse {
    retcode: String,
    data: Vec<RHData>,
}

async fn fetch_data() -> Result<(), String> {
    let client = Client::new();
    let payload = json!({
        "key": "sMfJgiGm3gOH7gNeJ8qJM94Y"
    });
    let response = match client
        .post("https://staging.smallcloud.ai/v1/rh-stats")
        .header("X-Token", "q7iDnGVVe4R8Y0455c")
        .json(&payload)
        .send().await {
        Ok(response) => response,
        Err(e) => return Err(format!("Error fetching reports: {}", e)),
    };
    info!("{:?}", response.status());

    let body_mb = response.bytes().await;
    if body_mb.is_err() {
        return Err("Error fetching reports".to_string())
    }
    let body = body_mb.unwrap();
    let mut reader = io::BufReader::new(&body[..]);
    let mut line = String::new();

    while reader.read_line(&mut line).await.is_ok() {
        let response_data_mb: Result<RHResponse, _> =serde_json::from_str(&line);
        if response_data_mb.is_err() {
            info!("response_data_mb.is_err");
            break;
        }
        info!("{:#?}", response_data_mb.unwrap());
        line.clear();
    }
    Ok(())
}

pub async fn get_dashboard_records(
    Extension(global_context): Extension<SharedGlobalContext>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx_locked = global_context.read().await;

    let api_key = gcx_locked.cmdline.api_key.clone();

    let _ = fetch_data().await;
    // if let Err(e) = reports {
    //     return Err(ScratchError::new(StatusCode::NO_CONTENT, format!("Error fetching reports: {}", e)));
    // }
    // let reports = reports.unwrap();
    // info!("{:?}", reports);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": true}).to_string()))
        .unwrap())
}
