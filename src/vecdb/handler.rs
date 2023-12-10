use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::ArrayData;
use arrow::buffer::Buffer;
use arrow::compute::concat_batches;
use arrow_array::{FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray, UInt64Array};
use arrow_array::array::Array;
use arrow_array::cast::{as_fixed_size_list_array, as_primitive_array, as_string_array};
use arrow_array::types::{Float32Type, UInt64Type};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures_util::TryStreamExt;
use lance::dataset::{WriteMode, WriteParams};
use tempfile::{tempdir, TempDir};
use tokio::sync::Mutex;
use vectordb::database::Database;
use vectordb::table::Table;

use crate::vecdb::structs::{Record, SplitResult};

pub type VecDBHandlerRef = Arc<Mutex<VecDBHandler>>;

impl Debug for VecDBHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "VecDBHandler: {:?}", self.cache_database.type_id())
    }
}

pub struct VecDBHandler {
    cache_database: Database,
    data_database_temp_dir: TempDir,
    cache_table: Table,
    data_table: Table,
    schema: SchemaRef,
    data_table_hashes: HashSet<String>,
    hashes_cache: HashMap<String, Record>,
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

fn cosine_similarity(vec1: &Vec<f32>, vec2: &Vec<f32>) -> f32 {
    let dot_product: f32 = vec1.iter().zip(vec2).map(|(x, y)| x * y).sum();
    let magnitude_vec1: f32 = vec1.iter().map(|x| x.powi(2)).sum::<f32>().sqrt();
    let magnitude_vec2: f32 = vec2.iter().map(|x| x.powi(2)).sum::<f32>().sqrt();
    dot_product / (magnitude_vec1 * magnitude_vec2)
}

fn cosine_distance(vec1: &Vec<f32>, vec2: &Vec<f32>) -> f32 {
    1.0 - cosine_similarity(vec1, vec2)
}


impl VecDBHandler {
    pub async fn init(cache_dir: PathBuf, embedding_size: i32) -> VecDBHandler {
        let cache_database = Database::connect(cache_dir.join("refact_vecdb_cache").to_str().unwrap()).await.unwrap();
        let data_database_temp_dir = tempdir().unwrap();
        let temp_database = Database::connect(data_database_temp_dir.path().to_str().unwrap()).await.unwrap();
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
        let cache_table = match cache_database.open_table("data").await {
            Ok(table) => { table }
            Err(e) => {
                let batches_iter = RecordBatchIterator::new(vec![].into_iter().map(Ok), schema.clone());
                cache_database.create_table("data", batches_iter, Option::from(WriteParams::default())).await.unwrap()
            }
        };
        let batches_iter = RecordBatchIterator::new(vec![].into_iter().map(Ok), schema.clone());
        let data_table = temp_database.create_table("data", batches_iter, Option::from(WriteParams::default())).await.unwrap();

        let hashes_record_batch = table_record_batch(&schema, &cache_table).await;
        let maybe_hashes_cache_iter = VecDBHandler::parse_table_iter(hashes_record_batch, true, None);
        let hashes_cache: HashMap<String, Record> = match maybe_hashes_cache_iter {
            Ok(hashes_cache_iter) => {
                hashes_cache_iter.iter().map(|x| {
                    (x.window_text_hash.clone(), x.clone())
                }).collect()
            }
            Err(_) => HashMap::new()
        };

        VecDBHandler {
            cache_database,
            data_database_temp_dir,
            schema,
            cache_table,
            data_table,
            data_table_hashes: HashSet::new(),
            hashes_cache,
            embedding_size,
        }
    }

    pub async fn size(&self) -> usize { self.data_table.count_rows().await.unwrap() }

    pub async fn cache_size(&self) -> usize { self.cache_table.count_rows().await.unwrap() }

    pub async fn try_add_from_cache(&mut self, data: Vec<SplitResult>) -> Vec<SplitResult> {
        let mut found_records: Vec<Record> = Vec::new();
        let mut left_results: Vec<SplitResult> = Vec::new();

        for split_result in data {
            if self.hashes_cache.contains_key(&split_result.window_text_hash) {
                found_records.push(self.hashes_cache.get(&split_result.window_text_hash).unwrap().clone());
            } else {
                left_results.push(split_result);
            }
        }
        self.add_or_update(found_records, false).await.unwrap();
        left_results
    }

    pub async fn add_or_update(&mut self, records: Vec<Record>, add_to_cache: bool) -> vectordb::error::Result<()> {
        fn make_emb_data(records: &Vec<Record>, embedding_size: i32) -> ArrayData {
            let vec_trait = Arc::new(Field::new("item", DataType::Float32, true));
            let mut emb_builder: Vec<f32> = vec![];

            for record in records {
                emb_builder.append(&mut record.vector.clone().expect("No embedding is provided"));
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
        let data_batches_iter = RecordBatchIterator::new(
            vec![RecordBatch::try_new(
                self.schema.clone(),
                vec![
                    Arc::new(FixedSizeListArray::from(vectors.clone())),
                    Arc::new(StringArray::from(window_texts.clone())),
                    Arc::new(StringArray::from(window_text_hashes.clone())),
                    Arc::new(StringArray::from(file_paths.clone())),
                    Arc::new(UInt64Array::from(start_lines.clone())),
                    Arc::new(UInt64Array::from(end_lines.clone())),
                    Arc::new(UInt64Array::from(time_adds.clone())),
                    Arc::new(StringArray::from(model_names.clone())),
                ],
            )],
            self.schema.clone(),
        );
        let cache_batches_iter = RecordBatchIterator::new(
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

        let data_res = self.data_table.add(
            data_batches_iter, Option::from(WriteParams {
                mode: WriteMode::Append,
                ..Default::default()
            }),
        );

        let cache_res = self.cache_table.add(
            cache_batches_iter, Option::from(WriteParams {
                mode: WriteMode::Append,
                ..Default::default()
            }),
        );
        if add_to_cache {
            self.hashes_cache.extend(
                records.iter().map(|x| (x.window_text_hash.clone(), x.clone())).collect::<Vec<_>>()
            );
            cache_res.await.unwrap();
        }
        self.data_table_hashes.extend(window_text_hashes);
        data_res.await
    }

    pub async fn remove(&mut self, file_path: &PathBuf) {
        self.cache_table.delete(
            format!("(file_path = {})", file_path.to_str().unwrap()).as_str()
        ).await.unwrap();
        self.data_table.delete(
            format!("(file_path = {})", file_path.to_str().unwrap()).as_str()
        ).await.unwrap();
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.data_table_hashes.contains(hash)
    }

    fn parse_table_iter(
        record_batch: RecordBatch,
        include_embedding: bool,
        embedding_to_compare: Option<&Vec<f32>>,
    ) -> vectordb::error::Result<Vec<Record>> {
        (0..record_batch.num_rows()).map(|idx| {
            let gathered_vec = as_primitive_array::<Float32Type>(
                &as_fixed_size_list_array(record_batch.column_by_name("vector").unwrap())
                    .iter()
                    .map(|x| x.unwrap())
                    .collect::<Vec<_>>()[idx]
            )
                .iter()
                .map(|x| x.unwrap()).collect();
            let distance = match embedding_to_compare {
                None => { -1.0 }
                Some(embedding) => { cosine_distance(&embedding, &gathered_vec) }
            };
            let embedding = match include_embedding {
                true => Some(gathered_vec),
                false => None
            };

            Ok(Record {
                vector: embedding,
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
                distance,
            })
        }).collect()
    }

    pub async fn search(&self, embedding: Vec<f32>, top_n: usize) -> vectordb::error::Result<Vec<Record>> {
        let query = self.data_table
            .search(Float32Array::from(embedding.clone()))
            .limit(top_n)
            .use_index(true)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let record_batch = concat_batches(&self.schema, &query)?;
        VecDBHandler::parse_table_iter(record_batch, false, Some(&embedding))
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

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
                vector: Some(vec![1.0, 2.0]), // Example values
                window_text: "sample text".to_string(),
                window_text_hash: "hash1".to_string(),
                file_path: PathBuf::from("/path/to/file"),
                start_line: 1,
                end_line: 2,
                time_added: SystemTime::now(),
                model_name: "model1".to_string(),
                distance: 1.0,
            },
        ];

        // Call add_or_update
        handler.add_or_update(records, true).await.unwrap();

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

        let time_added = SystemTime::now();
        let records = vec![
            Record {
                vector: Some(vec![1.0, 2.0, 3.0, 4.0]),
                window_text: "test text".to_string(),
                window_text_hash: "hash2".to_string(),
                file_path: PathBuf::from("/path/to/another/file"),
                start_line: 3,
                end_line: 4,
                time_added: time_added,
                model_name: "model2".to_string(),
                distance: 1.0,
            },
        ];
        handler.add_or_update(records, true).await.unwrap();

        let query_embedding = vec![1.0, 2.0, 3.0, 4.0];
        let results = handler.search(query_embedding, top_n).await.unwrap();

        assert!(!results.is_empty());
        assert_eq!(results[0].window_text, "test text");
        assert_eq!(results[0].window_text_hash, "hash2");
        assert_eq!(results[0].file_path, PathBuf::from("/path/to/another/file"));
        assert_eq!(results[0].start_line, 3);
        assert_eq!(results[0].end_line, 4);
        assert_eq!(results[0].model_name, "model2");
        assert_eq!(results[0].distance, 1.0);
    }
}
