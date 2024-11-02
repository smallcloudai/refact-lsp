
pub mod db_chore;
pub mod db_chore_event;
pub mod db_cmessage;
pub mod db_cthread;
pub mod db_init;
pub mod db_schema_20241102;
pub mod db_structs;


pub fn merge_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                // yay, it's recursive!
                merge_json(a.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (a, b) => {
            *a = b.clone();
        }
    }
}

