use std::collections::HashMap;
use std::sync::Arc;
use std::fmt::Write;
use serde_json::Value;
use async_trait::async_trait;
use parking_lot::Mutex as ParkMutex;
use rand::Rng;
use rusqlite::{params, Connection, Result};
use tracing::info;
use arrow::array::{ArrayData, Float32Array, StringArray, FixedSizeListArray, FixedSizeListBuilder, Float32Builder};
use arrow::buffer::Buffer;
use arrow_array::{RecordBatchIterator, RecordBatchReader};
use arrow_array::{RecordBatch, UInt64Array};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
// use arrow::datatypes::{DataType, Field, Schema};
// use arrow::record_batch::RecordBatch;
use lance::dataset::{WriteMode, WriteParams};
use vectordb::database::Database;
use tempfile::TempDir;
use tokio::sync::Mutex as AMutex;
use vectordb::table::Table;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::vecdb::vecdb_cache::VecDBCache;
use crate::vecdb::structs::{SimpleTextHashVector, VecdbConstants};
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


pub struct MemoryDatabase {
    pub conn: Arc<ParkMutex<Connection>>,
    pub vecdb_constants: VecdbConstants,
    pub vecdb_cache: Arc<AMutex<VecDBCache>>,
    pub data_table: Table,
    pub schema: SchemaRef,
    pub embedding_temp_dir: TempDir,
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

    pub async fn vectorize_all_payloads(
        &mut self,
        status: Arc<AMutex<crate::vecdb::structs::VecDbStatus>>,
        client: Arc<AMutex<reqwest::Client>>,
        api_key: &String,
        #[allow(non_snake_case)]
        B: usize,
   ) -> Result<(), String> {
        let mut memids: Vec<String> = Vec::new();
        let mut todo: Vec<SimpleTextHashVector> = Vec::new();
        {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare("SELECT memid, m_payload FROM memories")
                .map_err(|e| format!("Failed to prepare statement: {}", e))?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                ))
            }).map_err(|e| format!("Failed to query memories: {}", e))?;
            for row in rows {
                let (memid, m_payload) = row.map_err(|e| format!("Failed to read row: {}", e))?;
                let window_text_hash = official_text_hashing_function(&m_payload);
                let simple_text_hash_vector = SimpleTextHashVector {
                    window_text: m_payload,
                    window_text_hash,
                    vector: None,
                };
                memids.push(memid);
                todo.push(simple_text_hash_vector);
            }
        }

        {
            let mut cache = self.vecdb_cache.lock().await;
            cache.process_simple_hash_text_vector(&mut todo).await.map_err(|e| format!("Failed to vectorize payload: {}", e))?
            // this makes .vector appear for records that exist in cache
        }

        let mut to_vectorize = todo.iter_mut().filter(|x| x.vector.is_none()).collect::<Vec<&mut SimpleTextHashVector>>();
        for chunk in to_vectorize.chunks_mut(B) {
            let texts: Vec<String> = chunk.iter().map(|x| x.window_text.clone()).collect();
            let embedding_mb = crate::fetch_embedding::get_embedding_with_retry(
                client.clone(),
                &self.vecdb_constants.endpoint_embeddings_style,
                &self.vecdb_constants.model_name,
                &self.vecdb_constants.endpoint_embeddings_template,
                texts,
                api_key,
                1,
            ).await?;
            for (chunk_save, x) in chunk.iter_mut().zip(embedding_mb.iter()) {
                chunk_save.vector = Some(x.clone());  // <-- look here
            }
        }
        {
            let mut cache = self.vecdb_cache.lock().await;
            let temp_vec: Vec<SimpleTextHashVector> = to_vectorize.iter().map(|x| (**x).clone()).collect();
            cache.cache_add_new_records(temp_vec).await.map_err(|e| format!("Failed to update cache: {}", e))?;
        }

        // let schema = Arc::new(Schema::new(vec![
        //     Field::new("memid", DataType::Utf8, false),
        //     Field::new("thevec", DataType::FixedSizeList(
        //         Arc::new(Field::new("item", DataType::Float32, false)),
        //         constants.embedding_size,
        //     ), false),
        // ]));

        fn make_emb_data(records: &Vec<SimpleTextHashVector>, embedding_size: i32) -> Result<ArrayData, String> {
            let vec_trait = Arc::new(Field::new("item", DataType::Float32, true));
            let mut emb_builder: Vec<f32> = vec![];
            for record in records {
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
        let vectors: ArrayData = match make_emb_data(&todo, self.vecdb_constants.embedding_size) {
            Ok(res) => res,
            Err(err) => return Err(format!("{:?}", err))
        };
        let data_batches_iter = RecordBatchIterator::new(
            vec![RecordBatch::try_new(
                self.schema.clone(),
                vec![
                    Arc::new(FixedSizeListArray::from(vectors.clone())),
                    Arc::new(StringArray::from(memids)),
                ],
            )],
            self.schema.clone(),
        );
        let data_res = self.data_table.add(
            data_batches_iter,
            Some(WriteParams {
                mode: WriteMode::Append,
                ..Default::default()
            }),
        ).await;
        info!("Added {} memories to the database:\n{:?}", todo.len(), data_res);

        Ok(())
    }
}


pub async fn mem_init(
    cache_dir: &std::path::PathBuf,
    vecdb_cache: Arc<AMutex<VecDBCache>>,
    constants: &VecdbConstants,
) -> Result<Arc<AMutex<MemoryDatabase>>, String> {
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
    let schema = Arc::new(Schema::new(vec![
        Field::new("memid", DataType::Utf8, false),
        Field::new("thevec", DataType::FixedSizeList(
            Arc::new(Field::new("item", DataType::Float32, false)),
            constants.embedding_size,
        ), false),
    ]));
    let temp_database = Database::connect(embedding_path).await.map_err(|err| format!("Failed to connect to database: {:?}", err))?;
    let batches_iter = RecordBatchIterator::new(vec![].into_iter().map(Ok), schema.clone());
    let data_table = match temp_database.create_table("data", batches_iter, Option::from(WriteParams::default())).await {
        Ok(table) => table,
        Err(err) => return Err(format!("{:?}", err))
    };

    // Return everything
    let db = MemoryDatabase {
        conn: Arc::new(ParkMutex::new(cache_database)),
        vecdb_constants: constants.clone(),
        vecdb_cache: vecdb_cache,
        data_table,
        schema,
        embedding_temp_dir,
    };
    db.create_table()?;
    Ok(Arc::new(AMutex::new(db)))
}

fn generate_memid() -> String {
    let mut rng = rand::thread_rng();
    let mut memid = String::new();
    for _ in 0..10 {
        write!(&mut memid, "{:x}", rng.gen_range(0..16)).unwrap();
    }
    memid
}
