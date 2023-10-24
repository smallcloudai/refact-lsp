
pub struct HasVecdbResults {
    pub results: Vec<VecdbResult>,
    pub sent: bool,
}


        // let vdb_context_json = scratch.vecdb_context_json();
        // if vdb_context_json.len() > 0 {
        //     let new_result = json!({
        //         "choices": [{
        //             "delta": {
        //                 "content": vdb_context_json,
        //                 "role": "context"
        //             },
        //             "finish_reason": serde_json::Value::Null,
        //             "index": 0
        //         }],
        //         "created": json!(t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as f64 / 1000.0),
        //         "model": model_name.clone()
        //     });
        //     let value_str = format!("data: {}\n\n", serde_json::to_string(&new_result).unwrap());
        //     info!("yield: {:?}", value_str);
        //     yield Result::<_, String>::Ok(value_str);
        // }
