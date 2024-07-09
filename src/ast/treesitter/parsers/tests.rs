use std::collections::{HashMap, HashSet};
use std::collections::VecDeque;
use std::path::PathBuf;

use itertools::Itertools;
use ropey::Rope;
use serde::{Deserialize, Serialize};
use similar::DiffableStr;
use uuid::Uuid;

use crate::ast::structs::FileASTMarkup;
use crate::ast::treesitter::ast_instance_structs::{AstSymbolInstance, AstSymbolInstanceArc, SymbolInformation};
use crate::ast::treesitter::language_id::LanguageId;
use crate::ast::treesitter::parsers::AstLanguageParser;
use crate::ast::treesitter::skeletonizer::make_formatter;
use crate::ast::treesitter::structs::SymbolType;
use crate::files_in_workspace::Document;

mod rust;
mod python;
mod java;
mod cpp;
mod ts;
mod js;
mod csharp;


fn print_symbol(symbol: &AstSymbolInstanceArc,
                guid_to_symbol_map: &HashMap<Uuid, AstSymbolInstanceArc>,
                used_guids: &mut HashSet<Uuid>,
                code: &str, indent: usize) {
    let sym = symbol.read();
    if used_guids.contains(&sym.guid()) {
        return;
    }
    used_guids.insert(sym.guid().clone());
    let indent_str = " ".repeat(indent);
    let full_range = sym.full_range().clone();
    let range = full_range.start_byte..full_range.end_byte;
    let mut name = sym.name().to_string();
    if let Some(caller_guid) = sym.get_caller_guid() {
        if guid_to_symbol_map.contains_key(&caller_guid) {
            name = format!("{} -> {}", name, caller_guid.to_string().slice(0..6));
        }
    }
    
    // Prepare a single line summary of the symbol
    let summary = format!(
        "{}| {}{} | {} | {} | {}",
        full_range.start_point.row + 1,
        indent_str,
        sym.guid().to_string().slice(0..6),
        name,
        sym.symbol_type(),
        code.slice(range).lines().collect::<Vec<_>>().first().unwrap(),
    );

    // Print the summary
    println!("{}", summary);

    // Recursively print children if any
    let children = sym.childs_guid().iter().filter_map( 
        |x| guid_to_symbol_map.get(x)
    ).sorted_by_key(|x| x.read().full_range().start_byte).collect::<Vec<_>>();
    
    for child in children {
        print_symbol(&child, &guid_to_symbol_map, used_guids, code, indent + 4);  // Increase indent for child elements
    }
}

pub(crate) fn print(symbols: &Vec<AstSymbolInstanceArc>, code: &str) {
    let guid_to_symbol_map = symbols.iter()
        .map(|s| (s.read().guid().clone(), s.clone())).collect::<HashMap<_, _>>();
    let sorted = symbols.iter().sorted_by_key(|x| x.read().full_range().start_byte).collect::<Vec<_>>();
    let mut used_guids: HashSet<Uuid> = Default::default();

    for sym in sorted {
        print_symbol(&sym, &guid_to_symbol_map, &mut used_guids, code, 0);
    }
}

fn eq_symbols(symbol: &AstSymbolInstanceArc,
              ref_symbol: &Box<dyn AstSymbolInstance>) -> bool {
    let symbol = symbol.read();
    let _f = symbol.fields();
    let _ref_f = ref_symbol.fields();
    
    let sym_type = symbol.symbol_type() == ref_symbol.symbol_type();
    let name = if ref_symbol.name().contains(ref_symbol.guid().to_string().as_str()) {
        symbol.name().contains(symbol.guid().to_string().as_str())
    } else {
        symbol.name() == ref_symbol.name()
    };

    let lang = symbol.language() == ref_symbol.language();
    let file_path = symbol.file_path() == ref_symbol.file_path();
    let is_type = symbol.is_type() == ref_symbol.is_type();
    let is_declaration = symbol.is_declaration() == ref_symbol.is_declaration();
    let namespace = symbol.namespace() == ref_symbol.namespace();
    let full_range = symbol.full_range() == ref_symbol.full_range();
    
    let declaration_range = symbol.declaration_range() == ref_symbol.declaration_range();
    let definition_range = symbol.definition_range() == ref_symbol.definition_range();
    let is_error = symbol.is_error() == ref_symbol.is_error();
    
    sym_type && name && lang && file_path && is_type && is_declaration &&
        namespace && full_range && declaration_range && definition_range && is_error
}

fn compare_symbols(symbols: &Vec<AstSymbolInstanceArc>,
                   ref_symbols: &Vec<Box<dyn AstSymbolInstance>>) {
    let guid_to_sym = symbols.iter().map(|s| (s.clone().read().guid().clone(), s.clone())).collect::<HashMap<_, _>>();
    let ref_guid_to_sym = ref_symbols.iter().map(|s| (s.guid().clone(), s)).collect::<HashMap<_, _>>();
    let mut checked_guids: HashSet<Uuid> = Default::default();
    for sym in symbols {
        let sym_l = sym.read();
        let _t = sym_l.symbol_type();
        let _f = sym_l.fields();
        if checked_guids.contains(&sym_l.guid()) {
            continue;
        }
        let closest_sym = ref_symbols.iter().filter(|s| sym_l.full_range() == s.full_range())
            .filter(|x| eq_symbols(&sym, x))
            .collect::<Vec<_>>();
        assert_eq!(closest_sym.len(), 1);
        let closest_sym = closest_sym.first().unwrap();
        let mut candidates: Vec<(AstSymbolInstanceArc, &Box<dyn AstSymbolInstance>)> = vec![(sym.clone(), &closest_sym)];
        while let Some((sym, ref_sym)) = candidates.pop() {
            let sym_l = sym.read();
            if checked_guids.contains(&sym_l.guid()) {
                continue;
            }
            checked_guids.insert(sym_l.guid().clone());
            if !eq_symbols(&sym, ref_sym) {
                eq_symbols(&sym, ref_sym);
            }

            assert!(eq_symbols(&sym, ref_sym));
            assert!(
                (sym_l.parent_guid().is_some() && ref_sym.parent_guid().is_some())
                    || (sym_l.parent_guid().is_none() && ref_sym.parent_guid().is_none())
            );
            if sym_l.parent_guid().is_some() {
                if let Some(parent) = guid_to_sym.get(&sym_l.parent_guid().unwrap()) {
                    let ref_parent = ref_guid_to_sym.get(&ref_sym.parent_guid().unwrap()).unwrap();
                    candidates.push((parent.clone(), ref_parent));
                }
            }

            assert_eq!(sym_l.childs_guid().len(), ref_sym.childs_guid().len());

            let childs = sym_l.childs_guid().iter().filter_map(|x| guid_to_sym.get(x))
                .collect::<Vec<_>>();
            let ref_childs = ref_sym.childs_guid().iter().filter_map(|x| ref_guid_to_sym.get(x))
                .collect::<Vec<_>>();

            for child in childs {
                let child_l = child.read();
                let _f = child_l.fields();
                let closest_sym = ref_childs.iter().filter(|s| child_l.full_range() == s.full_range() 
                    && child_l.declaration_range() == s.declaration_range())
                    .collect::<Vec<_>>();
                let _fs: Vec<_> = closest_sym.iter().map(|x| x.fields().clone()).collect(); 
                
                assert_eq!(closest_sym.len(), 1);
                let closest_sym = closest_sym.first().unwrap();
                candidates.push((child.clone(), closest_sym));
            }

            assert!((sym_l.get_caller_guid().is_some() && ref_sym.get_caller_guid().is_some())
                || (sym_l.get_caller_guid().is_none() && ref_sym.get_caller_guid().is_none())
            );
            if sym_l.get_caller_guid().is_some() {
                if let Some(caller) = guid_to_sym.get(&sym_l.get_caller_guid().unwrap()) {
                    let ref_caller = ref_guid_to_sym.get(&ref_sym.get_caller_guid().unwrap()).unwrap();
                    candidates.push((caller.clone(), ref_caller));
                }
            }
        }
    }
    assert_eq!(checked_guids.len(), ref_symbols.len());
}

fn check_duplicates(symbols: &Vec<AstSymbolInstanceArc>) {
    let mut checked_guids: HashSet<Uuid> = Default::default();
    for sym in symbols {
        let sym = sym.read();
        let _f = sym.fields();
        assert!(!checked_guids.contains(&sym.guid()));
        checked_guids.insert(sym.guid().clone());
    }
}

fn check_duplicates_with_ref(symbols: &Vec<Box<dyn AstSymbolInstance>>) {
    let mut checked_guids: HashSet<Uuid> = Default::default();
    for sym in symbols {
        let _f = sym.fields();
        assert!(!checked_guids.contains(&sym.guid()));
        checked_guids.insert(sym.guid().clone());
    }
}

pub(crate) fn base_parser_test(parser: &mut Box<dyn AstLanguageParser>,
                               path: &PathBuf,
                               code: &str, symbols_str: &str) {
    let symbols = parser.parse(code, &path);
    // use std::fs;
    // let symbols_str_ = serde_json::to_string_pretty(&symbols).unwrap();
    // fs::write("output.json", symbols_str_).expect("Unable to write file");
    check_duplicates(&symbols);
    print(&symbols, code);

    let ref_symbols: Vec<Box<dyn AstSymbolInstance>> = serde_json::from_str(&symbols_str).unwrap();
    check_duplicates_with_ref(&ref_symbols);

    compare_symbols(&symbols, &ref_symbols);
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, Eq, PartialEq, Hash)]
struct Skeleton {
    pub line: String,
}

pub(crate) fn base_skeletonizer_test(lang: &LanguageId,
                                     parser: &mut Box<dyn AstLanguageParser>,
                                     file: &PathBuf,
                                     code: &str, skeleton_ref_str: &str) {
    let symbols = parser.parse(code, &file);
    let symbols_struct = symbols.iter().map(|s| s.read().symbol_info_struct()).collect();
    let doc = Document {
        path: file.clone(),
        text: Some(Rope::from_str(code)),
    };
    let guid_to_children: HashMap<Uuid, Vec<Uuid>> = symbols.iter().map(|s| (s.read().guid().clone(), s.read().childs_guid().clone())).collect();
    let ast_markup: FileASTMarkup = crate::ast::ast_file_markup::lowlevel_file_markup(&doc, &symbols_struct).unwrap();
    let guid_to_info: HashMap<Uuid, &SymbolInformation> = ast_markup.symbols_sorted_by_path_len.iter().map(|s| (s.guid.clone(), s)).collect();
    let formatter = make_formatter(lang);
    let class_symbols: Vec<_> = ast_markup.symbols_sorted_by_path_len.iter().filter(|x| x.symbol_type == SymbolType::StructDeclaration).collect();
    let mut skeletons: HashSet<Skeleton> = Default::default();
    for symbol in class_symbols {
        let skeleton_line = formatter.make_skeleton(&symbol, &code.to_string(), &guid_to_children, &guid_to_info);
        skeletons.insert(Skeleton { line: skeleton_line });
    }
    // use std::fs;
    // let symbols_str_ = serde_json::to_string_pretty(&skeletons).unwrap();
    // fs::write("output.json", symbols_str_).expect("Unable to write file");
    let ref_skeletons: Vec<Skeleton> = serde_json::from_str(&skeleton_ref_str).unwrap();
    let ref_skeletons: HashSet<Skeleton> = HashSet::from_iter(ref_skeletons.iter().cloned());
    assert_eq!(skeletons, ref_skeletons);
}


#[derive(Default, Debug, Serialize, Deserialize, Clone, Eq, PartialEq, Hash)]
struct Decl {
    pub top_row: usize,
    pub bottom_row: usize,
    pub line: String,
}

pub(crate) fn base_declaration_formatter_test(lang: &LanguageId,
                                              parser: &mut Box<dyn AstLanguageParser>,
                                              file: &PathBuf,
                                              code: &str, decls_ref_str: &str) {
    let symbols = parser.parse(code, &file);
    let symbols_struct = symbols.iter().map(|s| s.read().symbol_info_struct()).collect();
    let doc = Document {
        path: file.clone(),
        text: Some(Rope::from_str(code)),
    };
    let guid_to_children: HashMap<Uuid, Vec<Uuid>> = symbols.iter().map(|s| (s.read().guid().clone(), s.read().childs_guid().clone())).collect();
    let ast_markup: FileASTMarkup = crate::ast::ast_file_markup::lowlevel_file_markup(&doc, &symbols_struct).unwrap();
    let guid_to_info: HashMap<Uuid, &SymbolInformation> = ast_markup.symbols_sorted_by_path_len.iter().map(|s| (s.guid.clone(), s)).collect();
    let formatter = make_formatter(lang);
    let mut decls: HashSet<Decl> = Default::default();
    for symbol in &guid_to_info {
        let symbol = guid_to_info.get(&symbol.0).unwrap();
        if !vec![SymbolType::StructDeclaration, SymbolType::FunctionDeclaration].contains(&symbol.symbol_type) {
            continue;
        }
        let (line, (top_row, bottom_row)) = formatter.get_declaration_with_comments(&symbol, &code.to_string(), &guid_to_children, &guid_to_info);
        if !line.is_empty() {
            decls.insert(Decl {
                top_row,
                bottom_row,
                line,
            });
        }
    }
    // use std::fs;
    // let symbols_str_ = serde_json::to_string_pretty(&decls).unwrap();
    // fs::write("output.json", symbols_str_).expect("Unable to write file");
    let ref_decls: Vec<Decl> = serde_json::from_str(&decls_ref_str).unwrap();
    let ref_decls: HashSet<Decl> = HashSet::from_iter(ref_decls.iter().cloned());
    assert_eq!(decls, ref_decls);
}
