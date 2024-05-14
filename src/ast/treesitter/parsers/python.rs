use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::string::ToString;

use similar::DiffableStr;
use tree_sitter::{Node, Parser, Point, Range};
use tree_sitter_python::language;
use uuid::Uuid;

use crate::ast::treesitter::ast_instance_structs::{AstSymbolFields, AstSymbolInstanceArc, ClassFieldDeclaration, CommentDefinition, FunctionArg, FunctionCall, FunctionDeclaration, ImportDeclaration, ImportType, StructDeclaration, TypeDef, VariableDefinition, VariableUsage};
use crate::ast::treesitter::language_id::LanguageId;
use crate::ast::treesitter::parsers::{AstLanguageParser, internal_error, ParserError};
use crate::ast::treesitter::parsers::utils::{get_children_guids, get_guid};

static PYTHON_MODULES: [&str; 203] = [
    "abc", "aifc", "argparse","array", "asynchat", "asyncio", "asyncore", "atexit", "audioop",
    "base64", "bdb", "binascii", "binhex", "bisect", "builtins", "bz2", "calendar", "cgi", "cgitb",
    "chunk", "cmath", "cmd", "code", "codecs", "codeop", "collections", "colorsys", "compileall",
    "concurrent", "configparser", "contextlib", "contextvars", "copy", "copyreg", "crypt", "csv",
    "ctypes", "curses", "datetime", "dbm", "decimal", "difflib", "dis", "distutils", "doctest",
    "email", "encodings", "ensurepip", "enum", "errno", "faulthandler", "fcntl", "filecmp",
    "fileinput", "fnmatch", "formatter", "fractions", "ftplib", "functools", "gc", "getopt",
    "getpass", "gettext", "glob", "grp", "gzip", "hashlib", "heapq", "hmac", "html", "http",
    "idlelib", "imaplib", "imghdr", "imp", "importlib", "inspect", "io", "ipaddress", "itertools",
    "json", "keyword", "lib2to3","linecache", "locale", "logging", "lzma", "macpath","mailbox",
    "mailcap", "marshal", "math", "mimetypes","mmap", "modulefinder", "msilib", "msvcrt",
    "multiprocessing", "netrc", "nntplib", "numbers", "operator", "optparse", "os", "ossaudiodev",
    "parser", "pathlib", "pdb", "pickle", "pickletools", "pipes", "pkgutil", "platform", "plistlib",
    "poplib", "posix", "pprint", "profile", "pstats", "pty", "pwd", "py_compile", "pyclbr", "pydoc",
    "queue", "quopri", "random", "re", "readline", "reprlib", "resource", "rlcompleter", "runpy",
    "sched", "secrets", "select", "selectors", "shelve", "shlex", "shutil", "signal", "site", "smtpd",
    "smtplib", "sndhdr", "socket", "socketserver", "spwd", "sqlite3", "ssl", "stat", "statistics",
    "string", "stringprep", "struct", "subprocess", "sunau", "symbol", "symtable", "sys", "sysconfig",
    "syslog", "tabnanny", "tarfile", "telnetlib", "tempfile", "termios", "test", "textwrap",
    "threading", "time", "timeit", "tkinter", "token", "tokenize", "trace", "traceback",
    "tracemalloc", "tty", "turtle", "turtledemo", "types", "typing", "unicodedata", "unittest",
    "urllib", "uu", "uuid", "venv", "warnings", "wave", "weakref", "webbrowser", "winreg", "winsound",
    "wsgiref", "xdrlib", "xml", "xmlrpc", "zipapp", "zipfile", "zipimport", "zoneinfo"
];


pub(crate) struct PythonParser {
    pub parser: Parser,
}

pub fn parse_type(parent: &Node, code: &str) -> Option<TypeDef> {
    let kind = parent.kind();
    let text = code.slice(parent.byte_range()).to_string();
    match kind {
        "type" | "splat_type" => {
            let child = parent.child(0).unwrap();
            return parse_type(&child, code);
        }
        "identifier" => {
            return Some(TypeDef {
                name: Some(text),
                inference_info: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types: vec![],
            });
        }
        "integer" | "string" | "float" | "false" | "true" => {
            return Some(TypeDef {
                name: None,
                inference_info: Some(text),
                is_pod: true,
                namespace: "".to_string(),
                guid: None,
                nested_types: vec![],
            });
        }
        "generic_type" => {
            let name = parent.child(0).unwrap();
            let name = code.slice(name.byte_range()).to_string();
            let type_arguments = parent.child(1).unwrap();
            let mut nested_types = vec![];
            for i in 0..type_arguments.child_count() {
                let child = type_arguments.child(i).unwrap();
                if let Some(t) = parse_type(&child, code) {
                    nested_types.push(t);
                }
            }
            return Some(TypeDef {
                name: Some(name),
                inference_info: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types,
            });
        }
        "attribute" => {
            let attribute = parent.child_by_field_name("attribute").unwrap();
            let name = code.slice(attribute.byte_range()).to_string();
            let object = parent.child_by_field_name("object").unwrap();
            let nested_types = {
                if let Some(dtype) = parse_type(&object, code) {
                    vec![dtype]
                } else {
                    vec![]
                }
            };
            return Some(TypeDef {
                name: Some(name),
                inference_info: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types,
            });
        }
        "call" => {
            let function = parent.child_by_field_name("function").unwrap();
            let mut dtype = parse_type(&function, code).unwrap_or(TypeDef::default());
            dtype.inference_info = Some(code.slice(parent.byte_range()).to_string());
            return Some(dtype);
        }
        &_ => {}
    }
    None
}

fn parse_function_arg(parent: &Node, code: &str) -> Vec<FunctionArg> {
    let mut args: Vec<FunctionArg> = vec![];
    let kind = parent.kind();
    match kind {
        "identifier" | "typed_parameter" => {
            let arg = FunctionArg {
                name: code.slice(parent.byte_range()).to_string(),
                type_: None,
            };
            args.push(arg);
        }
        "typed_default_parameter" | "default_parameter" => {
            let name = parent.child_by_field_name("name").unwrap();
            if name.kind() == "identifier" {
                let arg = FunctionArg {
                    name: code.slice(name.byte_range()).to_string(),
                    type_: None,
                };
                args.push(arg);
            } else {
                args.extend(parse_function_arg(&name, code));
            }
        }
        "tuple_pattern" => {
            for i in 0..parent.child_count() - 1 {
                let child = parent.child(i).unwrap();
                args.extend(parse_function_arg(&child, code));
            }
        }
        _ => {}
    }

    for arg in args.iter_mut() {
        if let Some(type_node) = parent.child_by_field_name("type") {
            if let Some(dtype) = parse_type(&type_node, code) {
                if let Some(type_) = &mut arg.type_ {
                    type_.inference_info = dtype.inference_info;
                } else {
                    arg.type_ = Some(dtype);
                }
            }
        }
    }

    if let Some(value_node) = parent.child_by_field_name("value") {
        let value_text = code.slice(value_node.byte_range()).to_string();
        for arg in args.iter_mut() {
            if arg.type_.is_some() {
                let type_ = arg.type_.as_mut().unwrap();
                type_.inference_info = Some(value_text.clone());
            } else {
                arg.type_ = Some(TypeDef {
                    name: None,
                    inference_info: Some(value_text.clone()),
                    is_pod: false,
                    namespace: "".to_string(),
                    guid: None,
                    nested_types: vec![],
                })
            }
        }
    }

    args
}

const SPECIAL_SYMBOLS: &str = "{}(),.;_|&";
const PYTHON_KEYWORDS: [&'static str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
    "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield"
];

impl PythonParser {
    pub fn new() -> Result<PythonParser, ParserError> {
        let mut parser = Parser::new();
        parser
            .set_language(language())
            .map_err(internal_error)?;
        Ok(PythonParser { parser })
    }

    pub fn parse_struct_declaration(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &Uuid, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = StructDeclaration::default();

        decl.ast_fields.language = LanguageId::Python;
        decl.ast_fields.full_range = parent.range();
        decl.ast_fields.file_path = path.clone();
        decl.ast_fields.parent_guid = Some(parent_guid.clone());
        decl.ast_fields.guid = get_guid();
        decl.ast_fields.is_error = is_error;

        symbols.extend(self.find_error_usages(&parent, code, path, &decl.ast_fields.guid));

        if let Some(parent_node) = parent.parent() {
            if parent_node.kind() == "decorated_definition" {
                decl.ast_fields.full_range = parent_node.range();
            }
        }

        if let Some(name_node) = parent.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name_node.byte_range()).to_string();
        }
        if let Some(superclasses) = parent.child_by_field_name("superclasses") {
            for i in 0..superclasses.child_count() {
                let child = superclasses.child(i).unwrap();
                if let Some(dtype) = parse_type(&child, code) {
                    decl.inherited_types.push(dtype);
                }
            }
            symbols.extend(self.find_error_usages(&superclasses, code, path, &decl.ast_fields.guid));
        }
        if let Some(body) = parent.child_by_field_name("body") {
            symbols.extend(self.parse_usages(&body, code, path, &decl.ast_fields.guid, is_error));
        }

        decl.ast_fields.childs_guid = get_children_guids(&decl.ast_fields.guid, &symbols);
        symbols.push(Rc::new(RefCell::new(decl)));
        symbols
    }

    fn parse_assignment(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &Uuid, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut is_class_field = false;
        {
            let mut parent_mb = parent.parent();
            while parent_mb.is_some() {
                let p = parent_mb.unwrap();
                match p.kind() {
                    "class_definition" => {
                        is_class_field = true;
                        break;
                    }
                    "function_definition" => {
                        break;
                    }
                    &_ => {}
                }
                parent_mb = p.parent();
            }
        }


        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];
        if let Some(right) = parent.child_by_field_name("right") {
            symbols.extend(self.parse_usages(&right, code, path, parent_guid, is_error));
        }
        if let Some(body) = parent.child_by_field_name("body") {
            symbols.extend(self.parse_usages(&body, code, path, parent_guid, is_error));
        }

        let mut candidates: VecDeque<(Option<Node>, Option<Node>, Option<Node>)> = VecDeque::from(vec![
            (parent.child_by_field_name("left"),
             parent.child_by_field_name("type"),
             parent.child_by_field_name("right"))]);
        let mut right_for_all = false;
        while !candidates.is_empty() {
            let (left_mb, type_mb, right_mb) = candidates.pop_front().unwrap();
            if let Some(left) = left_mb {
                let text = code.slice(left.byte_range());
                if SPECIAL_SYMBOLS.contains(text) || text == "self" {
                    continue;
                }
                let kind = left.kind();
                match kind {
                    "identifier" => {
                        let mut fields = AstSymbolFields::default();
                        fields.language = LanguageId::Python;
                        fields.full_range = parent.range();
                        fields.file_path = path.clone();
                        fields.parent_guid = Some(parent_guid.clone());
                        fields.guid = get_guid();
                        fields.name = code.slice(left.byte_range()).to_string();
                        fields.is_error = is_error;

                        if is_class_field {
                            let mut decl = ClassFieldDeclaration::default();
                            decl.ast_fields = fields;
                            if let Some(type_node) = type_mb {
                                if let Some(type_) = parse_type(&type_node, code) {
                                    decl.type_ = type_;
                                }
                            }
                            symbols.push(Rc::new(RefCell::new(decl)));
                        } else {
                            let mut decl = VariableDefinition::default();
                            decl.ast_fields = fields;
                            if let Some(type_) = type_mb {
                                if let Some(dtype) = parse_type(&type_, code) {
                                    decl.type_ = dtype;
                                }
                            }
                            if let Some(right) = right_mb {
                                decl.type_.inference_info = Some(code.slice(right.byte_range()).to_string());
                                decl.type_.is_pod = vec!["integer", "string", "float", "false", "true"]
                                    .contains(&right.kind());
                            }
                            symbols.push(Rc::new(RefCell::new(decl)));
                        }
                    }
                    "attribute" => {
                        let usages = self.parse_usages(&left, code, path, parent_guid, is_error);
                        symbols.extend(usages);
                    }
                    "list_pattern" | "tuple_pattern" | "pattern_list" => {
                        let lefts: Vec<_> = (0..left.child_count())
                            .map(|i| left.child(i))
                            .filter(|node| !SPECIAL_SYMBOLS.contains(node.unwrap().kind()))
                            .collect();
                        let mut rights = vec![right_mb];
                        if let Some(right) = right_mb {
                            rights = (0..right.child_count())
                                .map(|i| right.child(i))
                                .filter(|node| !SPECIAL_SYMBOLS.contains(node.unwrap().kind()))
                                .collect();
                        }
                        if lefts.len() != rights.len() {
                            right_for_all = true;
                        }
                        for i in 0..lefts.len() {
                            let r = if right_for_all { right_mb } else { rights[i] };
                            candidates.push_back((*lefts.get(i).unwrap(), None, r));
                        }
                    }
                    "list_splat_pattern" => {
                        let child = left.child(0);
                        candidates.push_back((child, type_mb, right_mb));
                    }
                    &_ => {}
                }
            }
        }

        // https://github.com/tree-sitter/tree-sitter-python/blob/master/grammar.js#L844
        symbols
    }

    pub fn parse_usages(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &Uuid, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];
        let kind = parent.kind();
        #[cfg(test)]
        #[allow(unused)]
            let text = code.slice(parent.byte_range());
        // TODO lambda https://github.com/tree-sitter/tree-sitter-python/blob/master/grammar.js#L830
        match kind {
            "expression_statement" | "module" | "block" |
            "await" | "list_splat" | "yield" | "list_splat_pattern" |
            "tuple" | "set" | "list" | "dictionary" | "expression_list" | "comparison_operator" |
            "conditional_expression" | "as_pattern_target" | "print_statement" |
            "list_comprehension" | "dictionary_comprehension" | "set_comprehension" | "if_clause" |
            "with_statement" | "with_clause" | "case_clause" | "case_pattern" | "dotted_name" |
            "try_statement" | "except_clause" | "if_statement" | "elif_clause" | "else_clause" => {
                for i in 0..parent.child_count() {
                    let child = parent.child(i).unwrap();
                    symbols.extend(self.parse_usages(&child, code, path, parent_guid, is_error));
                }
            }
            "with_item" => {
                let value = parent.child_by_field_name("value").unwrap();
                symbols.extend(self.parse_usages(&value, code, path, parent_guid, is_error));
            }
            "class_definition" => {
                symbols.extend(self.parse_struct_declaration(&parent, code, path, parent_guid, is_error));
            }
            "function_definition" => {
                symbols.extend(self.parse_function_declaration(&parent, code, path, parent_guid, is_error));
            }
            "decorated_definition" => {
                if let Some(definition) = parent.child_by_field_name("definition") {
                    match definition.kind() {
                        "class_definition" => {
                            symbols.extend(self.parse_struct_declaration(&definition, code, path, parent_guid, is_error));
                        }
                        "function_definition" => {
                            symbols.extend(self.parse_function_declaration(&definition, code, path, parent_guid, is_error));
                        }
                        &_ => {}
                    }
                }
            }
            "as_pattern" => {
                let value = parent.child(0).unwrap();
                if let Some(alias) = parent.child_by_field_name("alias") {
                    let mut candidates = VecDeque::from(vec![alias.child(0).unwrap()]);
                    while !candidates.is_empty() {
                        let child = candidates.pop_front().unwrap();
                        let text = code.slice(child.byte_range());
                        if SPECIAL_SYMBOLS.contains(text) || text == "self" {
                            continue;
                        }
                        match child.kind() {
                            "identifier" => {
                                let mut decl = VariableDefinition::default();
                                decl.ast_fields.language = LanguageId::Python;
                                decl.ast_fields.full_range = parent.range();
                                decl.ast_fields.file_path = path.clone();
                                decl.ast_fields.parent_guid = Some(parent_guid.clone());
                                decl.ast_fields.guid = get_guid();
                                decl.ast_fields.name = text.to_string();
                                decl.type_.inference_info = Some(code.slice(value.byte_range()).to_string());
                                decl.ast_fields.is_error = is_error;
                                symbols.push(Rc::new(RefCell::new(decl)));
                            }
                            "list" | "set" | "tuple" => {
                                for i in 0..child.child_count() {
                                    candidates.push_back(child.child(i).unwrap());
                                }
                            }
                            &_ => {
                                symbols.extend(self.parse_usages(&child, code, path, parent_guid, is_error));
                            }
                        }
                    }
                }
            }
            "not_operator" | "unary_operator" => {
                let argument = parent.child_by_field_name("argument").unwrap();
                symbols.extend(self.parse_usages(&argument, code, path, parent_guid, is_error));
            }
            "boolean_operator" | "binary_operator" | "for_in_clause" | "augmented_assignment" => {
                let left = parent.child_by_field_name("left").unwrap();
                symbols.extend(self.parse_usages(&left, code, path, parent_guid, is_error));
                let right = parent.child_by_field_name("right").unwrap();
                symbols.extend(self.parse_usages(&right, code, path, parent_guid, is_error));
            }
            "pair" => {
                let key = parent.child_by_field_name("key").unwrap();
                symbols.extend(self.parse_usages(&key, code, path, parent_guid, is_error));
                let value = parent.child_by_field_name("value").unwrap();
                symbols.extend(self.parse_usages(&value, code, path, parent_guid, is_error));
            }
            "identifier" => {
                let mut usage = VariableUsage::default();
                usage.ast_fields.name = code.slice(parent.byte_range()).to_string();
                usage.ast_fields.language = LanguageId::Python;
                usage.ast_fields.full_range = parent.range();
                usage.ast_fields.file_path = path.clone();
                usage.ast_fields.parent_guid = Some(parent_guid.clone());
                usage.ast_fields.guid = get_guid();
                usage.ast_fields.is_error = is_error;
                symbols.push(Rc::new(RefCell::new(usage)));
            }
            "attribute" => {
                let attribute = parent.child_by_field_name("attribute").unwrap();
                let name = code.slice(attribute.byte_range()).to_string();
                let mut usage = VariableUsage::default();
                usage.ast_fields.name = name;
                usage.ast_fields.language = LanguageId::Python;
                usage.ast_fields.full_range = parent.range();
                usage.ast_fields.file_path = path.clone();
                usage.ast_fields.parent_guid = Some(parent_guid.clone());
                usage.ast_fields.guid = get_guid();
                usage.ast_fields.is_error = is_error;

                let object_node = parent.child_by_field_name("object").unwrap();
                let usages = self.parse_usages(&object_node, code, path, parent_guid, is_error);
                if let Some(last) = usages.last() {
                    usage.ast_fields.caller_guid = last.borrow().fields().parent_guid.clone();
                }
                symbols.extend(usages);
                symbols.push(Rc::new(RefCell::new(usage)));
            }
            "assignment" | "for_statement" => {
                symbols.extend(self.parse_assignment(&parent, code, path, parent_guid, is_error));
            }
            "while_statement" => {
                let condition = parent.child_by_field_name("condition").unwrap();
                symbols.extend(self.parse_usages(&condition, code, path, parent_guid, is_error));
                let body = parent.child_by_field_name("body").unwrap();
                symbols.extend(self.parse_usages(&body, code, path, parent_guid, is_error));
                if let Some(alternative) = parent.child_by_field_name("alternative") {
                    if let Some(body) = alternative.child_by_field_name("body") {
                        symbols.extend(self.parse_usages(&body, code, path, parent_guid, is_error));
                    }
                }
            }
            "match_statement" => {
                let subject = parent.child_by_field_name("subject").unwrap();
                symbols.extend(self.parse_usages(&subject, code, path, parent_guid, is_error));
                let body = parent.child_by_field_name("body").unwrap();
                symbols.extend(self.parse_usages(&body, code, path, parent_guid, is_error));
            }
            "call" => {
                symbols.extend(self.parse_call_expression(&parent, code, path, parent_guid, is_error));
            }
            "lambda" => {
                symbols.extend(self.parse_function_declaration(&parent, code, path, parent_guid, is_error));
            }
            "comment" => {
                let mut def = CommentDefinition::default();
                def.ast_fields.language = LanguageId::Python;
                def.ast_fields.full_range = parent.range();
                def.ast_fields.file_path = path.clone();
                def.ast_fields.parent_guid = Some(parent_guid.clone());
                def.ast_fields.guid = get_guid();
                def.ast_fields.is_error = false;
                symbols.push(Rc::new(RefCell::new(def)));
            }
            "import_from_statement" | "import_statement" => {
                let mut def = ImportDeclaration::default();
                def.ast_fields.language = LanguageId::Python;
                def.ast_fields.full_range = parent.range();
                def.ast_fields.file_path = path.clone();
                def.ast_fields.full_range = parent.range();
                def.ast_fields.parent_guid = Some(parent_guid.clone());
                
                let mut base_path_component: Vec<String> = Default::default();
                if let Some(module_name) = parent.child_by_field_name("module_name") {
                    if module_name.kind() == "relative_import" {
                        let base_path = code.slice(module_name.byte_range()).to_string();
                        if base_path.starts_with("..") {
                            base_path_component.push("..".to_string());
                            base_path_component.extend(base_path.slice(2..base_path.len()).split(".")
                                .map(|x| x.to_string())
                                .filter(|x| !x.is_empty())
                                .collect::<Vec<String>>());
                        } else if base_path.starts_with(".") {
                            base_path_component.push(".".to_string());
                            base_path_component.extend(base_path.slice(1..base_path.len()).split(".")
                                .map(|x| x.to_string())
                                .filter(|x| !x.is_empty())
                                .collect::<Vec<String>>());
                        } else {
                            base_path_component = base_path.split(".")
                                .map(|x| x.to_string())
                                .filter(|x| !x.is_empty())
                                .collect();
                        }
                    } else {
                        base_path_component = code.slice(module_name.byte_range()).to_string().split(".")
                            .map(|x| x.to_string())
                            .filter(|x| !x.is_empty())
                            .collect();
                    }
                }
                def.path_components = base_path_component.clone();
                if parent.child_by_field_name("name").is_some() {
                    let mut cursor = parent.walk();
                    for child in parent.children_by_field_name("name", &mut cursor) {
                        let mut def_local = def.clone();
                        def_local.ast_fields.guid = get_guid();

                        let mut path_components: Vec<String> = Default::default();
                        let mut alias: Option<String> = None;
                        match child.kind() {
                            "dotted_name" => {
                                path_components = code.slice(child.byte_range()).to_string().split(".").map(|x| x.to_string()).collect();
                            }
                            "aliased_import" => {
                                if let Some(name) = child.child_by_field_name("name") {
                                    path_components = code.slice(name.byte_range()).to_string().split(".").map(|x| x.to_string()).collect();
                                }
                                if let Some(alias_node) = child.child_by_field_name("alias") {
                                    alias = Some(code.slice(alias_node.byte_range()).to_string());
                                }
                            }
                            _ => {}
                        }
                        def_local.path_components.extend(path_components);
                        if let Some(first) = def_local.path_components.first() {
                            if PYTHON_MODULES.contains(&first.as_str()) {
                                def_local.import_type = ImportType::System;
                            } else if first == "." || first == ".." {
                                def_local.import_type = ImportType::UserModule;
                            }
                        }
                        def_local.ast_fields.name = def_local.path_components.last().unwrap().to_string();
                        def_local.alias = alias;

                        symbols.push(Rc::new(RefCell::new(def_local)));
                    }
                } else {
                    def.ast_fields.guid = get_guid();
                    symbols.push(Rc::new(RefCell::new(def)));
                }
            }
            "ERROR" => {
                symbols.extend(self.parse_error_usages(&parent, code, path, parent_guid));
            }
            _ => {
                for i in 0..parent.child_count() {
                    let child = parent.child(i).unwrap();
                    symbols.extend(self.parse_usages(&child, code, path, parent_guid, is_error));
                }
            }
        }
        symbols
    }

    pub fn parse_function_declaration(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &Uuid, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = FunctionDeclaration::default();
        decl.ast_fields.language = LanguageId::Python;
        decl.ast_fields.full_range = parent.range();
        decl.ast_fields.file_path = path.clone();
        decl.ast_fields.parent_guid = Some(parent_guid.clone());
        decl.ast_fields.is_error = is_error;
        if let Some(parent_node) = parent.parent() {
            if parent_node.kind() == "decorated_definition" {
                decl.ast_fields.full_range = parent_node.range();
            }
        }
        symbols.extend(self.find_error_usages(&parent, code, path, &decl.ast_fields.guid));

        let mut decl_end_byte: usize = parent.end_byte();
        let mut decl_end_point: Point = parent.end_position();

        if let Some(name_node) = parent.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name_node.byte_range()).to_string();
        }

        if let Some(parameters_node) = parent.child_by_field_name("parameters") {
            decl_end_byte = parameters_node.end_byte();
            decl_end_point = parameters_node.end_position();
            symbols.extend(self.find_error_usages(&parameters_node, code, path, &decl.ast_fields.guid));

            let params_len = parameters_node.child_count();
            let mut function_args = vec![];
            for idx in 0..params_len {
                let child = parameters_node.child(idx).unwrap();
                function_args.extend(parse_function_arg(&child, code));
            }
            decl.args = function_args;
        }
        decl.ast_fields.guid = get_guid();
        if let Some(return_type) = parent.child_by_field_name("return_type") {
            decl.return_type = parse_type(&return_type, code);
            decl_end_byte = return_type.end_byte();
            decl_end_point = return_type.end_position();
            symbols.extend(self.find_error_usages(&return_type, code, path, &decl.ast_fields.guid));
        }

        if let Some(body_node) = parent.child_by_field_name("body") {
            decl.ast_fields.definition_range = body_node.range();
            decl.ast_fields.declaration_range = Range {
                start_byte: decl.ast_fields.full_range.start_byte,
                end_byte: decl_end_byte,
                start_point: decl.ast_fields.full_range.start_point,
                end_point: decl_end_point,
            };
            symbols.extend(self.parse_usages(&body_node, code, path, &decl.ast_fields.guid, is_error));
        } else {
            decl.ast_fields.declaration_range = decl.ast_fields.full_range.clone();
        }

        decl.ast_fields.childs_guid = get_children_guids(&decl.ast_fields.guid, &symbols);
        symbols.push(Rc::new(RefCell::new(decl)));
        symbols
    }

    fn find_error_usages(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &Uuid) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        for i in 0..parent.child_count() {
            let child = parent.child(i).unwrap();
            if  child.kind() == "ERROR" {
                symbols.extend(self.parse_error_usages(&child, code, path, parent_guid));
            }
        }
        symbols
    }

    fn parse_error_usages(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &Uuid) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        match parent.kind() {
            "identifier" => {
                let name = code.slice(parent.byte_range()).to_string();
                if PYTHON_KEYWORDS.contains(&name.as_str()) {
                    return vec![];
                }
                let mut usage = VariableUsage::default();
                usage.ast_fields.name = name;
                usage.ast_fields.language = LanguageId::Python;
                usage.ast_fields.full_range = parent.range();
                usage.ast_fields.file_path = path.clone();
                usage.ast_fields.parent_guid = Some(parent_guid.clone());
                usage.ast_fields.guid = get_guid();
                usage.ast_fields.is_error = true;
                symbols.push(Rc::new(RefCell::new(usage)));
            }
            "attribute" => {
                let attribute = parent.child_by_field_name("attribute").unwrap();
                let name = code.slice(attribute.byte_range()).to_string();
                let mut usage = VariableUsage::default();
                usage.ast_fields.name = name;
                usage.ast_fields.language = LanguageId::Python;
                usage.ast_fields.full_range = parent.range();
                usage.ast_fields.file_path = path.clone();
                usage.ast_fields.parent_guid = Some(parent_guid.clone());
                usage.ast_fields.guid = get_guid();
                usage.ast_fields.is_error = true;

                let object_node = parent.child_by_field_name("object").unwrap();
                let usages = self.parse_error_usages(&object_node, code, path, parent_guid);
                if let Some(last) = usages.last() {
                    usage.ast_fields.caller_guid = last.borrow().fields().parent_guid.clone();
                }
                symbols.extend(usages);
                symbols.push(Rc::new(RefCell::new(usage)));
            }
            &_ => {
                for i in 0..parent.child_count() {
                    let child = parent.child(i).unwrap();
                    symbols.extend(self.parse_error_usages(&child, code, path, parent_guid));
                }
            }
        }

        symbols
    }

    pub fn parse_call_expression(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &Uuid, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = FunctionCall::default();
        decl.ast_fields.language = LanguageId::Python;
        decl.ast_fields.full_range = parent.range();
        decl.ast_fields.file_path = path.clone();
        decl.ast_fields.parent_guid = Some(parent_guid.clone());
        decl.ast_fields.guid = get_guid();
        decl.ast_fields.is_error = is_error;

        symbols.extend(self.find_error_usages(&parent, code, path, &decl.ast_fields.guid));

        let arguments_node = parent.child_by_field_name("arguments").unwrap();
        for i in 0..arguments_node.child_count() {
            let child = arguments_node.child(i).unwrap();
            let text = code.slice(child.byte_range());
            if SPECIAL_SYMBOLS.contains(&text) { continue; }
            symbols.extend(self.parse_usages(&child, code, path, &decl.ast_fields.guid, is_error));
        }
        symbols.extend(self.find_error_usages(&arguments_node, code, path, &decl.ast_fields.guid));

        let function_node = parent.child_by_field_name("function").unwrap();
        let text = code.slice(function_node.byte_range());
        let kind = function_node.kind();
        match kind {
            "identifier" => {
                decl.ast_fields.name = text.to_string();
            }
            "attribute" => {
                let object = function_node.child_by_field_name("object").unwrap();
                let usages = self.parse_usages(&object, code, path, parent_guid, is_error);
                if let Some(last) = usages.last() {
                    decl.ast_fields.caller_guid = last.borrow().fields().parent_guid.clone();
                }
                symbols.extend(usages);
                let attribute = function_node.child_by_field_name("attribute").unwrap();
                decl.ast_fields.name = code.slice(attribute.byte_range()).to_string();
            }
            _ => {
                let usages = self.parse_usages(&function_node, code, path, parent_guid, is_error);
                if let Some(last) = usages.last() {
                    decl.ast_fields.caller_guid = last.borrow().fields().parent_guid.clone();
                }
                symbols.extend(usages);
            }
        }

        decl.ast_fields.childs_guid = get_children_guids(&decl.ast_fields.guid, &symbols);
        symbols.push(Rc::new(RefCell::new(decl)));
        symbols
    }
}

impl AstLanguageParser for PythonParser {
    fn parse(&mut self, code: &str, path: &PathBuf) -> Vec<AstSymbolInstanceArc> {
        let tree = self.parser.parse(code, None).unwrap();
        let parent_guid = get_guid();
        let symbols = self.parse_usages(&tree.root_node(), code, path, &parent_guid, false);
        symbols
    }
}
