use schemars::{schema_for, JsonSchema};


pub fn json_schema<T: JsonSchema>() -> Result<serde_json::Value, String> {
    let schema = schema_for!(T);
    let json_schema = serde_json::to_value(&schema).map_err(|e| e.to_string())?;
    Ok(json_schema)
}

pub trait Integration: Send + Sync + Sized {
    fn new_from_yaml(value: &serde_yaml::Value) -> Result<Self, String>;
    fn to_json(&self) -> Result<serde_json::Value, String>;
    fn to_schema_json() -> Result<serde_json::Value, String>;
}
