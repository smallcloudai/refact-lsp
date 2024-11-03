use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use parking_lot::Mutex as ParkMutex;
use serde_json::json;
use serde::Deserialize;
use async_stream::stream;
use indexmap::IndexSet;

use crate::global_context::GlobalContext;
use crate::agent_db::db_structs::{ChoreDB, CThread};
use crate::agent_db::chore_pubsub_sleeping_procedure;


pub async fn look_for_a_job(
    gcx: Arc<ARwLock<GlobalContext>>,
) {
    let cdb = gcx.read().await.chore_db.clone();
    let lite_arc = cdb.lock().lite.clone();
    let mut might_work_on_cthread_id: IndexSet<String> = IndexSet::new();
    let post = crate::agent_db::db_cthread::CThreadSubscription {
        quicksearch: "".to_string(),
        limit: 100,
    };
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
        if !chore_pubsub_sleeping_procedure(gcx.clone(), &cdb).await {
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

        // if _should_work_on_cthread_condition_slow(&updated_cthread, &cdb).await? {
        //     work_on_cthread(gcx.clone(), updated_cthread.cthread_id.clone()).await?;
        // }
        while let Some(cthread_id) = might_work_on_cthread_id.pop() {
            perfom_the_job(gcx.clone(), &cthread_id).await;
        }
    }
}

async fn perfom_the_job(gcx: Arc<ARwLock<GlobalContext>>, cthread_id: &String)
{
    tracing::info!("DOING MY JOB {}", cthread_id);
    // match crate::agent_db::db_cthread::cthread_quicksearch(cdb.clone(), &updated_id, &post)
}

pub async fn look_for_a_job_start_tasks(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    for _ in 0..1 {
        let handle = tokio::spawn(look_for_a_job(gcx.clone()));
        handles.push(handle);
    }
    handles
}
