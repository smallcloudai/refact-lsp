use std::sync::Arc;

use async_trait::async_trait;
// use reqwest::header::AUTHORIZATION;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use serde_json::json;
use tokio::sync::Mutex as AMutex;

use crate::call_validation::{ChatMessage, ChatPost};
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
    let vdb_cont = vecdb_resp_to_prompt(&vdb_resp, limit_examples_cnt);
    if vdb_cont.len() > 0 {
        post.messages = [
            &post.messages[..post.messages.len() - 1],
            &[ChatMessage {
                role: "user".to_string(),
                content: vdb_cont,
            }],
            &post.messages[post.messages.len() - 1..],
        ].concat();
    }
}

// FIXME: move it to scratchpads section
fn vecdb_resp_to_prompt(
    resp: &Result<SearchResult, String>,
    limit_examples_cnt: usize,
) -> String {
    let mut cont = "".to_string();
    match resp {
        Ok(resp) => {
            cont.push_str("CONTEXT:\n");
            for i in 0..limit_examples_cnt {
                if i >= resp.results.len() {
                    break;
                }
                cont.push_str("FILENAME:\n");
                cont.push_str(resp.results[i].file_path.to_str().unwrap());
                cont.push_str("\nTEXT:");
                cont.push_str(resp.results[i].window_text.clone().as_str());
                cont.push_str("\n");
            }
            cont.push_str("\nRefer to the context to answer my next question.\n");
            cont
        }
        Err(e) => {
            format!("Vecdb error: {}", e);
            cont
        }
    }
}

#[derive(Debug)]
pub struct VecDbRemote {}

#[async_trait]
impl VecdbSearch for VecDbRemote {
    async fn search(
        &self,
        query: String,
        top_n: usize,
    ) -> Result<SearchResult, String> {
        let url = "http://127.0.0.1:8008/v1/vdb-search".to_string();
        let mut headers = HeaderMap::new();
        // headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", self.token)).unwrap());
        headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json").unwrap());
        let body = json!({
            "text": query,
            "top_n": top_n
        });
        let res = reqwest::Client::new()
            .post(&url)
            .headers(headers)
            .body(body.to_string())
            .send()
            .await.map_err(|e| format!("Vecdb search HTTP error (1): {}", e))?;

        let body = res.text().await.map_err(|e| format!("Vecdb search HTTP error (2): {}", e))?;
        // info!("Vecdb search result: {:?}", &body);
        let result: Vec<SearchResult> = serde_json::from_str(&body).map_err(|e| {
            format!("vecdb JSON problem: {}", e)
        })?;
        if result.len() == 0 {
            return Err("Vecdb search result is empty".to_string());
        }
        let result0 = result[0].clone();
        // info!("Vecdb search result: {:?}", &result0);
        Ok(result0)
    }
}
