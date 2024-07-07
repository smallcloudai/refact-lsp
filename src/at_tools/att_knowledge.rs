use std::collections::HashMap;
use serde_json::Value;
use tracing::info;
use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use rusqlite::{params, Connection, Result};
use std::sync::Arc;
use parking_lot::Mutex as ParkMutex;
use rand::Rng;
use std::fmt::Write;


pub struct AttKnowledge;

#[async_trait]
impl Tool for AttKnowledge {
    async fn execute(&self, ccx: &mut AtCommandsContext, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        info!("run @knowledge {:?}", args);
        let mut im_going_to_do = match args.get("im_going_to_do") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => { return Err(format!("argument `im_going_to_do` is not a string: {:?}", v)) },
            None => { return Err("argument `im_going_to_do` is missing".to_string()) }
        };

        let mut memories: Vec<String> = vec![];
        memories.push("memory 5f4he83\nThe Frog class represents a frog in a 2D environment, with position and velocity attributes. It is defined at /Users/kot/code/refact-lsp/tests/emergency_frog_situation/frog.py:5".to_string());

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: serde_json::to_string(&memories).unwrap(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
        }));

        Ok(results)
    }

    fn depends_on(&self) -> Vec<String> {
        vec!["ast".to_string()]
    }
}


pub struct MemoryDatabase {
    pub conn: Arc<ParkMutex<Connection>>,
}

impl MemoryDatabase {
    pub fn create_table(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memories (
                memid TEXT PRIMARY KEY,
                m_type TEXT NOT NULL,
                m_goal TEXT NOT NULL,
                m_project TEXT NOT NULL,
                m_payload TEXT NOT NULL,
                mstat_correct REAL NOT NULL DEFAULT 0,
                mstat_useful REAL NOT NULL DEFAULT 0,
                mstat_times_used INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        Ok(())
    }

    pub fn add(&self, memtype: &str, goal: &str, project: &str, payload: &str) -> Result<String> {
        let conn = self.conn.lock();
        let memid = generate_memid();
        conn.execute(
            "INSERT INTO memories (memid, m_type, m_goal, m_project, m_payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![memid, memtype, goal, project, payload],
        )?;
        Ok(memid)
    }

    pub fn erase(&self, memid: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM memories WHERE memid = ?1",
            params![memid],
        )?;
        Ok(())
    }

    pub fn update_used(&self, memid: &str, mstat_correct: f64, mstat_useful: f64) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE memories SET mstat_times_used = mstat_times_used + 1, mstat_correct = ?1, mstat_useful = ?2 WHERE memid = ?3",
            params![mstat_correct, mstat_useful, memid],
        )?;
        Ok(())
    }
}

pub fn mem_init(
    cache_dir: &std::path::PathBuf,
) -> Result<Arc<ParkMutex<MemoryDatabase>>, String> {
    let dbpath = cache_dir.join("memories.sqlite");
    let cache_database = Connection::open_with_flags(
        dbpath,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
    ).map_err(|err| format!("Failed to open database: {}", err))?;

    cache_database.execute("PRAGMA journal_mode=WAL", params![])
        .map_err(|err| format!("Failed to set journal mode: {}", err))?;

    Ok(Arc::new(ParkMutex::new(MemoryDatabase {
        conn: Arc::new(ParkMutex::new(cache_database)),
    })))
}

fn generate_memid() -> String {
    let mut rng = rand::thread_rng();
    let mut memid = String::new();
    for _ in 0..10 {
        write!(&mut memid, "{:x}", rng.gen_range(0..16)).unwrap();
    }
    memid
}


#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::Arc;
    use parking_lot::Mutex as ParkMutex;

    #[test]
    fn test_memories() -> Result<(), Box<dyn std::error::Error>> {
        let conn = Connection::open_in_memory()?;
        let memory_db = MemoryDatabase {
            conn: Arc::new(ParkMutex::new(conn)),
        };

        memory_db.create_table()?;

        let m0 = memory_db.add("seq-of-acts", "compile", "proj1", "Wow, running cargo build on proj1 was successful!")?;
        let m1 = memory_db.add("proj-fact", "compile", "proj1", "Looks like proj1 is written in fact in Rust.")?;
        let m2 = memory_db.add("seq-of-acts", "compile", "proj2", "Wow, running cargo build on proj2 was successful!")?;
        let m3 = memory_db.add("proj-fact", "compile", "proj2", "Looks like proj2 is written in fact in Rust.")?;

        memory_db.update_used(&m1, 0.95, 0.85)?;
        memory_db.erase(&m0)?;

        Ok(())
    }
}
