use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::call_validation::{ChatMessage, ChatPost, ContextFile};
use crate::vecdb::structs::{SearchResult, VecdbSearch};

pub async fn embed_vecdb_results<T>(
    vecdb_search: Arc<AMutex<Box<T>>>,
    post: &mut ChatPost,
    limit_examples_cnt: usize,
) where T: VecdbSearch {
    let my_vdb = vecdb_search.clone();
    let latest_msg_cont = &post.messages.last().unwrap().content;
    let vecdb_locked = my_vdb.lock().await;
    let vdb_resp = vecdb_locked.search(latest_msg_cont.clone(), limit_examples_cnt).await;
    let vdb_cont = vecdb_resp_to_prompt(&vdb_resp);
    if vdb_cont.is_ok() {
        post.messages = [
            &post.messages[..post.messages.len() - 1],
            &[ChatMessage {
                role: "context_file".to_string(),
                content: vdb_cont.unwrap()
            }],
            &post.messages[post.messages.len() - 1..],
        ].concat();
    }
}

fn vecdb_resp_to_prompt(
    resp: &Result<SearchResult, String>
) -> serde_json::Result<String> {
    let context_files: Vec<ContextFile> = match resp {
        Ok(search_res) => {
            search_res.results.iter().map(
                |x| ContextFile {
                    file_name: x.file_path.to_str().unwrap().to_string(),
                    file_content: x.window_text.clone(),
                }
            ).collect()

        }
        Err(_) => vec![]
    };
    serde_json::to_string(&context_files)
}
