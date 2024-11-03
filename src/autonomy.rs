use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock as ARwLock;
use indexmap::IndexSet;

use crate::global_context::GlobalContext;
use crate::agent_db::db_structs::{CThread, CMessage};
use crate::agent_db::chore_pubsub_sleeping_procedure;
use crate::agent_db::db_cthread::CThreadSubscription;

const SLEEP_IF_NO_WORK_SEC: u64 = 10;
const LOCK_TOO_OLD_SEC: f64 = 600.0;


pub async fn look_for_a_job(
    gcx: Arc<ARwLock<GlobalContext>>,
    worker_n: usize,
) {
    let worker_pid = std::process::id();
    let worker_name = format!("aworker/{}/{}", worker_pid, worker_n);
    let cdb = gcx.read().await.chore_db.clone();
    let lite_arc = cdb.lock().lite.clone();
    let mut might_work_on_cthread_id: IndexSet<String> = IndexSet::new();
    let post = CThreadSubscription {
        quicksearch: "".to_string(),
        limit: 100,
    };

    // intentional unwrap(), it's better to crash than continue with a non-functioning threads
    let mut last_pubsub_id = {
        let lite = cdb.lock().lite.clone();
        let max_pubsub_id: i64 = lite.lock().query_row("SELECT COALESCE(MAX(pubevent_id), 0) FROM pubsub_events", [], |row| row.get(0)).unwrap();
        let cthreads = crate::agent_db::db_cthread::cthread_quicksearch(cdb.clone(), &String::new(), &post).unwrap();
        for ct in cthreads.iter() {
            might_work_on_cthread_id.insert(ct.cthread_id.clone());
        }
        max_pubsub_id
    };

    loop {
        let sleep_seconds = if might_work_on_cthread_id.is_empty() { SLEEP_IF_NO_WORK_SEC } else { 1 };
        if !chore_pubsub_sleeping_procedure(gcx.clone(), &cdb, sleep_seconds).await {
            break;
        }
        let (deleted_cthread_ids, updated_cthread_ids) = match crate::agent_db::db_cthread::cthread_subsription_poll(lite_arc.clone(), &mut last_pubsub_id) {
            Ok(x) => x,
            Err(e) => {
                tracing::error!("wait_for_cthread_to_work_on(1): {}", e);
                break;
            }
        };
        might_work_on_cthread_id.extend(updated_cthread_ids.into_iter());
        for deleted_id in deleted_cthread_ids {
            might_work_on_cthread_id.remove(&deleted_id);
        }

        while let Some(cthread_id) = might_work_on_cthread_id.iter().next().cloned() {
            match look_if_the_job_for_me(gcx.clone(), &worker_name, &cthread_id).await {
                Ok(lock_success) => {
                    if lock_success {
                        might_work_on_cthread_id.remove(&cthread_id);
                    }
                }
                Err(e) => {
                    tracing::error!("{} cannot work on {}: {}", worker_name, cthread_id, e);
                    might_work_on_cthread_id.remove(&cthread_id);
                    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                }
            }
        }
    }
}

async fn look_if_the_job_for_me(
    gcx: Arc<ARwLock<GlobalContext>>,
    worker_name: &String,
    cthread_id: &String,
) -> Result<bool, String> {
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs_f64();
    let cdb = gcx.read().await.chore_db.clone();
    let lite_arc = cdb.lock().lite.clone();
    let (cthread_rec, cmessages) = {
        let mut conn = lite_arc.lock();
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        let mut cthread_rec = {
            let mut stmt = tx.prepare("SELECT * FROM cthreads WHERE cthread_id = ?1").unwrap();
            let rows = stmt.query(rusqlite::params![cthread_id]).map_err(|e| e.to_string())?;
            let mut cthreads = crate::agent_db::db_cthread::cthreads_from_rows(rows);
            cthreads.pop().ok_or_else(|| format!("No CThread found with id: {}", cthread_id))?
        };

        let cmessages = {
            let mut stmt = tx.prepare("SELECT * FROM cmessages WHERE cmessage_belongs_to_cthread_id = ?1 ORDER BY cmessage_num, cmessage_alt").unwrap();
            let rows = stmt.query(rusqlite::params![cthread_id]).map_err(|e| e.to_string())?;
            crate::agent_db::db_cmessage::cmessages_from_rows(rows)
        };

        assert!(cthread_rec.cthread_locked_by != *worker_name);

        let busy = !cthread_rec.cthread_locked_by.is_empty() && cthread_rec.cthread_locked_ts + LOCK_TOO_OLD_SEC > now;
        if busy {
            tracing::info!("{} {} busy", worker_name, cthread_id);
            return Ok(false);
        }

        let last_message_is_user = cmessages.last().map_or(false, |cmsg| {
            let cmessage: serde_json::Value = serde_json::from_str(&cmsg.cmessage_json).unwrap();
            cmessage["role"] == "user"
        });

        tracing::info!("{} {} last_message_is_user={} cthread_rec.cthread_error={:?}", worker_name, cthread_id, last_message_is_user, cthread_rec.cthread_error);
        if !last_message_is_user || !cthread_rec.cthread_error.is_empty() {
            return Ok(true);  // true means don't come back to it again
        }

        cthread_rec.cthread_locked_by = worker_name.clone();
        cthread_rec.cthread_locked_ts = now;
        crate::agent_db::db_cthread::cthread_set_lowlevel(&tx, &cthread_rec)?;
        tx.commit().map_err(|e| e.to_string())?;
        (cthread_rec, cmessages)
    };

    tracing::info!("{} {} autonomous work start", worker_name, cthread_id);
    let mut apply_json: serde_json::Value;

    match do_the_job(gcx, worker_name, &cthread_rec, &cmessages) {
        Ok(result) => {
            apply_json = result;
        }
        Err(e) => {
            apply_json = serde_json::json!({
                "cthread_error": format!("{}", e),
            });
        }
    }
    apply_json["cthread_id"] = serde_json::json!(cthread_id);
    apply_json["cthread_locked_by"] = serde_json::json!("");
    apply_json["cthread_locked_ts"] = serde_json::json!(0);
    tracing::info!("{} {} /autonomous work\n{}", worker_name, cthread_id, apply_json);
    crate::agent_db::db_cthread::cthread_apply_json(cdb, apply_json)?;
    return Ok(true);
}

fn do_the_job(
    gcx: Arc<ARwLock<GlobalContext>>,
    worker_name: &String,
    cthread_rec: &CThread,
    cmessages: &Vec<CMessage>,
) -> Result<serde_json::Value, String> {
    return Ok(serde_json::json!({}));
}

pub async fn look_for_a_job_start_tasks(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    for n in 0..1 {
        let handle = tokio::spawn(look_for_a_job(
            gcx.clone(),
            n,
        ));
        handles.push(handle);
    }
    handles
}
