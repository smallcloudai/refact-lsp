use std::error::Error;
use tracing::error;
use hyper::{Body, Response, StatusCode};
use serde_json::json;
use std::fmt;
use axum::Json;
use axum::response::IntoResponse;
use crate::global_context::SharedGlobalContext;
use crate::telemetry_basic;


#[derive(Debug, Clone)]
pub struct ScratchError {
    pub status_code: StatusCode,
    pub message: String,
    pub telemetry_skip: bool,    // because already posted a better description directly
}

impl IntoResponse for ScratchError {
    fn into_response(self) -> axum::response::Response {
        let payload = json!({
            "detail": self.message,
        });
        let status_code = self.clone().status_code;
        // tokio::spawn(async move {
        //     let e = self;
        //     if !e.telemetry_skip {
        //         let tele_storage = &e.global_context.read().await.telemetry;
        //         let mut tele_storage_locked = tele_storage.write().unwrap();
        //         tele_storage_locked.tele_net.push(telemetry_basic::TelemetryNetwork::new(
        //             e.path.clone(),
        //             format!("{}", e.method),
        //             false,
        //             format!("{}", e.message),
        //         ));
        //     }
        //     return Ok(e.to_response());
        // });

        (status_code, Json(payload)).into_response()
    }
}

impl std::error::Error for ScratchError {}
unsafe impl Send for ScratchError {}

unsafe impl Sync for ScratchError {}
impl fmt::Display for ScratchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.status_code, self.message)
    }
}

impl ScratchError {
    pub fn new(status_code: StatusCode, message: String) -> Self {
        ScratchError {
            status_code,
            message,
            telemetry_skip: false,
        }
    }

    pub fn new_but_skip_telemetry(status_code: StatusCode, message: String) -> Self {
        ScratchError {
            status_code,
            message,
            telemetry_skip: true,
        }
    }

    pub fn to_response(&self) -> Response<Body> {
        let body = json!({"detail": self.message}).to_string();
        error!("client will see {}", body);
        let response = Response::builder()
            .status(self.status_code)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        response
    }
}
