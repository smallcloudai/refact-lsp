use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::Hasher;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::sync::RwLock as StdRwLock;

use hyper::StatusCode;
use structopt::StructOpt;
use tokenizers::Tokenizer;
use tokio::signal;
use tokio::sync::{Mutex as AMutex, Semaphore};
use tokio::sync::RwLock as ARwLock;
use tracing::{error, info};

use crate::ast::ast_module::AstModule;
use crate::caps::CodeAssistantCaps;
use crate::completion_cache::CompletionCache;
use crate::custom_error::ScratchError;
use crate::files_in_workspace::DocumentsState;
use crate::telemetry::telemetry_structs;
use crate::vecdb::vecdb::VecDb;

#[derive(Debug, StructOpt, Clone)]
pub struct CommandLine {
    #[structopt(long, default_value="pong", help="A message to return in /v1/ping, useful to verify you're talking to the same process that you've started.")]
    pub ping_message: String,
    #[structopt(long, help="Send logs to stderr, as opposed to ~/.cache/refact/logs, so it's easier to debug.")]
    pub logs_stderr: bool,
    #[structopt(long, short="u", help="URL to start working. The first step is to fetch refact-caps / coding_assistant_caps.json.")]
    pub address_url: String,
    #[structopt(long, short="k", default_value="", help="The API key to authenticate your requests, will appear in HTTP requests this binary makes.")]
    pub api_key: String,
    #[structopt(long, short="p", default_value="0", help="Bind 127.0.0.1:<port> to listen for HTTP requests, such as /v1/code-completion, /v1/chat, /v1/caps.")]
    pub http_port: u16,
    #[structopt(long, default_value="", help="End-user client version, such as version of VS Code plugin.")]
    pub enduser_client_version: String,
    #[structopt(long, short="b", help="Send basic telemetry (counters and errors)")]
    pub basic_telemetry: bool,
    #[structopt(long, short="s", help="Send snippet telemetry (code snippets)")]
    pub snippet_telemetry: bool,
    #[structopt(long, default_value="0", help="Bind 127.0.0.1:<port> and act as an LSP server. This is compatible with having an HTTP server at the same time.")]
    pub lsp_port: u16,
    #[structopt(long, default_value="0", help="Act as an LSP server, use stdin stdout for communication. This is compatible with having an HTTP server at the same time. But it's not compatible with LSP port.")]
    pub lsp_stdin_stdout: u16,
    #[structopt(long, help="Trust self-signed SSL certificates")]
    pub insecure: bool,
    #[structopt(long, short="v", help="Verbose logging, lots of output")]
    pub verbose: bool,
    #[structopt(long, help="Use AST. For it to start working, give it a jsonl files list or LSP workspace folders.")]
    pub ast: bool,
    #[structopt(long, help="Use AST light mode. Could be useful for large projects and weak systems. In this mode we don't parse variables")]
    pub ast_light_mode: bool,
    #[structopt(long, default_value="15000", help="Maximum files for AST index, to avoid OOM on large projects.")]
    pub ast_max_files: usize,
    #[structopt(long, help="Use vector database. Give it a jsonl files list or LSP workspace folders, and also caps need to have an embedding model.")]
    pub vecdb: bool,
    #[structopt(long, default_value="15000", help="Maximum files count for VecDB index, to avoid OOM.")]
    pub vecdb_max_files: usize,
    #[structopt(long, short="f", default_value="", help="A path to jsonl file with {\"path\": ...} on each line, files will immediately go to vecdb and ast")]
    pub files_jsonl_path: String,
    #[structopt(long, default_value="", help="Vecdb storage path")]
    pub vecdb_forced_path: String,
    #[structopt(long, short="w", default_value="", help="Workspace folder to find files for vecdb and AST. An LSP or HTTP request can override this later.")]
    pub workspace_folder: String,
}
impl CommandLine {
    fn create_hash(msg: String) -> String {
        let mut hasher = DefaultHasher::new();
        hasher.write(msg.as_bytes());
        format!("{:x}", hasher.finish())
    }
    pub fn get_prefix(&self) -> String {
        Self::create_hash(format!("{}:{}", self.address_url.clone(), self.api_key.clone()))[..6].to_string()
    }
}

pub struct GlobalContext {
    pub cmdline: CommandLine,
    pub http_client: reqwest::Client,
    pub http_client_slowdown: Arc<Semaphore>,
    pub cache_dir: PathBuf,
    pub caps: Option<Arc<StdRwLock<CodeAssistantCaps>>>,
    pub caps_reading_lock: Arc<AMutex<bool>>,
    pub caps_last_error: String,
    pub caps_last_attempted_ts: u64,
    pub tokenizer_map: HashMap< String, Arc<StdRwLock<Tokenizer>>>,
    pub tokenizer_download_lock: Arc<AMutex<bool>>,
    pub completions_cache: Arc<StdRwLock<CompletionCache>>,
    pub telemetry: Arc<StdRwLock<telemetry_structs::Storage>>,
    pub vec_db: Arc<AMutex<Option<VecDb>>>,
    pub ast_module: Option<Arc<ARwLock<AstModule>>>,
    pub vec_db_error: String,
    pub ask_shutdown_sender: Arc<StdMutex<std::sync::mpsc::Sender<String>>>,
    pub documents_state: DocumentsState,
}

pub type SharedGlobalContext = Arc<ARwLock<GlobalContext>>;  // TODO: remove this type alias, confusing

const CAPS_RELOAD_BACKOFF: u64 = 60;       // seconds
const CAPS_BACKGROUND_RELOAD: u64 = 3600;  // seconds

pub async fn try_load_caps_quickly_if_not_present(
    global_context: Arc<ARwLock<GlobalContext>>,
    max_age_seconds: u64,
) -> Result<Arc<StdRwLock<CodeAssistantCaps>>, ScratchError> {
    let caps_reading_lock: Arc<AMutex<bool>> = global_context.read().await.caps_reading_lock.clone();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let caps_last_attempted_ts;
    {
        // global_context is not locked, but a specialized async mutex is, up until caps are saved
        let _caps_reading_locked = caps_reading_lock.lock().await;
        let max_age = if max_age_seconds > 0 { max_age_seconds } else { CAPS_BACKGROUND_RELOAD };
        {
            let mut cx_locked = global_context.write().await;
            if cx_locked.caps_last_attempted_ts + max_age < now {
                cx_locked.caps = None;
                cx_locked.caps_last_attempted_ts = 0;
                caps_last_attempted_ts = 0;
            } else {
                if let Some(caps_arc) = cx_locked.caps.clone() {
                    return Ok(caps_arc.clone());
                }
                caps_last_attempted_ts = cx_locked.caps_last_attempted_ts;
            }
        }
        if caps_last_attempted_ts + CAPS_RELOAD_BACKOFF > now {
            let global_context_locked = global_context.write().await;
            return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, global_context_locked.caps_last_error.clone()));
        }
        let caps_result = crate::caps::load_caps(
            CommandLine::from_args(),
            global_context.clone()
        ).await;
        {
            let mut global_context_locked = global_context.write().await;
            global_context_locked.caps_last_attempted_ts = now;
            match caps_result {
                Ok(caps) => {
                    global_context_locked.caps = Some(caps.clone());
                    global_context_locked.caps_last_error = "".to_string();
                    info!("quick load caps successful");
                    let _ = write!(std::io::stderr(), "CAPS\n");
                    Ok(caps)
                },
                Err(e) => {
                    error!("caps fetch failed: \"{}\"", e);
                    global_context_locked.caps_last_error = format!("caps fetch failed: {}", e);
                    return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, global_context_locked.caps_last_error.clone()));
                }
            }
        }
    }
}

pub async fn look_for_piggyback_fields(
    global_context: Arc<ARwLock<GlobalContext>>,
    anything_from_server: &serde_json::Value)
{
    let mut global_context_locked = global_context.write().await;
    if let Some(dict) = anything_from_server.as_object() {
        let new_caps_version = dict.get("caps_version").and_then(|v| v.as_i64()).unwrap_or(0);
        if new_caps_version > 0 {
            if let Some(caps) = global_context_locked.caps.clone() {
                let caps_locked = caps.read().unwrap();
                if caps_locked.caps_version < new_caps_version {
                    info!("detected biggyback caps version {} is newer than the current version {}", new_caps_version, caps_locked.caps_version);
                    global_context_locked.caps = None;
                    global_context_locked.caps_last_attempted_ts = 0;
                }
            }
        }
    }
}

pub async fn block_until_signal(
    ask_shutdown_receiver: std::sync::mpsc::Receiver<String>,
    shutdown_flag: Arc<AtomicBool>
) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let sigterm = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    #[cfg(unix)]
    let sigusr1 = async {
        signal::unix::signal(signal::unix::SignalKind::user_defined1())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let sigusr1 = std::future::pending::<()>();

    let shutdown_flag_clone = shutdown_flag.clone();
    tokio::select! {
        _ = ctrl_c => {
            info!("SIGINT signal received");
            shutdown_flag_clone.store(true, Ordering::SeqCst);
        },
        _ = sigterm => {
            info!("SIGTERM signal received");
            shutdown_flag_clone.store(true, Ordering::SeqCst);
        },
        _ = sigusr1 => {
            info!("SIGUSR1 signal received");
        },
        _ = tokio::task::spawn_blocking(move || {
            let _ = ask_shutdown_receiver.recv();
            shutdown_flag.store(true, Ordering::SeqCst);
        }) => {
            info!("graceful shutdown to store telemetry");
        }
    }
}

pub async fn create_global_context(
    cache_dir: PathBuf,
) -> (Arc<ARwLock<GlobalContext>>, std::sync::mpsc::Receiver<String>, Arc<AtomicBool>, CommandLine) {
    let cmdline = CommandLine::from_args();
    let (ask_shutdown_sender, ask_shutdown_receiver) = std::sync::mpsc::channel::<String>();
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let mut http_client_builder = reqwest::Client::builder();
    if cmdline.insecure {
        http_client_builder = http_client_builder.danger_accept_invalid_certs(true)
    }
    let http_client = http_client_builder.build().unwrap();

    let mut workspace_dirs: Vec<PathBuf> = vec![];
    if !cmdline.workspace_folder.is_empty() {
        let path = crate::files_correction::canonical_path(&cmdline.workspace_folder);
        workspace_dirs = vec![path];
    }
    let cx = GlobalContext {
        cmdline: cmdline.clone(),
        http_client,
        http_client_slowdown: Arc::new(Semaphore::new(2)),
        cache_dir,
        caps: None,
        caps_reading_lock: Arc::new(AMutex::<bool>::new(false)),
        caps_last_error: String::new(),
        caps_last_attempted_ts: 0,
        tokenizer_map: HashMap::new(),
        tokenizer_download_lock: Arc::new(AMutex::<bool>::new(false)),
        completions_cache: Arc::new(StdRwLock::new(CompletionCache::new())),
        telemetry: Arc::new(StdRwLock::new(telemetry_structs::Storage::new())),
        vec_db: Arc::new(AMutex::new(None)),
        ast_module: None,
        vec_db_error: String::new(),
        ask_shutdown_sender: Arc::new(StdMutex::new(ask_shutdown_sender)),
        documents_state: DocumentsState::new(workspace_dirs).await,
    };
    let gcx = Arc::new(ARwLock::new(cx));
    if cmdline.ast {
        let ast_module = Arc::new(ARwLock::new(
            AstModule::ast_indexer_init(
                cmdline.ast_max_files,
                shutdown_flag.clone(),
                cmdline.ast_light_mode
            ).await.expect("Failed to initialize ast module")
        ));
        gcx.write().await.ast_module = Some(ast_module);
    }
    {
        let gcx_weak = Arc::downgrade(&gcx);
        gcx.write().await.documents_state.init_watcher(gcx_weak, tokio::runtime::Handle::current());
    }
    (gcx, ask_shutdown_receiver, shutdown_flag, cmdline)
}
