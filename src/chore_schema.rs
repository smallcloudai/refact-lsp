use std::sync::Arc;
use serde::{Deserialize, Serialize};

use crate::call_validation::ChatMessage;


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Chore {
    pub chore_id: String,
    pub chore_title: String,
    pub chore_spontaneous_work_enable: bool,
    pub chore_event_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChoreEvent {
    pub chore_event_id: String,
    pub chore_event_summary: String,
    pub chore_event_ts: f64,
    pub chore_event_link: String,
    pub chore_event_cthread_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatThread {
    pub cthread_id: String,
    #[serde(default)]
    pub cthread_messages: Vec<ChatMessage>,
    pub cthread_title: String,
    pub cthread_toolset: String,      // quick/explore/agent
    pub cthread_model_used: String,
    pub cthread_error: String,        // assign to special value "pause" to avoid auto repost to the model
    pub cthread_anything_new: bool,   // the ⚪
    pub cthread_created_ts: f64,
    pub cthread_updated_ts: f64,
    pub cthread_archived_ts: f64,     // associated container died, cannot continue
}

pub struct ChoreDB {
    pub sleddb: Arc<sled::Db>,
}
