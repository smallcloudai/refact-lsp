use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify as ANotify;
use tokio::sync::RwLock as ARwLock;
use parking_lot::Mutex as ParkMutex;
use rusqlite::Connection;
use std::sync::atomic::AtomicBool;

use crate::agent_db::db_structs::ChoreDB;
use crate::global_context::GlobalContext;


fn _make_connection(
    _chore_db_fn: String,
) -> Result<Arc<ParkMutex<ChoreDB>>, String> {
    let db = Connection::open_with_flags(
        "experimental_db.sqlite",
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
    ).map_err(|err| format!("Failed to open database: {}", err))?;
    db.busy_timeout(std::time::Duration::from_secs(30)).map_err(|err| format!("Failed to set busy timeout: {}", err))?;
    db.execute_batch("PRAGMA cache_size = 0; PRAGMA shared_cache = OFF;").map_err(|err| format!("Failed to set cache pragmas: {}", err))?;
    let journal_mode: String = db.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0)).map_err(|err| format!("Failed to set journal mode: {}", err))?;
    if journal_mode != "wal" {
        return Err(format!("Failed to set WAL journal mode. Current mode: {}", journal_mode));
    }
    let db = ChoreDB {
        lite: Arc::new(ParkMutex::new(db)),
        chore_sleeping_point: Arc::new(ANotify::new()),
    };
    Ok(Arc::new(ParkMutex::new(db)))
}

pub async fn chore_db_init(
    chore_db_fn: String,
) -> Arc<ParkMutex<ChoreDB>> {
    let db = match _make_connection(chore_db_fn) {
        Ok(db) => db,
        Err(err) => panic!("Failed to initialize chore database: {}", err),
    };
    let lite_arc = {
        db.lock().lite.clone()
    };
    crate::agent_db::db_schema_20241102::create_tables_20241102(&*lite_arc.lock(), false).expect("Failed to create tables");
    db
}

pub async fn pubsub_sleeping_procedure(
    gcx: Arc<ARwLock<GlobalContext>>,
    db: &Arc<ParkMutex<ChoreDB>>,
) -> bool {
    let shutdown_flag: Arc<AtomicBool> = gcx.read().await.shutdown_flag.clone();
    if shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let sleeping_point = db.lock().chore_sleeping_point.clone();
    match tokio::time::timeout(Duration::from_secs(5), sleeping_point.notified()).await {
        Ok(_) => { },
        Err(_) => { },   // timeout
    }
    let should_continue = !shutdown_flag.load(std::sync::atomic::Ordering::Relaxed);
    should_continue
}
