use std::any::Any;
use std::fmt::{Debug, Formatter};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

use arrow::array::ArrayData;
use arrow::buffer::Buffer;
use arrow_array::{ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_array::array::Array;
use arrow_array::cast::{as_fixed_size_list_array, as_primitive_array, as_string_array};
use arrow_array::types::Float32Type;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use async_once::AsyncOnce;
use futures_util::{StreamExt, TryStreamExt};
use hyper::header::CONTENT_TYPE;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use structopt::lazy_static::lazy_static;
use vectordb::database::Database;
use vectordb::table::Table;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Record {
    pub vector: Vec<f32>,
    pub text: String
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VecDBPostFiles {
    pub name: String,
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VecDBPost {
    pub model: String,
    pub texts: Vec<VecDBPostFiles>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VecDBDataEmbeddingPost {
    pub name: String,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VecDBEmbeddingPost {
    pub data: Vec<VecDBPostFiles>,
}


pub async fn get_embeddings(data: VecDBPost, client: Client) -> Vec<Record> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json").unwrap());
    let req = client.post("http://localhost:8008/v1/embeddings")
        .headers(headers)
        .body(data.to_string())
        .send()
        .await;
}


pub type VecDBHandlerRef = Arc<RwLock<VecDBHandler>>;

static TABLE_NAME: &str = "data";

impl Debug for VecDBHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "VecDBHandler: {:?}", self.database.type_id())
    }
}

pub struct VecDBHandler {
    database: Database,
    schema: SchemaRef,
    table_name: String
}

impl VecDBHandler {
    pub async fn init(cache_dir: &PathBuf) -> VecDBHandler {
        let conn = Database::connect(cache_dir.join("vecdb").to_str().unwrap()).await.unwrap();
        let vec_trait = Arc::new(Field::new("item", DataType::Float32, false));
        let schema = Arc::new(Schema::new(vec![
            Field::new("vector", DataType::FixedSizeList(vec_trait, 2), false),
            Field::new("text", DataType::Utf8, false)
        ]));
        VecDBHandler {
            database: conn,
            schema,
            table_name: TABLE_NAME.to_string(),
        }
    }
    pub async fn get_table(&self) -> Table {
        let table = self.database.open_table(TABLE_NAME).await;
        match table {
            Ok(table) => { table }
            Err(_) => {
                let empty_batch = RecordBatch::new_empty(self.schema.clone());
                let batches: Vec<RecordBatch> = vec![empty_batch.clone()];
                let reader = RecordBatchIterator::new(
                    batches.into_iter().map(Ok), empty_batch.schema());
                self.database.create_table(TABLE_NAME, reader, None).await.unwrap()
            }
        }
    }

    pub async fn find(&self, embedding: Vec<f32>) -> vectordb::error::Result<Vec<Record>> {
        let table = self.get_table().await;
        let query = table.search(Float32Array::from(embedding))
            .limit(3);
        let mut res = query.execute().await.unwrap().try_collect::<Vec<_>>().await.unwrap();
        let batch = &res[0];
        let column_text = batch.column_by_name("text").unwrap();
        let texts: Vec<String> = as_string_array(column_text.as_ref()).iter().map(|x| x.unwrap().to_string()).collect();
        let column_emb = batch.column_by_name("vector").unwrap();
        let vectors: Vec<ArrayRef> = as_fixed_size_list_array(column_emb.as_ref()).iter().map(|x| x.unwrap()).collect();

        // let dist_emb = batch.column_by_name("_distance").unwrap();
        // let vectorss: Vec<f32> = as_primitive_array::<Float32Type>(dist_emb.as_ref()).iter().map(|x| x.unwrap()).collect();

        let mut res = vec![];
        for idx in 0..batch.num_rows() {
            let vector = as_primitive_array::<Float32Type>(vectors[idx].as_ref()).iter().map(|x| x.unwrap()).collect();
            let text = &texts[idx];
            res.push(Record {
                vector: vector,
                text: text.clone(),
            })
        }
        Ok(res)
    }

    pub async fn add(&self, records: Vec<Record>) -> vectordb::error::Result<()> {
        let mut batches: Vec<RecordBatch> = vec![];
        let vec_trait = Arc::new(Field::new("item", DataType::Float32, false));
        for chunked_records in records.chunks(8) {
            let mut value_builder: Vec<f32> = vec![];
            let mut texts: Vec<String> = vec![];
            for record in chunked_records {
                value_builder.append(&mut record.vector.clone());
                texts.push(record.text.clone());
            }

            let value_data = ArrayData::builder(DataType::Float32)
                .add_buffer(Buffer::from_vec(value_builder))
                .len(chunked_records.len() * 2)
                .build()
                .unwrap();
            let list_data = ArrayData::builder(DataType::FixedSizeList(vec_trait.clone(), 2))
                .len(chunked_records.len())
                .add_child_data(value_data.clone())
                .build()
                .unwrap();

            let record_batch = RecordBatch::try_new(
                self.schema.clone(),
                vec![
                    Arc::new(FixedSizeListArray::from(list_data)),
                    Arc::new(StringArray::from(texts))
                ]
            ).unwrap();
            batches.push(record_batch);
        }
        let reader = RecordBatchIterator::new(
            batches.into_iter().map(Ok), self.schema.clone());

        let mut table = self.get_table().await;

        table.add(reader, None).await
    }
}