use std::pin::Pin;

use axum::Extension;
use axum::Router;
use axum::routing::get;
use axum::routing::post;
use futures::Future;
use hyper::Body;
use hyper::Response;

use crate::{telemetry_get, telemetry_post};
use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::http::routers::v1::ast::{handle_v1_ast_clear_index, handle_v1_ast_file_markup, handle_v1_ast_file_dump, handle_v1_ast_file_symbols, handle_v1_ast_index_file, handle_v1_ast_search_by_content, handle_v1_ast_search_by_name, handle_v1_ast_search_related_declarations, handle_v1_ast_search_usages_by_declarations, handle_v1_ast_force_reindex, handle_v1_ast_status};
use crate::http::routers::v1::at_commands::{handle_v1_command_completion, handle_v1_command_preview};
use crate::http::routers::v1::at_tools::handle_v1_tools_available;
use crate::http::routers::v1::caps::handle_v1_caps;
use crate::http::routers::v1::caps::handle_v1_ping;
use crate::http::routers::v1::chat::{handle_v1_chat, handle_v1_chat_completions};
use crate::http::routers::v1::code_completion::{handle_v1_code_completion_web, handle_v1_code_completion_prompt};
use crate::http::routers::v1::dashboard::get_dashboard_plots;
use crate::http::routers::v1::graceful_shutdown::handle_v1_graceful_shutdown;
use crate::http::routers::v1::snippet_accepted::handle_v1_snippet_accepted;
use crate::http::routers::v1::telemetry_network::handle_v1_telemetry_network;
use crate::http::routers::v1::lsp_like_handlers::{handle_v1_lsp_did_change, handle_v1_lsp_add_folder, handle_v1_lsp_initialize, handle_v1_lsp_remove_folder};
use crate::http::routers::v1::status::handle_v1_rag_status;
use crate::http::routers::v1::toolbox::handle_v1_customization;
use crate::http::routers::v1::toolbox::handle_v1_rewrite_assistant_says_to_at_commands;
use crate::http::routers::v1::vecdb::{handle_v1_vecdb_search, handle_v1_vecdb_status};
use crate::http::utils::telemetry_wrapper;

pub mod code_completion;
pub mod chat;
pub mod telemetry_network;
pub mod snippet_accepted;
pub mod caps;
pub mod graceful_shutdown;
mod dashboard;
pub mod lsp_like_handlers;
pub mod toolbox;
pub mod vecdb;
mod at_commands;
mod ast;
mod at_tools;
mod status;

pub fn make_v1_router() -> Router {
    Router::new()
        .route("/ping", telemetry_get!(handle_v1_ping))

        .route("/code-completion", telemetry_post!(handle_v1_code_completion_web))
        .route("/chat", telemetry_post!(handle_v1_chat))
        .route("/chat/completions", telemetry_post!(handle_v1_chat_completions))  // standard
        .route("/telemetry-network", telemetry_post!(handle_v1_telemetry_network))
        .route("/snippet-accepted", telemetry_post!(handle_v1_snippet_accepted))

        .route("/caps", telemetry_get!(handle_v1_caps))
        .route("/graceful-shutdown", telemetry_get!(handle_v1_graceful_shutdown))

        .route("/vdb-search", telemetry_post!(handle_v1_vecdb_search))
        .route("/vdb-status", telemetry_get!(handle_v1_vecdb_status))
        .route("/at-command-completion", telemetry_post!(handle_v1_command_completion))
        .route("/at-command-preview", telemetry_post!(handle_v1_command_preview))

        .route("/tools", telemetry_get!(handle_v1_tools_available))

        .route("/lsp-initialize", telemetry_post!(handle_v1_lsp_initialize))
        .route("/lsp-did-changed", telemetry_post!(handle_v1_lsp_did_change))
        .route("/lsp-add-folder", telemetry_post!(handle_v1_lsp_add_folder))
        .route("/lsp-remove-folder", telemetry_post!(handle_v1_lsp_remove_folder))

        .route("/get-dashboard-plots", telemetry_get!(get_dashboard_plots))

        .route("/ast-search-by-name", telemetry_post!(handle_v1_ast_search_by_name))
        .route("/ast-search-by-content", telemetry_post!(handle_v1_ast_search_by_content))
        .route("/ast-search-related-declarations", telemetry_post!(handle_v1_ast_search_related_declarations))
        .route("/ast-search-usages-by-declarations", telemetry_post!(handle_v1_ast_search_usages_by_declarations))
        .route("/ast-file-markup", telemetry_post!(handle_v1_ast_file_markup))
        .route("/ast-file-dump", telemetry_post!(handle_v1_ast_file_dump))
        .route("/ast-file-symbols", telemetry_post!(handle_v1_ast_file_symbols))
        .route("/ast-index-file", telemetry_post!(handle_v1_ast_index_file))
        .route("/ast-force-reindex", telemetry_get!(handle_v1_ast_force_reindex))
        .route("/ast-clear-index", telemetry_get!(handle_v1_ast_clear_index))
        .route("/ast-status", telemetry_get!(handle_v1_ast_status))

        .route("/rag-status", telemetry_get!(handle_v1_rag_status))
        // experimental
        .route("/customization", telemetry_get!(handle_v1_customization))
        .route("/rewrite-assistant-says-to-at-commands", telemetry_post!(handle_v1_rewrite_assistant_says_to_at_commands))

        .route("/code-completion-prompt", telemetry_post!(handle_v1_code_completion_prompt))
}
