use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex};
use indexmap::IndexSet;

use crate::global_context::GlobalContext;
use crate::agent_db::db_structs::{CThread, CMessage};
use crate::agent_db::chore_pubsub_sleeping_procedure;
use crate::agent_db::db_cthread::CThreadSubscription;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::at_commands::at_commands::AtCommandsContext;

const SLEEP_IF_NO_WORK_SEC: u64 = 10;
const LOCK_TOO_OLD_SEC: f64 = 600.0;


pub async fn look_for_a_job(
    gcx: Arc<ARwLock<GlobalContext>>,
    worker_n: usize,
) {
    let worker_pid = std::process::id();
    let worker_name = format!("aworker-{}-{}", worker_pid, worker_n);
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

    match do_the_job(gcx, worker_name, &cthread_rec, &cmessages).await {
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

    Ok(true)  // true means don't come back to it again
}

async fn do_the_job(
    gcx: Arc<ARwLock<GlobalContext>>,
    worker_name: &String,
    cthread_rec: &CThread,
    cmessages: &Vec<CMessage>,
) -> Result<serde_json::Value, String> {
    let cdb = gcx.read().await.chore_db.clone();
    let (lite, chore_sleeping_point) = {
        let db = cdb.lock();
        (db.lite.clone(), db.chore_sleeping_point.clone())
    };

    let messages: Vec<ChatMessage> = cmessages.iter().map(|cmsg| { serde_json::from_str(&cmsg.cmessage_json).map_err(|e| format!("{}", e))}).collect::<Result<Vec<_>, _>>()?;
    let message_info: Vec<String> = messages.iter().map(|msg| {
        let role = &msg.role;
        let content_brief = match &msg.content {
            ChatContent::SimpleText(text) => { format!("{}", text.len()) },
            ChatContent::Multimodal(elements) => {
                elements.iter().map(|el| {
                    if el.is_text() {
                        format!("text{}", el.m_content.len())
                    } else {
                        format!("{}[image]", el.m_type)
                    }
                }).collect::<Vec<_>>().join("+")
            },
        };
        let mut tool_calls_brief = match &msg.tool_calls {
            Some(tool_calls) => tool_calls.iter().map(|call| call.function.name.clone()).collect::<Vec<_>>().join("/"),
            None => String::new(),
        };
        if !tool_calls_brief.is_empty() {
            tool_calls_brief.insert(0, '/');
        }
        format!("{}/{}{}", role, content_brief, tool_calls_brief)
    }).collect();
    let message_info_str = message_info.join(", ");
    tracing::info!("{} started work on {}\n[{}]", worker_name, cthread_rec.cthread_id, message_info_str);

    // wrap_up_depth: usize,
    // wrap_up_tokens_cnt: usize,
    // wrap_up_prompt: &str,
    // wrap_up_n: usize,
    let tools_turned_on_by_cmdline = crate::tools::tools_description::tools_merged_and_filtered(gcx.clone()).await?;
    let allow_experimental = gcx.read().await.cmdline.experimental;
    let tools_desclist = crate::tools::tools_description::tool_description_list_from_yaml(
        tools_turned_on_by_cmdline,
        None,
        allow_experimental
    ).await?;
    let tools = tools_desclist.into_iter().filter_map(|tool_desc| {
        let good =
            (cthread_rec.cthread_toolset == "explore" && !tool_desc.agentic) ||
            (cthread_rec.cthread_toolset == "agent");
        if good {
            Some(tool_desc.into_openai_style())
        } else {
            None
        }
    }).collect::<Vec<_>>();

    let max_new_tokens = 2048;
    let n = 1;
    let only_deterministic_messages = false;
    let (mut chat_post, spad) = crate::subchat::create_chat_post_and_scratchpad(
        gcx.clone(),
        &cthread_rec.cthread_model,
        messages.iter().collect::<Vec<_>>(),
        Some(cthread_rec.cthread_temperature as f32),
        max_new_tokens,
        n,
        Some(tools),
        None,
        only_deterministic_messages,
    ).await?;
    let n_ctx = chat_post.max_tokens;  // create_chat_post_and_scratchpad saves n_ctx here :/

    let ccx: Arc<AMutex<AtCommandsContext>> = Arc::new(AMutex::new(AtCommandsContext::new(
        gcx.clone(),
        n_ctx,
        7,
        false,
        messages.clone(),
        cthread_rec.cthread_id.clone(),
    ).await));

    // XXX at commands
    tracing::info!("{} start chat_interaction()", worker_name);
    let chat_response_msgs = crate::subchat::chat_interaction(ccx.clone(), spad, &mut chat_post).await?;
    if chat_response_msgs.len() == 0 {
        return Err("Oops strange, chat_interaction() returned no choices".to_string());
    }
    let choice0: Vec<ChatMessage> = chat_response_msgs[0].clone();

    {
        let mut lite_locked = lite.lock();
        let tx = lite_locked.transaction().map_err(|e| e.to_string())?;
        for (i, chat_message) in choice0.iter().enumerate() {
            let mut cmessage_usage_prompt = 0;
            let mut cmessage_usage_completion = 0;
            if let Some(u) = &chat_message.usage {
                cmessage_usage_prompt = u.prompt_tokens as i32;
                cmessage_usage_completion = u.completion_tokens as i32;
            } else {
                tracing::warn!("running {} didn't produce usage so it's hard to calculate tokens :/", cthread_rec.cthread_model);
            }
            let cmessage = CMessage {
                cmessage_belongs_to_cthread_id: cthread_rec.cthread_id.clone(),
                cmessage_alt: 0,
                cmessage_num: (cmessages.len() as i32) + (i as i32),
                cmessage_prev_alt: 0,
                cmessage_usage_model: cthread_rec.cthread_model.clone(),
                cmessage_usage_prompt,
                cmessage_usage_completion,
                cmessage_json: serde_json::to_string(chat_message).map_err(|e| format!("{}", e))?,
            };
            crate::agent_db::db_cmessage::cmessage_set(&tx, cmessage);
        }
        tx.commit().map_err(|e| e.to_string())?;
    }
    chore_sleeping_point.notify_waiters();


    // let old_messages = messages.clone();
    // let results = chat_response_msgs.iter().map(|new_msgs| {
    //     let mut extended_msgs = old_messages.clone();
    //     extended_msgs.extend(new_msgs.clone());
    //     extended_msgs
    // }).collect::<Vec<Vec<ChatMessage>>>();

    // if let Some(usage_collector) = usage_collector_mb {
    //     crate::subchat::update_usage_from_messages(usage_collector, &results);
    // }

    // {
    //     // keep session
    //     let mut step_n = 0;
    //     loop {
    //         let last_message = messages.last().unwrap();
    //         // if last_message.role == "assistant" && last_message.tool_calls.is_none() {
    //             // don't have tool calls, exit the loop unconditionally, model thinks it has finished the work
    //             break;
    //         }
    //         if last_message.role == "assistant" && last_message.tool_calls.is_some() {
    //             // have tool calls, let's see if we need to wrap up or not
    //             if step_n >= wrap_up_depth {
    //                 break;
    //             }
    //             if let Some(usage) = &last_message.usage {
    //                 if usage.prompt_tokens + usage.completion_tokens > wrap_up_tokens_cnt {
    //                     break;
    //                 }
    //             }
    //         }
    //         messages = subchat_single(
    //             ccx.clone(),
    //             model_name,
    //             messages.clone(),
    //             tools_subset.clone(),
    //             Some("auto".to_string()),
    //             false,
    //             temperature,
    //             None,
    //             1,
    //             Some(&mut usage_collector),
    //             tx_toolid_mb.clone(),
    //             tx_chatid_mb.clone(),
    //         ).await?[0].clone();
    //         step_n += 1;
    //     }
    //     // result => session
    // }
    // let last_message = messages.last().unwrap();
    // if let Some(tool_calls) = &last_message.tool_calls {
    //     if !tool_calls.is_empty() {
    //         messages = subchat_single(
    //             ccx.clone(),
    //             model_name,
    //             messages,
    //             vec![],
    //             Some("none".to_string()),
    //             true,   // <-- only runs tool calls
    //             temperature,
    //             None,
    //             1,
    //             Some(&mut usage_collector),
    //             tx_toolid_mb.clone(),
    //             tx_chatid_mb.clone(),
    //         ).await?[0].clone();
    //     }
    // }
    // messages.push(ChatMessage::new("user".to_string(), wrap_up_prompt.to_string()));
    // let choices = subchat_single(
    //     ccx.clone(),
    //     model_name,
    //     messages,
    //     vec![],
    //     Some("none".to_string()),
    //     false,
    //     temperature,
    //     None,
    //     wrap_up_n,
    //     Some(&mut usage_collector),
    //     tx_toolid_mb.clone(),
    //     tx_chatid_mb.clone(),
    // ).await?;
    // if let Some(last_message) = messages.last_mut() {
    //     last_message.usage = Some(usage_collector);
    // }
    Ok(serde_json::json!({}))
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
