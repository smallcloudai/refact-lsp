use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex as AMutex;

use crate::ast::structs::AstQuerySearchResult;
use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::at_params::AtParamSymbolPathQuery;
use crate::call_validation::{ChatMessage, ContextFile};
use tracing::info;


#[derive(Debug, Serialize, Deserialize, Clone)]
struct SimplifiedSymbolDeclarationStruct {
    pub symbol_path: String,
    pub symbol_type: String,
    pub line1: usize,
    pub line2: usize,
}

async fn results2message(result: &AstQuerySearchResult) -> ChatMessage {
    info!("results2message {:?}", result);
    let mut symbols = vec![];
    for res in &result.search_results {
        let file_path: String = res.symbol_declaration.meta_path
            .split("::")
            .map(|x| x.to_string())
            .collect::<Vec<String>>()
            .first()
            .cloned()
            .unwrap_or("".to_string());
        let content = res.symbol_declaration.get_content().await.unwrap_or("".to_string());
        symbols.push(ContextFile {
            file_name: file_path,
            file_content: content,
            line1: res.symbol_declaration.definition_info.range.start_point.row as i32,
            line2: res.symbol_declaration.definition_info.range.end_point.row as i32,
            usefullness: res.sim_to_query
        });
    }
    ChatMessage {
        role: "context_file".to_string(),
        content: json!(symbols).to_string(),
    }
}

pub struct AtAstDefinition {
    pub name: String,
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtAstDefinition {
    pub fn new() -> Self {
        AtAstDefinition {
            name: "@ast_definition".to_string(),
            params: vec![
                Arc::new(AMutex::new(AtParamSymbolPathQuery::new()))
            ],
        }
    }
}

#[async_trait]
impl AtCommand for AtAstDefinition {
    fn name(&self) -> &String {
        &self.name
    }
    fn params(&self) -> &Vec<Arc<AMutex<dyn AtParam>>> {
        &self.params
    }
    async fn are_args_valid(&self, args: &Vec<String>, context: &AtCommandsContext) -> Vec<bool> {
        let mut results = Vec::new();
        for (arg, param) in args.iter().zip(self.params.iter()) {
            let param = param.lock().await;
            results.push(param.is_value_valid(arg, context).await);
        }
        results
    }

    async fn can_execute(&self, _: &Vec<String>, _: &AtCommandsContext) -> bool {
        return true;
    }

    async fn execute(&self, _query: &String, args: &Vec<String>, _top_n: usize, context: &AtCommandsContext, _parsed_args: &HashMap<String, String>) -> Result<ChatMessage, String> {
        let can_execute = self.can_execute(args, context).await;
        if !can_execute {
            return Err("incorrect arguments".to_string());
        }
        info!("execute @ast_definition {:?}", args);
        let symbol_path = match args.get(0) {
            Some(x) => x,
            None => return Err("no symbol path".to_string()),
        };
        let binding = context.global_context.read().await;
        let x = match *binding.ast_module.lock().await {
            Some(ref ast) => {
                match ast.search_declarations_by_symbol_path(symbol_path.clone(), 3).await {
                    Ok(res) => Ok(results2message(&res).await),
                    Err(err) => Err(err)
                }
            }
            None => Err("Ast module is not available".to_string())
        }; x
    }
}
