use rusqlite::Connection;


pub fn create_tables_20241102(conn: &Connection, reset_memory: bool) -> Result<(), String> {
    if reset_memory {
        conn.execute("DROP TABLE IF EXISTS pubsub_events", []).map_err(|e| e.to_string())?;
        conn.execute("DROP TABLE IF EXISTS cthreads", []).map_err(|e| e.to_string())?;
        conn.execute("DROP TABLE IF EXISTS cmessages", []).map_err(|e| e.to_string())?;
    }
    conn.execute(
        "CREATE TABLE IF NOT EXISTS pubsub_events (
            pubevent_id INTEGER PRIMARY KEY AUTOINCREMENT,
            pubevent_channel TEXT NOT NULL,
            pubevent_action TEXT NOT NULL,
            pubevent_json TEXT NOT NULL
        )",
        [],
    ).map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS cthreads (
            cthread_id TEXT PRIMARY KEY,
            cthread_belongs_to_chore_event_id TEXT DEFAULT NULL,
            cthread_title TEXT NOT NULL,
            cthread_toolset TEXT NOT NULL,
            cthread_model_used TEXT NOT NULL,
            cthread_error TEXT NOT NULL,
            cthread_anything_new BOOLEAN NOT NULL,
            cthread_created_ts REAL NOT NULL,
            cthread_updated_ts REAL NOT NULL,
            cthread_archived_ts REAL NOT NULL
        )",
        [],
    ).map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS cmessages (
            cmessage_belongs_to_cthread_id TEXT NOT NULL,
            cmessage_alt INT NOT NULL,
            cmessage_num INT NOT NULL,
            cmessage_prev_alt INT NOT NULL,
            cmessage_usage_model TEXT NOT NULL,
            cmessage_usage_prompt TEXT NOT NULL,
            cmessage_usage_completion TEXT NOT NULL,
            cmessage_json TEXT NOT NULL,
            PRIMARY KEY (cmessage_belongs_to_cthread_id, cmessage_alt, cmessage_num),
            FOREIGN KEY (cmessage_belongs_to_cthread_id)
                REFERENCES cthreads(cthread_id)
                ON DELETE CASCADE
        )",
        [],
    ).map_err(|e| e.to_string())?;
    Ok(())
}
