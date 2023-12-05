use std::any::Any;
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use arrow::array::ArrayData;
use arrow::buffer::Buffer;
use arrow::compute::concat_batches;
use arrow_array::{FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray, UInt64Array};
use arrow_array::array::Array;
use arrow_array::cast::{as_fixed_size_list_array, as_primitive_array, as_string_array, AsArray};
use arrow_array::types::{Float32Type, UInt64Type};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures_util::{StreamExt, TryStreamExt};
use lance::dataset::{WriteMode, WriteParams};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::Instrument;
use vectordb::database::Database;
use vectordb::table::Table;

use crate::vecdb::structs::Record;

pub type VecDBHandlerRef = Arc<Mutex<VecDBHandler>>;

impl Debug for VecDBHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "VecDBHandler: {:?}", self.database.type_id())
    }
}

pub struct VecDBHandler {
    database: Database,
    table: Table,
    schema: SchemaRef,
    hashes_cache: HashSet<String>,
    embedding_size: i32,
}

async fn table_record_batch(schema: &SchemaRef, table: &Table) -> RecordBatch {
    // expose the private dataset field
    let dataset = table.search(Float32Array::from_iter_values([1.0])).dataset.clone();
    let batches = dataset.scan()
        .try_into_stream()
        .await.unwrap()
        .try_collect::<Vec<_>>()
        .await.unwrap();
    concat_batches(&schema, &batches).unwrap()
}

impl VecDBHandler {
    pub async fn init(cache_dir: PathBuf, embedding_size: i32) -> VecDBHandler {
        let database = Database::connect(cache_dir.join("vecdb").to_str().unwrap()).await.unwrap();
        let vec_trait = Arc::new(Field::new("item", DataType::Float32, true));
        let schema = Arc::new(Schema::new(vec![
            Field::new("vector", DataType::FixedSizeList(vec_trait, embedding_size), true),
            Field::new("window_text", DataType::Utf8, true),
            Field::new("window_text_hash", DataType::Utf8, true),
            Field::new("file_path", DataType::Utf8, true),
            Field::new("start_line", DataType::UInt64, true),
            Field::new("end_line", DataType::UInt64, true),
            Field::new("time_added", DataType::UInt64, true),
            Field::new("model_name", DataType::Utf8, true),
        ]));
        let table = match database.open_table("data").await {
            Ok(table) => { table }
            Err(_) => {
                let batches_iter = RecordBatchIterator::new(vec![].into_iter().map(Ok), schema.clone());
                database.create_table("data", batches_iter, Option::from(WriteParams::default())).await.unwrap()
            }
        };

        let hashes_cache: Vec<String> = table_record_batch(&schema, &table).await
            .column_by_name("window_text_hash")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("ArrayRef must be a StringArray")
            .iter()
            .map(|maybe_string| maybe_string.map_or_else(String::new, |str_val| str_val.to_string()))
            .collect();
        VecDBHandler {
            database,
            schema,
            table,
            hashes_cache: HashSet::from_iter(hashes_cache),
            embedding_size,
        }
    }

    pub async fn size(&self) -> usize {
        self.table.count_rows().await.unwrap()
    }

    pub async fn add_or_update(&mut self, records: Vec<Record>) -> vectordb::error::Result<()> {
        fn make_emb_data(records: &Vec<Record>, embedding_size: i32) -> ArrayData {
            let vec_trait = Arc::new(Field::new("item", DataType::Float32, true));
            let mut emb_builder: Vec<f32> = vec![];

            for record in records {
                emb_builder.append(&mut record.vector.clone());
            }

            let emb_data = ArrayData::builder(DataType::Float32)
                .add_buffer(Buffer::from_vec(emb_builder))
                .len(records.len() * embedding_size as usize)
                .build()
                .unwrap();

            return ArrayData::builder(DataType::FixedSizeList(vec_trait.clone(), embedding_size))
                .len(records.len())
                .add_child_data(emb_data.clone())
                .build()
                .unwrap();
        }

        if records.is_empty() {
            return Ok(());
        }

        let vectors: ArrayData = make_emb_data(&records, self.embedding_size);
        let window_texts: Vec<String> = records.iter().map(|x| x.window_text.clone()).collect();
        let window_text_hashes: Vec<String> = records.iter().map(|x| x.window_text_hash.clone()).collect();
        let file_paths: Vec<String> = records.iter().map(|x| x.file_path.to_str().unwrap().to_string()).collect();
        let start_lines: Vec<u64> = records.iter().map(|x| x.start_line).collect();
        let end_lines: Vec<u64> = records.iter().map(|x| x.end_line).collect();
        let time_adds: Vec<u64> = records.iter().map(|x| x.time_added.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).collect();
        let model_names: Vec<String> = records.iter().map(|x| x.model_name.clone()).collect();
        let batches_iter = RecordBatchIterator::new(
            vec![RecordBatch::try_new(
                self.schema.clone(),
                vec![
                    Arc::new(FixedSizeListArray::from(vectors)),
                    Arc::new(StringArray::from(window_texts)),
                    Arc::new(StringArray::from(window_text_hashes.clone())),
                    Arc::new(StringArray::from(file_paths)),
                    Arc::new(UInt64Array::from(start_lines)),
                    Arc::new(UInt64Array::from(end_lines)),
                    Arc::new(UInt64Array::from(time_adds)),
                    Arc::new(StringArray::from(model_names)),
                ],
            )],
            self.schema.clone(),
        );

        let res = self.table.add(
            batches_iter, Option::from(WriteParams {
                mode: WriteMode::Append,
                ..Default::default()
            })
        );
        self.hashes_cache.extend(window_text_hashes);
        res.await
    }

    pub async fn remove(&mut self, file_path: &PathBuf) {
        self.table.delete(
            format!("(file_path = {})", file_path.to_str().unwrap()).as_str()
        ).await.unwrap();
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.hashes_cache.contains(hash)
    }

    pub async fn search(&self, embedding: Vec<f32>, top_n: usize) -> vectordb::error::Result<Vec<Record>> {
        let query = self.table
            .search(Float32Array::from(embedding))
            .limit(top_n)
            .use_index(true)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let record_batch = concat_batches(&self.schema, &query)?;
        let res: Result<Vec<Record>, _> = (0..record_batch.num_rows()).map(|idx| {
            Ok(Record {
                vector: as_primitive_array::<Float32Type>(
                    &as_fixed_size_list_array(record_batch.column_by_name("vector").unwrap())
                        .iter()
                        .map(|x| x.unwrap())
                        .collect::<Vec<_>>()[idx]
                )
                    .iter()
                    .map(|x| x.unwrap()).collect(),
                window_text: as_string_array(record_batch.column_by_name("window_text")
                    .expect("Missing column 'window_text'"))
                    .value(idx)
                    .to_string(),
                window_text_hash: as_string_array(record_batch.column_by_name("window_text_hash")
                    .expect("Missing column 'window_text_hash'"))
                    .value(idx)
                    .to_string(),
                file_path: PathBuf::from(as_string_array(record_batch.column_by_name("file_path")
                    .expect("Missing column 'file_path'"))
                    .value(idx)
                    .to_string()),
                start_line: as_primitive_array::<UInt64Type>(record_batch.column_by_name("start_line")
                    .expect("Missing column 'start_line'"))
                    .value(idx),
                end_line: as_primitive_array::<UInt64Type>(record_batch.column_by_name("end_line")
                    .expect("Missing column 'end_line'"))
                    .value(idx),
                time_added: std::time::UNIX_EPOCH + std::time::Duration::from_secs(
                    as_primitive_array::<UInt64Type>(
                        record_batch.column_by_name("time_added")
                            .expect("Missing column 'time_added'"))
                        .value(idx)
                ),
                model_name: as_string_array(record_batch.column_by_name("model_name")
                    .expect("Missing column 'model_name'"))
                    .value(idx)
                    .to_string(),
                score: 1.0,  // TODO: investigate if we can really take this value from the vectordb
            })
        }).collect();
        res
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio;

    use super::*;

    #[tokio::test]
    async fn test_init() {
        let temp_dir = tempdir().unwrap();
        let embedding_size = 2;
        let mut handler = VecDBHandler::init(
            temp_dir.path().to_path_buf(),
            embedding_size,
        ).await;
        assert_eq!(handler.size().await, 0);
    }

    #[tokio::test]
    async fn test_add_or_update() {
        let temp_dir = tempdir().unwrap();
        let embedding_size = 2;
        let mut handler = VecDBHandler::init(
            temp_dir.path().to_path_buf(),
            embedding_size,
        ).await;
        let expected_size = 1;

        // Prepare a sample record
        let records = vec![
            Record {
                vector: vec![1.0, 2.0], // Example values
                window_text: "sample text".to_string(),
                window_text_hash: "hash1".to_string(),
                file_path: PathBuf::from("/path/to/file"),
                start_line: 1,
                end_line: 2,
                time_added: SystemTime::now(),
                model_name: "model1".to_string(),
                score: 1.0,
            },
        ];

        // Call add_or_update
        handler.add_or_update(records).await.unwrap();

        // Validate the records
        assert_eq!(handler.size().await, expected_size);
    }

    #[tokio::test]
    async fn test_search() {
        let temp_dir = tempdir().unwrap();
        let embedding_size = 4;
        let mut handler = VecDBHandler::init(
            temp_dir.path().to_path_buf(),
            embedding_size,
        ).await;
        let top_n = 1;

        // Add a record to the database
        let time_added = SystemTime::now();
        let records = vec![
            Record {
                vector: vec![1.0, 2.0, 3.0, 4.0],
                window_text: "test text".to_string(),
                window_text_hash: "hash2".to_string(),
                file_path: PathBuf::from("/path/to/another/file"),
                start_line: 3,
                end_line: 4,
                time_added: time_added,
                model_name: "model2".to_string(),
                score: 1.0,
            },
        ];
        handler.add_or_update(records).await.unwrap();

        let query_embedding = vec![1.0, 2.0, 3.0, 4.0];
        let results = handler.search(query_embedding, top_n).await.unwrap();

        assert!(!results.is_empty());
        assert_eq!(results[0].window_text, "test text");
        assert_eq!(results[0].window_text_hash, "hash2");
        assert_eq!(results[0].file_path, PathBuf::from("/path/to/another/file"));
        assert_eq!(results[0].start_line, 3);
        assert_eq!(results[0].end_line, 4);
        assert_eq!(results[0].model_name, "model2");
        assert_eq!(results[0].score, 1.0);
    }
}
