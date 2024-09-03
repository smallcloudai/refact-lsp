use hashbrown::HashMap;
use std::cmp::Ordering;
use std::path::PathBuf;
use log::warn;
use uuid::Uuid;

use crate::ast::treesitter::ast_instance_structs::{AstSymbolInstanceRc, FunctionDeclaration};
use crate::ast::treesitter::language_id::LanguageId;
use crate::ast::treesitter::structs::SymbolType;

pub struct FilePathIterator {
    paths: Vec<PathBuf>,
    index: usize, // Current position in the list
}

impl FilePathIterator {
    fn new(start_path: PathBuf, mut all_paths: Vec<PathBuf>) -> FilePathIterator {
        all_paths.sort_by(|a, b| {
            FilePathIterator::compare_paths(&start_path, a, b)
        });

        FilePathIterator {
            paths: all_paths,
            index: 0,
        }
    }

    pub fn compare_paths(start_path: &PathBuf, a: &PathBuf, b: &PathBuf) -> Ordering {
        let start_components: Vec<_> = start_path.components().collect();
        let a_components: Vec<_> = a.components().collect();
        let b_components: Vec<_> = b.components().collect();

        let a_distance = a_components
            .iter()
            .zip(&start_components)
            .take_while(|(a, b)| a == b)
            .count();
        let b_distance = b_components.iter()
            .zip(&start_components)
            .take_while(|(a, b)| a == b)
            .count();

        a_distance.cmp(&b_distance).reverse()
    }
}

impl Iterator for FilePathIterator {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.paths.len() {
            let path = self.paths[self.index].clone();
            self.index += 1;
            Some(path)
        } else {
            None
        }
    }
}

fn is_self_ref(symbol: &AstSymbolInstanceRc) -> bool {
    match symbol.borrow().language() {
        LanguageId::Cpp |
        LanguageId::CSharp |
        LanguageId::Kotlin |
        LanguageId::Java |
        LanguageId::JavaScript |
        LanguageId::Php |
        LanguageId::TypeScript => symbol.borrow().name() == "this",

        LanguageId::Python |
        LanguageId::Ruby |
        LanguageId::Rust => symbol.borrow().name() == "self",

        _ => false
    }
}

fn get_type_by_decl_symbol(
    symbol: &AstSymbolInstanceRc,
    guid_by_symbols: &HashMap<Uuid, AstSymbolInstanceRc>,
) -> Option<AstSymbolInstanceRc> {
    match symbol.borrow().symbol_type() {
        SymbolType::StructDeclaration => {
            Some(symbol.clone())
        }
        SymbolType::FunctionDeclaration => {
            symbol
                .borrow()
                .as_any()
                .downcast_ref::<FunctionDeclaration>()
                .map(|x| x.return_type.as_ref())
                .flatten()
                .map(|x| x.guid.as_ref())
                .flatten()
                .map(|x| guid_by_symbols.get(x))
                .flatten()
                .cloned()
        }
        SymbolType::VariableDefinition | SymbolType::ClassFieldDeclaration => {
            symbol
                .borrow()
                .types()
                .iter()
                .filter_map(|t| t.guid)
                .filter_map(|g| guid_by_symbols.get(&g))
                .next()
                .cloned()
        }
        _ => None
    }
}

pub fn find_decl_by_caller_guid(
    symbol: &AstSymbolInstanceRc,
    guid_by_symbols: &HashMap<Uuid, AstSymbolInstanceRc>,
    search_by_caller_var_index: &HashMap<(Uuid, String), AstSymbolInstanceRc>,
    search_by_caller_func_index: &HashMap<(Uuid, String), AstSymbolInstanceRc>,
) -> Option<(Option<AstSymbolInstanceRc>, Option<AstSymbolInstanceRc>)> {
    let search_symbol_indexes: Vec<&HashMap<(Uuid, String), AstSymbolInstanceRc>> = if !symbol.borrow().is_error() {
        match symbol.borrow().symbol_type() {
            SymbolType::VariableUsage => { vec![search_by_caller_var_index] }
            SymbolType::FunctionCall => { vec![search_by_caller_func_index] }
            _ => {
                return None; 
            }
        }
    } else {
        vec![
            search_by_caller_var_index,
            search_by_caller_func_index
        ]
    };
    let caller_symbol = match symbol
        .borrow()
        .get_caller_guid()
        .map(|x| guid_by_symbols.get(&x))
        .flatten() {
        Some(s) => s,
        None => {
            return None 
        }
    };
    let search_request = if let Some(caller_linked_decl_type) = caller_symbol.borrow().get_linked_decl_type() {
        let type_symbol = match caller_linked_decl_type
            .guid
            .map(|x| guid_by_symbols.get(&x))
            .flatten() {
            Some(s) => s,
            None => {
                return None 
            }
        };
        // TODO: name can be cut if the symbol is error, make 
        (type_symbol.borrow().guid().clone(), symbol.borrow().name().to_string())
    } else {
        // Caller type is not filled, skipping by now 
        // TODO: Later can try to go to the higher level caller, can be useful for Rust
        return None;
    };

    match search_symbol_indexes
        .iter()
        .filter_map(|index| index.get(&search_request))
        .next() {
        Some(decl_symbol) => {
            let decl_type = get_type_by_decl_symbol(decl_symbol, guid_by_symbols);
            Some((Some(decl_symbol.clone()), decl_type.clone()))
        }
        None => {
            None
        }
    }
}

fn find_decl_by_name_for_var_usage(
    symbol: &AstSymbolInstanceRc,
    path_by_symbols: &HashMap<PathBuf, Vec<AstSymbolInstanceRc>>,
    guid_by_symbols: &HashMap<Uuid, AstSymbolInstanceRc>,
    search_by_caller_var_index: &HashMap<(Uuid, String), AstSymbolInstanceRc>,
) -> Option<(Option<AstSymbolInstanceRc>, Option<AstSymbolInstanceRc>)> {
    let mut decl_symbol = None;
    let mut decl_type = None;

    // Strategy 1. Try to match `magic` name
    if is_self_ref(&symbol) {
        let mut parent_symbol = symbol
            .borrow()
            .parent_guid()
            .map(|x| guid_by_symbols.get(&x))
            .flatten();
        loop {
            if let Some(s) = parent_symbol {
                if s.borrow().symbol_type() == SymbolType::StructDeclaration {
                    decl_type = Some(s.clone());
                    break;
                } else {
                    parent_symbol = s
                        .borrow()
                        .parent_guid()
                        .map(|x| guid_by_symbols.get(&x))
                        .flatten();
                    continue;
                }
            } else {
                break;
            }
        }
    }

    // Strategy 2. Looking for the declaration symbol up until the first function declaration 
    if decl_type.is_none() && symbol.borrow().full_range().start_point.row > 0 {
        let mut file_symbols_by_lines: HashMap<usize, Vec<AstSymbolInstanceRc>> = HashMap::new();
        for symbol in path_by_symbols
            .get(symbol.borrow().file_path())
            .unwrap_or(&vec![])
            .iter() {
            file_symbols_by_lines
                .entry(symbol.borrow().full_range().start_point.row)
                .or_insert(vec![]).push(symbol.clone());
        }

        let mut function_decl_symbol = None;
        for i in (0..symbol.borrow().full_range().start_point.row - 1).rev() {
            if let Some(line_symbols) = file_symbols_by_lines.get(&i) {
                let function_decl_symbol_mb = line_symbols
                    .iter()
                    .filter(|x| x.borrow().symbol_type() == SymbolType::FunctionDeclaration)
                    .next()
                    .cloned();
                if function_decl_symbol_mb.is_some() {
                    function_decl_symbol = function_decl_symbol_mb;
                    break;
                }
                
                if decl_symbol.is_none() {
                    decl_symbol = line_symbols
                        .iter()
                        .filter(|x| x.borrow().symbol_type() == SymbolType::VariableDefinition
                            || x.borrow().symbol_type() == SymbolType::ClassFieldDeclaration)
                        .filter(|x| x.borrow().name() == symbol.borrow().name())
                        .next()
                        .cloned();
                }
            }
        }

        // Strategy 3. Looking for the declaration type in the function signature (if presents)
        if decl_symbol.is_none() && function_decl_symbol.is_some() {
            for arg in function_decl_symbol
                .expect("checked above").borrow()
                .as_any()
                .downcast_ref::<FunctionDeclaration>()
                .expect("checked above").args.iter() {
                if arg.name == symbol.borrow().name() {
                    decl_type = arg
                        .type_.as_ref()
                        .map(|x| x.guid.as_ref())
                        .flatten()
                        .map(|x| guid_by_symbols.get(x))
                        .flatten()
                        .cloned();
                }
            }
        }
    }

    // Strategy 4. Looking for the declaration symbol in the class
    if decl_symbol.is_none() && decl_type.is_none() {
        let mut struct_decl_symbol_mb = symbol
            .borrow()
            .parent_guid()
            .map(|x| guid_by_symbols.get(&x))
            .flatten();
        loop {
            if let Some(s) = struct_decl_symbol_mb {
                if s.borrow().symbol_type() == SymbolType::StructDeclaration {
                    break;
                } else {
                    struct_decl_symbol_mb = s
                        .borrow()
                        .parent_guid()
                        .map(|x| guid_by_symbols.get(&x))
                        .flatten();
                    continue;
                }
            } else {
                break;
            }
        }
        decl_symbol = struct_decl_symbol_mb
            .map(|x| search_by_caller_var_index.get(&(
                x.borrow().guid().clone(), symbol.borrow().name().to_string()
            )))
            .flatten()
            .cloned();
    }

    if decl_type.is_none() {
        decl_type = if let Some(s) = decl_symbol.as_ref() {
            get_type_by_decl_symbol(&s, &guid_by_symbols)
        } else {
            None
        }
    }

    Some((decl_symbol.clone(), decl_type.clone()))
}

fn find_decl_by_name_for_fn_usage(
    symbol: &AstSymbolInstanceRc,
    guid_by_symbols: &HashMap<Uuid, AstSymbolInstanceRc>,
    declaration_symbols_by_name: &HashMap<String, Vec<AstSymbolInstanceRc>>,
) -> Option<(Option<AstSymbolInstanceRc>, Option<AstSymbolInstanceRc>)> {
    let mut decl_symbol: Option<AstSymbolInstanceRc> = None;
    let mut decl_type: Option<AstSymbolInstanceRc> = None;
    
    let file_path = symbol.borrow().file_path().clone();
    let binding = vec![];
    let symbols = declaration_symbols_by_name
        .get(symbol.borrow().name())
        .unwrap_or(&binding);

    let func_decl_symbols = symbols
        .iter()
        .filter(|x| x.borrow().symbol_type() == SymbolType::FunctionDeclaration)
        .cloned()
        .collect::<Vec<_>>();
    let struct_decl_symbols = symbols
        .iter()
        .filter(|x| x.borrow().symbol_type() == SymbolType::StructDeclaration)
        .cloned()
        .collect::<Vec<_>>();

    // Strategy 1. Looking for the function declaration in the current file
    decl_symbol = func_decl_symbols
        .iter()
        .filter(|x| *x.borrow().file_path() == file_path)
        .next()
        .cloned();

    // Strategy 2. Looking for the type declaration (considering symbol as a constructor call) in the current file
    if decl_symbol.is_none() {
        decl_type = struct_decl_symbols
            .iter()
            .filter(|x| *x.borrow().file_path() == file_path)
            .next()
            .cloned();
    }

    if decl_symbol.is_none() && decl_type.is_none() {
        // Strategy 3. Looking for the function declaration in the whole project
        decl_symbol = func_decl_symbols
            .iter()
            .filter(|x| *x.borrow().file_path() != file_path)
            .min_by(|a, b| {
                // TODO: use import-based distance  
                let path_a = a.borrow().file_path().clone();
                let path_b = b.borrow().file_path().clone();
                FilePathIterator::compare_paths(&file_path, &path_a, &path_b)
            })
            .cloned();
    }

    if decl_symbol.is_none() && decl_type.is_none() {
        // Strategy 4. Looking for the type declaration in the whole project
        decl_type = struct_decl_symbols
            .iter()
            .filter(|x| *x.borrow().file_path() != file_path)
            .min_by(|a, b| {
                // TODO: use import-based distance  
                let path_a = a.borrow().file_path().clone();
                let path_b = b.borrow().file_path().clone();
                FilePathIterator::compare_paths(&file_path, &path_a, &path_b)
            })
            .cloned();
    }
    
    if decl_type.is_none() {
        decl_type = if let Some(s) = decl_symbol.as_ref() {
            get_type_by_decl_symbol(&s, &guid_by_symbols)
        } else {
            None
        }
    } 

    Some((decl_symbol.clone(), decl_type.clone()))
}

pub fn find_decl_by_name(
    symbol: &AstSymbolInstanceRc,
    path_by_symbols: &HashMap<PathBuf, Vec<AstSymbolInstanceRc>>,
    guid_by_symbols: &HashMap<Uuid, AstSymbolInstanceRc>,
    search_by_caller_var_index: &HashMap<(Uuid, String), AstSymbolInstanceRc>,
    declaration_symbols_by_name: &HashMap<String, Vec<AstSymbolInstanceRc>>,
) -> Option<(Option<AstSymbolInstanceRc>, Option<AstSymbolInstanceRc>)> {
    if symbol.borrow().is_error() {
        match find_decl_by_name_for_var_usage(
            symbol, path_by_symbols, guid_by_symbols, search_by_caller_var_index,
        ) {
            Some(res) => {
                if res.0.is_none() && res.1.is_none() {
                    find_decl_by_name_for_fn_usage(
                        symbol, guid_by_symbols, declaration_symbols_by_name,
                    )
                } else {
                    Some(res)
                }
            }
            None => None
        }
    } else {
        if symbol.borrow().symbol_type() == SymbolType::VariableUsage {
            find_decl_by_name_for_var_usage(
                symbol, path_by_symbols, guid_by_symbols, search_by_caller_var_index,
            )
        } else {
            find_decl_by_name_for_fn_usage(
                symbol, guid_by_symbols, declaration_symbols_by_name,
            )
        }
    }
}
