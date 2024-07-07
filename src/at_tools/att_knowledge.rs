use std::collections::HashMap;
use std::sync::Arc;
use std::fmt::Write;
use serde_json::Value;
use async_trait::async_trait;
use parking_lot::Mutex as ParkMutex;
use rand::Rng;
use rusqlite::{params, Connection, Result};
use tracing::info;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::vecdb::vecdb_cache::VecDBCache;


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
    pub vecdb_cache: Arc<AMutex<VecDBCache>>,
}

impl MemoryDatabase {
    pub fn create_table(&self) -> Result<(), String> {
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
        ).map_err(|e| e.to_string())?;
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

    pub fn print_everything(&self) -> Result<String, String> {
        let mut table_contents = String::new();
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT * FROM memories")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, i32>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;

        for row in rows {
            let (memid, m_type, m_goal, m_project, m_payload, mstat_correct, mstat_useful, mstat_times_used) = row
                .map_err(|e| e.to_string())?;
            table_contents.push_str(&format!(
                "memid={}, type={}, goal: {:?}, project: {:?}, payload: {:?}, correct={}, useful={}, times_used={}\n",
                memid, m_type, m_goal, m_project, m_payload, mstat_correct, mstat_useful, mstat_times_used
            ));
        }
        Ok(table_contents)
    }
}


pub fn mem_init(
    cache_dir: &std::path::PathBuf,
    vecdb_cache: Arc<AMutex<VecDBCache>>,
) -> Result<Arc<ParkMutex<MemoryDatabase>>, String> {
    let dbpath = cache_dir.join("memories.sqlite");
    let cache_database = Connection::open_with_flags(
        dbpath,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
    ).map_err(|err| format!("Failed to open database: {}", err))?;

    cache_database.busy_timeout(std::time::Duration::from_secs(30))
        .map_err(|err| format!("Failed to set busy timeout: {}", err))?;

    cache_database.execute_batch("PRAGMA cache_size = 0; PRAGMA shared_cache = OFF;")
        .map_err(|err| format!("Failed to set cache pragmas: {}", err))?;

    let journal_mode: String = cache_database.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(|err| format!("Failed to set journal mode: {}", err))?;

    if journal_mode != "wal" {
        return Err(format!("Failed to set WAL journal mode. Current mode: {}", journal_mode));
    }

    let db = MemoryDatabase {
        conn: Arc::new(ParkMutex::new(cache_database)),
        vecdb_cache,
    };

    db.create_table()?;

    Ok(Arc::new(ParkMutex::new(db)))
}

fn generate_memid() -> String {
    let mut rng = rand::thread_rng();
    let mut memid = String::new();
    for _ in 0..10 {
        write!(&mut memid, "{:x}", rng.gen_range(0..16)).unwrap();
    }
    memid
}
