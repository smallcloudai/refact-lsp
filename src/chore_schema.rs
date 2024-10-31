use std::sync::Arc;
use serde::{Deserialize, Serialize};
use parking_lot::Mutex as ParkMutex;

use crate::call_validation::ChatMessage;


#[derive(Debug, Serialize, Deserialize)]
pub struct Chore {
    pub chore_id: String,
    pub chore_title: String,
    pub chore_spontaneous_work_enable: bool,
    pub chore_event_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChoreEvent {
    pub chore_event_id: String,
    pub chore_event_summary: String,
    pub chore_event_ts: f64,
    pub chore_event_link: String,
    pub chore_event_cthread_id: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ChatThread {
    pub cthread_id: String,
    pub cthread_belongs_to_chore_event_id: Option<String>,
    pub cthread_title: String,
    pub cthread_toolset: String,      // quick/explore/agent
    pub cthread_model_used: String,
    pub cthread_error: String,        // assign to special value "pause" to avoid auto repost to the model
    pub cthread_anything_new: bool,   // the ⚪
    pub cthread_created_ts: f64,
    pub cthread_updated_ts: f64,
    pub cthread_archived_ts: f64,     // associated container died, cannot continue
}

// db_v1/cthread_sub     { quicksearch, limit } -> SSE
// db_v1/cthread_update  { Option<cthread_id>, fields } -> cthread_id (and SSE in other channel)
// db_v1/cthread_delete  { cthread_id } -> ok or detail
// db_v1/cmessages_sub     { cthread_id } -> SSE
// db_v1/cmessages_update  { cthread_id, n_onwards } -> ok or detail


pub struct ChoreDB {
    pub lite: Arc<ParkMutex<rusqlite::Connection>>,
}
