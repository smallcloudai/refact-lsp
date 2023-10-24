use crate::call_validation::{ChatMessage};
// use reqwest::header::AUTHORIZATION;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use serde_json::json;

use async_trait::async_trait;


#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VecdbResultRec {
    pub file_name: String,
    pub text: String,
    pub score: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VecdbResult {
    pub results: Vec<VecdbResultRec>,
}

pub async fn add_vecdb2messages(
    vdb_result: &VecdbResult,
    messages: &mut Vec<ChatMessage>,
) {
    if vdb_result.results.len() > 0 {
        *messages = [
            &messages[..messages.len() -1],
            &[ChatMessage {
                role: "user".to_string(),
                content: vecdb_resp_to_prompt(vdb_result),
            }],
            &messages[messages.len() -1..],
        ].concat();
    }
}


fn vecdb_resp_to_prompt(
    vdb_result: &VecdbResult,
) -> String {
    let mut cont = "".to_string();
    cont.push_str("CONTEXT:\n");
    for r in vdb_result.results.iter() {

        cont.push_str("FILENAME:\n");
        cont.push_str(r.file_name.clone().as_str());
        cont.push_str("\nTEXT:");
        cont.push_str(r.text.clone().as_str());
        cont.push_str("\n");
    }
    cont.push_str("\nRefer to the context to answer my next question.\n");
    cont
}

#[async_trait]
pub trait VecdbSearch: Send {
    async fn search(
        &mut self,
        query: &str,
    ) -> Result<VecdbResult, String>;
}

#[derive(Debug, Clone)]
pub struct VecdbSearchTest {
}

impl VecdbSearchTest {
    pub fn new() -> Self {
        VecdbSearchTest {
        }
    }
}

// unsafe impl Send for VecdbSearchTest {}

#[async_trait]
impl VecdbSearch for VecdbSearchTest {
    async fn search(
        &mut self,
        query: &str,
    ) -> Result<VecdbResult, String> {
        let url = "http://127.0.0.1:8008/v1/vdb-search".to_string();
        let mut headers = HeaderMap::new();
        // headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", self.token)).unwrap());
        headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json").unwrap());
        let body = json!({
            "texts": [query],
            "account": "XXX",
            "top_k": 3,
        });
        let res = reqwest::Client::new()
            .post(&url)
            .headers(headers)
            .body(body.to_string())
            .send()
            .await.map_err(|e| format!("Vecdb search HTTP error (1): {}", e))?;

        let body = res.text().await.map_err(|e| format!("Vecdb search HTTP error (2): {}", e))?;
        // info!("Vecdb search result: {:?}", &body);
        let result: Vec<VecdbResult> = serde_json::from_str(&body).map_err(|e| {
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
