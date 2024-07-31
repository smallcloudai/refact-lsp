curl http://127.0.0.1:8001/v1/subchat -k \
  -H 'Content-Type: application/json' \
  -d '{
  "model_name": "gpt-4o-mini",
  "messages": [
    {"role": "user", "content": "Check out definition of Frog and summarize in 10 words"}
  ],
  "tools_turn_on": ["definition"],
  "wrap_up_depth": 2,
  "wrap_up_tokens_cnt": 8000
}'

# struct SubChatPost {
#     model_name: String,
#     messages: Vec<ChatMessage>,
#     depth: usize,
#     #[serde(default)]
#     tools: Option<Vec<Value>>,
#     #[serde(default)]
#     tool_choice: Option<String>,
#     #[serde(default)]
#     wrap_up_tokens_cnt: Option<usize>,
# }

