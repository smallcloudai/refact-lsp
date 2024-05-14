use std::sync::Arc;
use std::collections::HashMap;
use std::cell::RefCell;
use uuid::Uuid;
use crate::ast::structs::FileASTMarkup;
use crate::ast::treesitter::ast_instance_structs::SymbolInformation;
use crate::files_in_workspace::Document;


pub fn lowlevel_file_markup(
    doc: &Document,
    symbols: &Vec<SymbolInformation>,
) -> Result<FileASTMarkup, String> {
    let t0 = std::time::Instant::now();
    assert!(doc.text.is_some());
    let mut symbols4export: Vec<Arc<RefCell<SymbolInformation>>> = symbols.iter().map(|s| {
        Arc::new(RefCell::new(s.clone()))
    }).collect();
    let guid_to_symbol: HashMap<Uuid, Arc<RefCell<SymbolInformation>>> = symbols4export.iter().map(
        |s| (s.borrow().guid.clone(), s.clone())
    ).collect();
    fn recursive_path_of_guid(guid_to_symbol: &HashMap<Uuid, Arc<RefCell<SymbolInformation>>>, guid: &Uuid) -> String
    {
        return match guid_to_symbol.get(guid) {
            Some(x) => {
                let pname = if !x.borrow().name.is_empty() { x.borrow().name.clone() } else { x.borrow().guid.to_string()[..8].to_string() };
                let pp = recursive_path_of_guid(&guid_to_symbol, &x.borrow().parent_guid);
                format!("{}::{}", pp, pname)
            }
            None => {
                // FIXME:
                // info!("parent_guid {} not found, maybe outside of this file", guid);
                "UNK".to_string()
            }
        };
    }
    for s in symbols4export.iter_mut() {
        let symbol_path = recursive_path_of_guid(&guid_to_symbol, &s.borrow().guid);
        s.borrow_mut().symbol_path = symbol_path.clone();
    }
    // longer symbol path at the bottom => parent always higher than children
    symbols4export.sort_by(|a, b| {
        a.borrow().symbol_path.len().cmp(&b.borrow().symbol_path.len())
    });
    let x = FileASTMarkup {
        file_path: doc.path.clone(),
        file_content: doc.text.as_ref().unwrap().to_string(),
        symbols_sorted_by_path_len: symbols4export.iter().map(|s| {
            s.borrow().clone()
        }).collect(),
    };
    tracing::info!("file_markup {:>4} symbols in {:.3}ms for {}",
        x.symbols_sorted_by_path_len.len(),
        t0.elapsed().as_secs_f32(),
        crate::nicer_logs::last_n_chars(&doc.path.to_string_lossy().to_string(),
        30));
    Ok(x)
}

