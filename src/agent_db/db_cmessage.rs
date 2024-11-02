use std::sync::Arc;
use parking_lot::Mutex as ParkMutex;
use rusqlite::params;
use crate::agent_db::db_structs::ChoreDB;
use crate::agent_db::db_structs::CMessage;


pub fn cmessages_from_rows(
    mut rows: rusqlite::Rows,
) -> Vec<CMessage> {
    let mut cmessages = Vec::new();
    while let Some(row) = rows.next().unwrap_or(None) {
        cmessages.push(CMessage {
            cmessage_belongs_to_cthread_id: row.get("cmessage_belongs_to_cthread_id").unwrap(),
            cmessage_alt: row.get("cmessage_alt").unwrap(),
            cmessage_num: row.get("cmessage_num").unwrap(),
            cmessage_prev_alt: row.get("cmessage_prev_alt").unwrap(),
            cmessage_usage_model: row.get("cmessage_usage_model").unwrap(),
            cmessage_usage_prompt: row.get("cmessage_usage_prompt").unwrap(),
            cmessage_usage_completion: row.get("cmessage_usage_completion").unwrap(),
            cmessage_json: row.get("cmessage_json").unwrap(),
        });
    }
    cmessages
}

pub fn cmessage_set(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cmessage: CMessage,
) {
    let db = cdb.lock();
    let conn = db.lite.lock();
    conn.execute(
        "INSERT OR REPLACE INTO cmessage (
            cmessage_belongs_to_cthread_id,
            cmessage_alt,
            cmessage_num,
            cmessage_prev_alt,
            cmessage_usage_model,
            cmessage_usage_prompt,
            cmessage_usage_completion,
            cmessage_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            cmessage.cmessage_belongs_to_cthread_id,
            cmessage.cmessage_alt,
            cmessage.cmessage_num,
            cmessage.cmessage_prev_alt,
            cmessage.cmessage_usage_model,
            cmessage.cmessage_usage_prompt,
            cmessage.cmessage_usage_completion,
            cmessage.cmessage_json,
        ],
    ).expect("Failed to insert or replace chat message");
}

pub fn cmessage_get(
    cdb: Arc<ParkMutex<ChoreDB>>,
    cmessage_belongs_to_cthread_id: String,
    cmessage_alt: i32,
    cmessage_num: i32,
) -> Result<CMessage, String> {
    let db = cdb.lock();
    let conn = db.lite.lock();
    let mut stmt = conn.prepare(
        "SELECT * FROM cmessage WHERE cmessage_belongs_to_cthread_id = ?1 AND cmessage_alt = ?2 AND cmessage_num = ?3"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query(params![cmessage_belongs_to_cthread_id, cmessage_alt, cmessage_num])
        .map_err(|e| e.to_string())?;
    let cmessages = cmessages_from_rows(rows);
    cmessages.into_iter().next()
        .ok_or_else(|| format!("No CMessage found with {}:{}:{}", cmessage_belongs_to_cthread_id, cmessage_alt, cmessage_num))
}