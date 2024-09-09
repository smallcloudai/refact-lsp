use sled::{Db, IVec};
use uuid::Uuid;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::task;
use crate::ast::alt_minimalistic::{AltIndex, AltState, AltDefinition};
use crate::ast::alt_parse_anything::{parse_anything_and_add_file_path, filesystem_path_to_double_colon_path};
use serde_cbor;

async fn alt_index_init() -> Arc<AMutex<AltIndex>>
{
    // # let config = sled::Config::new().temporary(true);
    // # let db = config.open()?;
    let db: Arc<Db> = Arc::new(task::spawn_blocking(|| sled::open("/tmp/my_db.sled").unwrap()).await.unwrap());
    db.clear().unwrap();
    // db.open_tree(b"unprocessed items").unwrap();
    let altindex = AltIndex {
        sleddb: db,
    };
    Arc::new(AMutex::new(altindex))
}

async fn doc_add(altindex: Arc<AMutex<AltIndex>>, cpath: &String, text: &String)
{
    let definitions = parse_anything_and_add_file_path(cpath, text);
    let db = altindex.lock().await.sleddb.clone();
    let mut batch = sled::Batch::default();
    for definition in definitions.values() {
        let serialized = serde_cbor::to_vec(&definition).unwrap();
        let official_path = definition.official_path.join("::");
        let d_key = format!("d/{}", official_path);
        batch.insert(d_key.as_bytes(), serialized);
        let mut path_parts: Vec<&str> = definition.official_path.iter().map(|s| s.as_str()).collect();
        while !path_parts.is_empty() {
            let c_key = format!("c/{} ⚡ {}", path_parts.join("::"), official_path);
            batch.insert(c_key.as_bytes(), b"huu");
            path_parts.remove(0);
        }
    }
    db.apply_batch(batch).unwrap();
}

async fn doc_remove(altindex: Arc<AMutex<AltIndex>>, cpath: &String)
{
    let to_delete_prefix = filesystem_path_to_double_colon_path(cpath);
    let official_path = format!("d/{}", to_delete_prefix.join("::"));
    let db = altindex.lock().await.sleddb.clone();
    let mut batch = sled::Batch::default();
    let mut iter = db.scan_prefix(official_path);
    while let Some(Ok((key, value))) = iter.next() {
        let d_key_b = key.clone();
        if let Ok(definition) = serde_cbor::from_slice::<AltDefinition>(&value) {
            let mut path_parts: Vec<&str> = definition.official_path.iter().map(|s| s.as_str()).collect();
            while !path_parts.is_empty() {
                let c_key = format!("c/{} ⚡ {}", path_parts.join("::"), definition.official_path.join("::"));
                batch.remove(c_key.as_bytes());
                path_parts.remove(0);
            }
        }
        batch.remove(&d_key_b);
    }
    db.apply_batch(batch).unwrap();
}

async fn connect_everything(altindex: Arc<AMutex<AltIndex>>)
{
}

async fn dump_database(altindex: Arc<AMutex<AltIndex>>)
{
    let db = altindex.lock().await.sleddb.clone();
    println!("\nsled has {} reconds", db.len());
    let iter = db.iter();
    for item in iter {
        let (key, value) = item.unwrap();
        let key_string = String::from_utf8(key.to_vec()).unwrap(); // Convert key to String
        if key_string.starts_with("d/") { // Check if the key is a d_key
            match serde_cbor::from_slice::<AltDefinition>(&value) {
                Ok(definition) => println!("{}\n{:?}", key_string, definition),
                Err(e) => println!("Failed to deserialize value at {}: {:?}", key_string, e),
            }
        }
        if key_string.starts_with("c/") {
            println!("{}", key_string);
        }
    }
}
// async fn doc_symbols(altindex: Arc<AMutex<AltState>>, cpath: &String) -> Vec<Arc<AltDefinition>>
// {
// }

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn read_file(file_path: &str) -> String {
        fs::read_to_string(file_path).expect("Unable to read file")
    }

    #[tokio::test]
    async fn test_alt_db() {
        let altindex = alt_index_init().await;

        let cpp_library_path = "src/ast/alt_testsuite/cpp_goat_library.h";
        let cpp_library_text = read_file(cpp_library_path);
        doc_add(altindex.clone(), &cpp_library_path.to_string(), &cpp_library_text).await;

        let cpp_main_path = "src/ast/alt_testsuite/cpp_goat_main.cpp";
        let cpp_main_text = read_file(cpp_main_path);
        doc_add(altindex.clone(), &cpp_main_path.to_string(), &cpp_main_text).await;

        connect_everything(altindex.clone()).await;

        dump_database(altindex.clone()).await;

        doc_remove(altindex.clone(), &cpp_library_path.to_string()).await;
        doc_remove(altindex.clone(), &cpp_main_path.to_string()).await;

        dump_database(altindex.clone()).await;
    }
}
