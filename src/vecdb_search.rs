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
