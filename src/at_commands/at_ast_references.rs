use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Mutex as AMutex;

use crate::ast::structs::FileReferencesResult;
use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::at_params::AtParamFilePath;
use crate::call_validation::{ChatMessage, SimplifiedSymbolDeclaration};

fn results2message(result: &FileReferencesResult) -> ChatMessage {
    let simplified_symbols: Vec<SimplifiedSymbolDeclaration> = result.symbols.iter().map(|x| {
        let path = format!("{:?}::", result.file_path).to_string();
        SimplifiedSymbolDeclaration {
            symbol_path: x.meta_path.replace(path.as_str(), ""),
            symbol_type: format!("{:?}", x.symbol_type),
            line1: x.definition_info.range.start_point.row,
            line2: x.definition_info.range.end_point.row,

        }
    }).collect();
    ChatMessage {
        role: "context_file".to_string(),
        content: json!(simplified_symbols).to_string(),
    }
}

pub struct AtAstReferences {
    pub name: String,
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtAstReferences {
    pub fn new() -> Self {
        AtAstReferences {
            name: "@ast_reference".to_string(),
            params: vec![
                Arc::new(AMutex::new(AtParamFilePath::new()))
            ],
        }
    }
}

#[async_trait]
impl AtCommand for AtAstReferences {
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

    async fn can_execute(&self, args: &Vec<String>, context: &AtCommandsContext) -> bool {
        if self.are_args_valid(args, context).await.iter().any(|&x| x == false) || args.len() != self.params.len() {
            return false;
        }
        return true;
    }

    async fn execute(&self, _query: &String, args: &Vec<String>, _top_n: usize, context: &AtCommandsContext) -> Result<ChatMessage, String> {
        let can_execute = self.can_execute(args, context).await;
        if !can_execute {
            return Err("incorrect arguments".to_string());
        }
        let file_path = match args.get(0) {
            Some(x) => x,
            None => return Err("no file path".to_string()),
        };

        let binding = context.global_context.read().await;
        let x = match *binding.ast_module.lock().await {
            Some(ref ast) => {
                match ast.get_file_references(PathBuf::from(file_path)).await {
                    Ok(res) => Ok(results2message(&res)),
                    Err(err) => Err(err)
                }
            }
            None => Err("Ast module is not available".to_string())
        }; x
    }
}
