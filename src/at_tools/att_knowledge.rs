use std::collections::HashMap;
use std::sync::Arc;
use std::fmt::Write;
use std::path::PathBuf;
use serde_json::Value;
use tracing::info;

use async_trait::async_trait;
use parking_lot::Mutex as ParkMutex;
use rand::Rng;
use rusqlite::{params, Connection, Result};
use arrow::array::{ArrayData, Float32Array, StringArray, FixedSizeListArray, RecordBatchIterator, RecordBatch};
use arrow::buffer::Buffer;
use arrow::compute::concat_batches;
use arrow_array::cast::{as_fixed_size_list_array, as_primitive_array, as_string_array};
use arrow_array::types::Float32Type;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures_util::TryStreamExt;
use itertools::Itertools;
use lance::dataset::{WriteMode, WriteParams};
use lance_linalg::distance::cosine_distance;
use reqwest::Client;
use vectordb::database::Database;
use tempfile::TempDir;
use tokio::sync::Mutex as AMutex;
use vectordb::table::Table;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::vecdb::vecdb_cache::VecDBCache;
use crate::vecdb::structs::{MemoRecord, SimpleTextHashVector, VecdbConstants, VecDbStatus};
use crate::ast::chunk_utils::official_text_hashing_function;


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


pub struct MemoriesDatabase {
    pub conn: Arc<ParkMutex<Connection>>,
    pub vecdb_constants: VecdbConstants,
    pub memories_table: Table,
    pub schema_arc: SchemaRef,
    pub dirty_memids: Vec<String>,
    pub dirty_everything: bool,
}

impl MemoriesDatabase {
    pub async fn init(
        cache_dir: &PathBuf,
        // vecdb_cache: Arc<AMutex<VecDBCache>>,
        constants: &VecdbConstants,
    ) -> Result<MemoriesDatabase, String> {
        // SQLite database for memories, permanent on disk
        let dbpath = cache_dir.join("memories.sqlite");
        let cache_database = Connection::open_with_flags(
            dbpath,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
        ).map_err(|err| format!("Failed to open database: {}", err))?;
        cache_database.busy_timeout(std::time::Duration::from_secs(30)).map_err(|err| format!("Failed to set busy timeout: {}", err))?;
        cache_database.execute_batch("PRAGMA cache_size = 0; PRAGMA shared_cache = OFF;").map_err(|err| format!("Failed to set cache pragmas: {}", err))?;
        let journal_mode: String = cache_database.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0)).map_err(|err| format!("Failed to set journal mode: {}", err))?;
        if journal_mode != "wal" {
            return Err(format!("Failed to set WAL journal mode. Current mode: {}", journal_mode));
        }

        // Arrow database for embeddings, only valid for the current process
        let embedding_temp_dir = TempDir::new().map_err(|e| format!("Failed to create temp dir: {}", e))?;
        let embedding_path = embedding_temp_dir.path().to_str().unwrap();
        let schema_arc = Arc::new(Schema::new(vec![
            Field::new("mem_id", DataType::Utf8, false),
            Field::new("vector", DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                constants.embedding_size,
            ), false),
        ]));
        let temp_database = Database::connect(embedding_path).await.map_err(|err| format!("Failed to connect to database: {:?}", err))?;
        let batches_iter = RecordBatchIterator::new(vec![].into_iter().map(Ok), schema_arc.clone());
        let memories_table = match temp_database.create_table("memories", batches_iter, Option::from(WriteParams::default())).await {
            Ok(t) => t,
            Err(err) => return Err(format!("{:?}", err))
        };

        // Return everything
        let db = MemoriesDatabase {
            conn: Arc::new(ParkMutex::new(cache_database)),
            vecdb_constants: constants.clone(),
            memories_table,
            schema_arc,
            dirty_memids: Vec::new(),
            dirty_everything: true,
        };
        db._create_table()?;
        Ok(db)
    }

    fn _create_table(&self) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memories (
                mem_id TEXT PRIMARY KEY,
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

    pub fn add(&self, mem_type: &str, goal: &str, project: &str, payload: &str) -> Result<String, String> {
        fn generate_mem_id() -> String {
            let mut rng = rand::thread_rng();
            let mut mem_id = String::new();
            for _ in 0..10 {
                write!(&mut mem_id, "{:x}", rng.gen_range(0..16)).unwrap();
            }
            mem_id
        }

        let conn = self.conn.lock();
        let mem_id = generate_mem_id();
        conn.execute(
            "INSERT INTO memories (mem_id, m_type, m_goal, m_project, m_payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![mem_id, mem_type, goal, project, payload],
        ).map_err(|e| e.to_string())?;
        Ok(mem_id)
    }

    pub fn erase(&self, mem_id: &str) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM memories WHERE mem_id = ?1",
            params![mem_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_used(&self, mem_id: &str, mstat_correct: f64, mstat_useful: f64) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE memories SET mstat_times_used = mstat_times_used + 1, mstat_correct = ?1, mstat_useful = ?2 WHERE mem_id = ?3",
            params![mstat_correct, mstat_useful, mem_id],
        ).map_err(|e| e.to_string())?;
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
            let (mem_id, m_type, m_goal, m_project, m_payload, mstat_correct, mstat_useful, mstat_times_used) = row
                .map_err(|e| e.to_string())?;
            table_contents.push_str(&format!(
                "mem_id={}, type={}, goal: {:?}, project: {:?}, payload: {:?}, correct={}, useful={}, times_used={}\n",
                mem_id, m_type, m_goal, m_project, m_payload, mstat_correct, mstat_useful, mstat_times_used
            ));
        }
        Ok(table_contents)
    }

    fn parse_table_iter(
        record_batch: RecordBatch,
        include_embedding: bool,
        embedding_to_compare: Option<&Vec<f32>>,
    ) -> vectordb::error::Result<Vec<MemoRecord>> {
        (0..record_batch.num_rows()).map(|idx| {
            let gathered_vec = as_primitive_array::<Float32Type>(
                &as_fixed_size_list_array(record_batch.column_by_name("vector").unwrap())
                    .iter()
                    .map(|x| x.unwrap())
                    .collect::<Vec<_>>()[idx]
            )
                .iter()
                .map(|x| x.unwrap()).collect::<Vec<_>>();
            let distance = match embedding_to_compare {
                None => { -1.0 }
                Some(embedding) => { cosine_distance(&embedding, &gathered_vec) }
            };
            let embedding = match include_embedding {
                true => Some(gathered_vec),
                false => None
            };

            Ok(MemoRecord {
                vector: embedding,
                mem_id: as_string_array(record_batch.column_by_name("mem_id")
                   .expect("Missing column 'mem_id'"))
                   .value(idx)
                   .to_string(),
                distance,
            })
        }).collect()
    }

    pub async fn search(
        &mut self,
        embedding: &Vec<f32>,
        top_n: usize,
    ) -> vectordb::error::Result<Vec<MemoRecord>> {
        let query = self.memories_table
            .clone()
            .search(Some(Float32Array::from(embedding.clone())))
            .limit(top_n)
            .use_index(true)
            .execute().await?
            .try_collect::<Vec<_>>().await?;
        let record_batch = concat_batches(&self.schema_arc, &query)?;
        
        match MemoriesDatabase::parse_table_iter(record_batch, false, Some(&embedding)) {
            Ok(records) => {
                let filtered = records.into_iter().dedup().sorted_unstable_by(|a, b|a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal)).collect::<Vec<_>>();
                Ok(filtered)
            },
            Err(e) => Err(e),
        }
    }
}

async fn recall_payload_for_dirty_memories_and_mark_them_not_dirty(
    memdb: Arc<AMutex<MemoriesDatabase>>,
) -> Result<(Vec<String>, Vec<SimpleTextHashVector>), String> {
    let mut memids: Vec<String> = Vec::new();
    let mut todo: Vec<SimpleTextHashVector> = Vec::new();
    let mut memdb_locked = memdb.lock().await;
    let rows: Vec<(String, String)> = {
        let conn = memdb_locked.conn.lock();
        if memdb_locked.dirty_everything {
            let mut stmt = conn.prepare("SELECT mem_id, m_payload FROM memories")
                .map_err(|e| format!("Failed to prepare statement: {}", e))?;
            let x = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                ))
            })
            .map_err(|e| format!("Failed to query memories: {}", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect rows: {}", e))?;
            x
        } else if !memdb_locked.dirty_memids.is_empty() {
            let placeholders = (0..memdb_locked.dirty_memids.len())
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let query = format!("SELECT mem_id, m_payload FROM memories WHERE mem_id IN ({})", placeholders);
            let mut stmt = conn.prepare(&query)
                .map_err(|e| format!("Failed to prepare statement: {}", e))?;
            let x = stmt.query_map(rusqlite::params_from_iter(memdb_locked.dirty_memids.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                ))
            })
            .map_err(|e| format!("Failed to query memories: {}", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect rows: {}", e))?;
            x
        } else {
            Vec::new()
        }
    };
    for (mem_id, m_payload) in rows {
        let window_text_hash = official_text_hashing_function(&m_payload);
        let simple_text_hash_vector = SimpleTextHashVector {
            window_text: m_payload,
            window_text_hash,
            vector: None,
        };
        memids.push(mem_id);
        todo.push(simple_text_hash_vector);
    }
    memdb_locked.dirty_memids.clear();
    memdb_locked.dirty_everything = false;
    Ok((memids, todo))
}

pub async fn vectorize_dirty_memories(
    memdb: Arc<AMutex<MemoriesDatabase>>,
    vecdb_cache: Arc<AMutex<VecDBCache>>,
    status: Arc<AMutex<VecDbStatus>>,
    client: Arc<AMutex<Client>>,
    api_key: &String,
    #[allow(non_snake_case)]
    B: usize,
) -> Result<(), String> {
    let (memids, mut todo) = recall_payload_for_dirty_memories_and_mark_them_not_dirty(memdb.clone()).await?;
    if memids.is_empty() {
        return Ok(());
    }

    {
        let mut cache_locked = vecdb_cache.lock().await;
        cache_locked.process_simple_hash_text_vector(&mut todo).await.map_err(|e| format!("Failed to get vectors from cache: {}", e))?
        // this makes todo[].vector appear for records that exist in cache
    }

    let todo_len = todo.len();
    let mut to_vectorize = todo.iter_mut().filter(|x| x.vector.is_none()).collect::<Vec<&mut SimpleTextHashVector>>();
    info!("{} memories total, {} to vectorize", todo_len, to_vectorize.len());
    let my_constants: VecdbConstants = memdb.lock().await.vecdb_constants.clone();
    for chunk in to_vectorize.chunks_mut(B) {
        let texts: Vec<String> = chunk.iter().map(|x| x.window_text.clone()).collect();
        let embedding_mb = crate::fetch_embedding::get_embedding_with_retry(
            client.clone(),
            &my_constants.endpoint_embeddings_style,
            &my_constants.model_name,
            &my_constants.endpoint_embeddings_template,
            texts,
            api_key,
            1,
        ).await?;
        for (chunk_save, x) in chunk.iter_mut().zip(embedding_mb.iter()) {
            chunk_save.vector = Some(x.clone());  // <-- this will make the rest of todo[].vector appear
        }
    }

    {
        let mut cache_locked = vecdb_cache.lock().await;
        let temp_vec: Vec<SimpleTextHashVector> = to_vectorize.iter().map(|x| (**x).clone()).collect();
        cache_locked.cache_add_new_records(temp_vec).await.map_err(|e| format!("Failed to update cache: {}", e))?;
    }

    // Save to lance
    fn make_emb_data(records: &Vec<SimpleTextHashVector>, embedding_size: i32) -> Result<ArrayData, String> {
        let vec_trait = Arc::new(Field::new("item", DataType::Float32, true));
        let mut emb_builder: Vec<f32> = vec![];
        for record in records {
            assert!(record.vector.is_some());
            assert_eq!(record.vector.as_ref().unwrap().len(), embedding_size as usize);
            emb_builder.append(&mut record.vector.clone().expect("No embedding is provided"));
        }
        let emb_data_res = ArrayData::builder(DataType::Float32)
            .add_buffer(Buffer::from_vec(emb_builder))
            .len(records.len() * embedding_size as usize)
            .build();
        let emb_data = match emb_data_res {
            Ok(res) => res,
            Err(err) => { return Err(format!("{:?}", err)); }
        };
        match ArrayData::builder(DataType::FixedSizeList(vec_trait.clone(), embedding_size))
            .len(records.len())
            .add_child_data(emb_data.clone())
            .build()
        {
            Ok(res) => Ok(res),
            Err(err) => return Err(format!("{:?}", err))
        }
    }
    let vectors: ArrayData = match make_emb_data(&todo, my_constants.embedding_size) {
        Ok(res) => res,
        Err(err) => return Err(format!("{:?}", err))
    };

    let my_schema_arc = memdb.lock().await.schema_arc.clone();
    let data_batches_iter = RecordBatchIterator::new(
        vec![RecordBatch::try_new(
            my_schema_arc.clone(),
            vec![
                Arc::new(StringArray::from(memids)),
                Arc::new(FixedSizeListArray::from(vectors.clone())),
            ],
        )],
        my_schema_arc.clone(),
    );
    let data_res = {
        let mut memdb_locked = memdb.lock().await;
        memdb_locked.memories_table.add(
            data_batches_iter,
            Some(WriteParams {
                mode: WriteMode::Append,
                ..Default::default()
            }),
        ).await
    };
    info!("Updated {} memories in the database:\n{:?}", todo.len(), data_res);

    Ok(())
}
