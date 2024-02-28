use std::io::Write;
use std::sync::Arc;
use std::net::IpAddr;
use std::str::FromStr;

use axum::{Extension, http::{StatusCode, Uri}, response::IntoResponse};
use hyper::Server;
use tokio::sync::RwLock as ARwLock;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::global_context::GlobalContext;
use crate::http::routers::make_refact_http_server;

pub mod routers;
mod utils;

async fn handler_404(path: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("no handler for {}", path))
}


pub async fn start_server(
    global_context: Arc<ARwLock<GlobalContext>>,
    ask_shutdown_receiver: std::sync::mpsc::Receiver<String>,
) -> Option<JoinHandle<()>> {
    let (host, port) = {
        let read_guard = global_context.read().await;
        let host = read_guard.cmdline.http_host.clone();
        let port = read_guard.cmdline.http_port;
        (host, port)
    };
    if port == 0 {
        return None;
    }
    let ip = IpAddr::from_str(&host).expect("Invalid IP address");
    return Some(tokio::spawn(async move {
        let addr = (ip, port).into();
        let builder = Server::try_bind(&addr).map_err(|e| {
            write!(std::io::stderr(), "PORT_BUSY {}\n", e).unwrap();
            std::io::stderr().flush().unwrap();
            format!("port busy, address {}: {}", addr, e)
        });
        match builder {
            Ok(builder) => {
                info!("HTTP server listening on {}", addr);
                let router = make_refact_http_server().layer(Extension(global_context.clone()));
                let server = builder
                    .serve(router.into_make_service())
                    .with_graceful_shutdown(crate::global_context::block_until_signal(ask_shutdown_receiver));
                let resp = server.await.map_err(|e| format!("HTTP server error: {}", e));
                if let Err(e) = resp {
                    error!("server error: {}", e);
                } else {
                    info!("clean shutdown");
                }
            }
            Err(e) => {
                error!("server error: {}", e);
            }
        }
    }));
}
