use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify as ANotify;
use parking_lot::Mutex as ParkMutex;


#[derive(Serialize, Deserialize, Default)]
pub struct Chore {
    pub chore_id: String,
    pub chore_title: String,
    pub chore_spontaneous_work_enable: bool,
    pub chore_created_ts: f64,
    pub chore_archived_ts: f64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ChoreEvent {
    pub chore_event_id: String,
    pub chore_event_belongs_to_chore_id: String,
    pub chore_event_summary: String,
    pub chore_event_ts: f64,
    pub chore_event_link: String,
    pub chore_event_cthread_id: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct CThread {
    pub cthread_id: String,
    pub cthread_belongs_to_chore_event_id: Option<String>,
    pub cthread_title: String,
    pub cthread_toolset: String,      // quick/explore/agent
    pub cthread_model: String,
    pub cthread_temperature: f64,
    pub cthread_error: String,        // assign to special value "pause" to avoid auto repost to the model
    pub cthread_anything_new: bool,   // the ⚪
    pub cthread_created_ts: f64,
    pub cthread_updated_ts: f64,
    pub cthread_archived_ts: f64,     // associated container died, cannot continue
    pub cthread_locked_by: String,
    pub cthread_locked_ts: f64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct CMessage {
    // primary key starts here
    pub cmessage_belongs_to_cthread_id: String,
    pub cmessage_alt: i32,
    pub cmessage_num: i32,
    // /primary
    pub cmessage_prev_alt: i32,
    pub cmessage_usage_model: String,
    pub cmessage_usage_prompt: String,
    pub cmessage_usage_completion: String,
    pub cmessage_json: String,
}

// db_v1/cthread_sub     { quicksearch, limit } -> SSE
// db_v1/cthread_update  { Option<cthread_id>, fields } -> cthread_id (and SSE in other channel)
// db_v1/cthread_delete  { cthread_id } -> ok or detail
// db_v1/cmessages_sub     { cthread_id } -> SSE
// db_v1/cmessage_update  { cthread_id, n_onwards } -> ok or detail


pub struct ChoreDB {
    pub lite: Arc<ParkMutex<rusqlite::Connection>>,
    pub chore_sleeping_point: Arc<ANotify>,
}
