use tracing::{error, info};
use crate::call_validation::{ChatMessage};
use crate::vecdb_search::{VecdbResult};

use async_trait::async_trait;
use serde_json::json;


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


pub struct HasVecdbResults {
    pub was_sent: bool,
    pub in_json: serde_json::Value,
}

impl HasVecdbResults {
    pub fn new() -> Self {
        HasVecdbResults {
            was_sent: false,
            in_json: json!(null)
        }
    }
}

#[async_trait]
pub trait HasVecdb: Send {
    async fn add2messages(
        &mut self,
        vdb_result_mb: Result<VecdbResult, String>,
        messages: &mut Vec<ChatMessage>,
    );
    fn response_streaming(&mut self) -> Result<serde_json::Value, String>;
}

#[async_trait]
impl HasVecdb for HasVecdbResults {
    async fn add2messages(
        &mut self,
        vdb_result_mb: Result<VecdbResult, String>,
        messages: &mut Vec<ChatMessage>,
    ) {
        // info!("messages.len(): {}", messages.len());
        if messages.len() > 1 {
            return;
        }
        match vdb_result_mb {
            Ok(vdb_result) => {
                if vdb_result.results.len() > 0 {
                    *messages = [
                        &messages[..messages.len() -1],
                        &[ChatMessage {
                            role: "user".to_string(),
                            content: vecdb_resp_to_prompt(&vdb_result),
                        }],
                        &messages[messages.len() -1..],
                    ].concat();
                    self.in_json = json!(&vdb_result);
                }
            }
            Err(e) => { error!("Vecdb error: {}", e); }
        }
    }

    fn response_streaming(&mut self) -> Result<serde_json::Value, String> {
        if self.was_sent == true || self.in_json.is_null() {
            return Ok(json!(null));
        }
        self.was_sent = true;
        return Ok(json!({
            "choices": [{
                "delta": {
                    "content": self.in_json.clone(),
                    "role": "context"
                },
                "finish_reason": serde_json::Value::Null,
                "index": 0
            }],
        }));
    }
}
