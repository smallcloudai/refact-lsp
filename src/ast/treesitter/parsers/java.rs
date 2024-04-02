use std::path::PathBuf;
use std::string::ToString;
use std::sync::{Arc, RwLock};

use similar::DiffableStr;
use tree_sitter::{Node, Parser, Range};
use tree_sitter_java::language;

use crate::ast::treesitter::ast_instance_structs::{AstSymbolInstanceArc, ClassFieldDeclaration, CommentDefinition, FunctionArg, FunctionCall, FunctionDeclaration, StructDeclaration, TypeDef, VariableDefinition, VariableUsage};
use crate::ast::treesitter::language_id::LanguageId;
use crate::ast::treesitter::parsers::{AstLanguageParser, internal_error, ParserError};
use crate::ast::treesitter::parsers::utils::{get_children_guids, get_guid, str_hash};

pub(crate) struct JavaParser {
    pub parser: Parser,
}

pub fn parse_type(parent: &Node, code: &str) -> Option<TypeDef> {
    let kind = parent.kind();
    let text = code.slice(parent.byte_range()).to_string();
    match kind {
        "type_parameters" | "type_list" => {
            let child = parent.child(0).unwrap();
            return parse_type(&child, code);
        }
        "type_identifier" | "identifier" => {
            return Some(TypeDef {
                name: Some(text),
                inference_info: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types: vec![],
            });
        }
        "void_type" | "integral_type" | "floating_point_type" | "boolean_type" => {
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
            let mut decl = TypeDef {
                name: None,
                inference_info: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types: vec![],
            };
            for i in 0..parent.child_count() {
                let child = parent.child(i).unwrap();
                match child.kind() {
                    "type_identifier" => {
                        decl.name = Some(code.slice(child.byte_range()).to_string());
                    }
                    "type_arguments" => {
                        for i in 0..child.child_count() {
                            let child = child.child(i).unwrap();
                            if let Some(t) = parse_type(&child, code) {
                                decl.nested_types.push(t);
                            }
                        }
                    }
                    &_ => {}
                }
            }

            return Some(decl);
        }
        "array_type" => {
            let mut decl = TypeDef {
                name: Some("[]".to_string()),
                inference_info: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types: vec![],
            };
            if let Some(dimensions) = parent.child_by_field_name("dimensions") {
                decl.name = Some(code.slice(dimensions.byte_range()).to_string());
            }

            if let Some(element) = parent.child_by_field_name("element") {
                if let Some(dtype) = parse_type(&element, code) {
                    decl.nested_types.push(dtype);
                }
            }
            return Some(decl);
        }
        "type_parameter" => {
            let mut def = TypeDef::default();
            for i in 0..parent.child_count() {
                let child = parent.child(i).unwrap();
                match child.kind() {
                    "type_identifier" => {
                        def.name = Some(code.slice(child.byte_range()).to_string());
                    }
                    "type_bound" => {
                        if let Some(dtype) = parse_type(&child, code) {
                            def.nested_types.push(dtype);
                        }
                    }
                    &_ => {}
                }
            }
        }
        "scoped_type_identifier" => {
            fn _parse(&parent: &Node, code: &str) -> String {
                let mut result = String::default();
                for i in 0..parent.child_count() {
                    let child = parent.child(i).unwrap();
                    match child.kind() {
                        "type_identifier" => {
                            if result.is_empty() {
                                result = code.slice(child.byte_range()).to_string();
                            } else {
                                result = result + "." + &*code.slice(child.byte_range()).to_string();
                            }
                        }
                        "scoped_type_identifier" => {
                            if result.is_empty() {
                                result = _parse(&child, code);
                            } else {
                                result = _parse(&child, code) + "." + &*result;
                            }
                        }
                        &_ => {}
                    }
                }
                result
            }
            let mut decl = TypeDef {
                name: None,
                inference_info: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types: vec![],
            };

            for i in 0..parent.child_count() {
                let child = parent.child(i).unwrap();
                match child.kind() {
                    "type_identifier" => {
                        decl.name = Some(code.slice(child.byte_range()).to_string());
                    }
                    "scoped_type_identifier" => {
                        decl.namespace = _parse(&child, code);
                    }
                    &_ => {}
                }
            }
            return Some(decl);
        }
        &_ => {}
    }
    None
}

fn parse_function_arg(parent: &Node, code: &str) -> FunctionArg {
    let mut arg = FunctionArg::default();
    if let Some(name) = parent.child_by_field_name("name") {
        arg.name = code.slice(name.byte_range()).to_string();
    }
    if let Some(dimensions) = parent.child_by_field_name("dimensions") {
        arg.type_ = Some(TypeDef {
            name: Some(code.slice(dimensions.byte_range()).to_string()),
            inference_info: None,
            is_pod: false,
            namespace: "".to_string(),
            guid: None,
            nested_types: vec![],
        })
    }

    if let Some(type_node) = parent.child_by_field_name("type") {
        if let Some(dtype) = parse_type(&type_node, code) {
            if let Some(arg_dtype) = &mut arg.type_ {
                arg_dtype.nested_types.push(dtype);
            } else {
                arg.type_ = Some(dtype);
            }
        }
    }

    arg
}

impl JavaParser {
    pub fn new() -> Result<JavaParser, ParserError> {
        let mut parser = Parser::new();
        parser
            .set_language(language())
            .map_err(internal_error)?;
        Ok(JavaParser { parser })
    }

    pub fn parse_struct_declaration(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &String, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = StructDeclaration::default();

        decl.ast_fields.language = LanguageId::Java;
        decl.ast_fields.full_range = parent.range();
        decl.ast_fields.declaration_range = parent.range();
        decl.ast_fields.definition_range = parent.range();
        decl.ast_fields.file_path = path.clone();
        decl.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
        decl.ast_fields.parent_guid = Some(parent_guid.clone());
        decl.ast_fields.guid = get_guid();
        decl.ast_fields.is_error = is_error;

        if let Some(name_node) = parent.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name_node.byte_range()).to_string();
        }

        if let Some(node) = parent.child_by_field_name("superclass") {
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if let Some(dtype) = parse_type(&child, code) {
                    decl.inherited_types.push(dtype);
                }
            }
        }
        if let Some(node) = parent.child_by_field_name("interfaces") {
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                match child.kind() {
                    "type_list" => {
                        for i in 0..child.child_count() {
                            let child = child.child(i).unwrap();
                            if let Some(dtype) = parse_type(&child, code) {
                                decl.inherited_types.push(dtype);
                            }
                        }
                    }
                    &_ => {}
                }
            }
        }
        if let Some(_) = parent.child_by_field_name("type_parameters") {}


        if let Some(body) = parent.child_by_field_name("body") {
            decl.ast_fields.declaration_range = body.range();
            decl.ast_fields.definition_range = Range {
                start_byte: decl.ast_fields.full_range.start_byte,
                end_byte: decl.ast_fields.declaration_range.start_byte,
                start_point: decl.ast_fields.full_range.start_point,
                end_point: decl.ast_fields.declaration_range.start_point,
            };
            symbols.extend(self.parse_usages(&body, code, path, &decl.ast_fields.guid, is_error));
        }

        decl.ast_fields.childs_guid = get_children_guids(&decl.ast_fields.guid, &symbols);
        symbols.push(Arc::new(RwLock::new(decl)));
        symbols
    }

    fn parse_variable_definition(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &String, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];
        let mut type_ = TypeDef::default();
        if let Some(type_node) = parent.child_by_field_name("type") {
            if let Some(dtype) = parse_type(&type_node, code) {
                type_ = dtype;
            }
        }

        for i in 0..parent.child_count() {
            let child = parent.child(i).unwrap();
            match child.kind() {
                "variable_declarator" => {
                    let local_dtype = type_.clone();
                    let mut decl = VariableDefinition::default();
                    decl.ast_fields.language = LanguageId::Java;
                    decl.ast_fields.full_range = parent.range();
                    decl.ast_fields.file_path = path.clone();
                    decl.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
                    decl.ast_fields.parent_guid = Some(parent_guid.clone());
                    decl.ast_fields.guid = get_guid();
                    decl.ast_fields.is_error = is_error;
                    decl.type_ = type_.clone();

                    if let Some(name) = child.child_by_field_name("name") {
                        decl.ast_fields.name = code.slice(name.byte_range()).to_string();
                    }
                    if let Some(value) = child.child_by_field_name("value") {
                        decl.type_.inference_info = Some(code.slice(value.byte_range()).to_string());
                        symbols.extend(self.parse_usages(&value, code, path, parent_guid, is_error));
                    }
                    if let Some(dimensions) = child.child_by_field_name("dimensions") {
                        decl.type_ = TypeDef {
                            name: Some(code.slice(dimensions.byte_range()).to_string()),
                            inference_info: None,
                            is_pod: false,
                            namespace: "".to_string(),
                            guid: None,
                            nested_types: vec![local_dtype],
                        };
                    } else {
                        decl.type_ = local_dtype;
                    }
                    symbols.push(Arc::new(RwLock::new(decl)));
                }
                &_ => {}
            }
        }

        symbols
    }

    fn parse_field_declaration(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &String, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];
        let mut dtype = TypeDef::default();
        if let Some(type_node) = parent.child_by_field_name("type") {
            if let Some(type_) = parse_type(&type_node, code) {
                dtype = type_;
            }
        }

        for i in 0..parent.child_count() {
            let child = parent.child(i).unwrap();
            match child.kind() {
                "variable_declarator" => {
                    let local_dtype = dtype.clone();

                    let mut decl = ClassFieldDeclaration::default();
                    decl.ast_fields.language = LanguageId::Java;
                    decl.ast_fields.full_range = parent.range();
                    decl.ast_fields.file_path = path.clone();
                    decl.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
                    decl.ast_fields.parent_guid = Some(parent_guid.clone());
                    decl.ast_fields.guid = get_guid();
                    decl.ast_fields.is_error = is_error;
                    if let Some(name) = child.child_by_field_name("name") {
                        decl.ast_fields.name = code.slice(name.byte_range()).to_string();
                    }
                    if let Some(value) = child.child_by_field_name("value") {
                        decl.type_.inference_info = Some(code.slice(value.byte_range()).to_string());
                        symbols.extend(self.parse_usages(&value, code, path, parent_guid, is_error));
                    }
                    if let Some(dimensions) = child.child_by_field_name("dimensions") {
                        decl.type_ = TypeDef {
                            name: Some(code.slice(dimensions.byte_range()).to_string()),
                            inference_info: None,
                            is_pod: false,
                            namespace: "".to_string(),
                            guid: None,
                            nested_types: vec![local_dtype],
                        };
                    } else {
                        decl.type_ = local_dtype;
                    }
                    symbols.push(Arc::new(RwLock::new(decl)));
                }
                _ => {}
            }
        }
        symbols
    }

    fn parse_enum_field_declaration(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &String, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];
        let mut decl = ClassFieldDeclaration::default();
        decl.ast_fields.language = LanguageId::Java;
        decl.ast_fields.full_range = parent.range();
        decl.ast_fields.file_path = path.clone();
        decl.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
        decl.ast_fields.parent_guid = Some(parent_guid.clone());
        decl.ast_fields.guid = get_guid();
        decl.ast_fields.is_error = is_error;
        if let Some(name) = parent.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name.byte_range()).to_string();
        }
        if let Some(arguments) = parent.child_by_field_name("arguments") {
            decl.type_.inference_info = Some(code.slice(arguments.byte_range()).to_string());
            for i in 0..arguments.child_count() {
                let child = arguments.child(i).unwrap();
                symbols.extend(self.parse_usages(&child, code, path, parent_guid, is_error));
                if let Some(dtype) = parse_type(&child, code) {
                    decl.type_.nested_types.push(dtype);
                }
            }
        }
        symbols.push(Arc::new(RwLock::new(decl)));
        symbols
    }

    pub fn parse_usages(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &String, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];
        let kind = parent.kind();
        #[cfg(test)]
            let text = code.slice(parent.byte_range());
        match kind {
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                symbols.extend(self.parse_struct_declaration(&parent, code, path, parent_guid, is_error));
            }
            "local_variable_declaration" => {
                symbols.extend(self.parse_variable_definition(&parent, code, path, parent_guid, is_error));
            }
            "method_declaration" => {
                symbols.extend(self.parse_function_declaration(&parent, code, path, parent_guid, is_error));
            }
            "method_invocation" | "object_creation_expression" => {
                symbols.extend(self.parse_call_expression(&parent, code, path, parent_guid, is_error));
            }
            "field_declaration" => {
                symbols.extend(self.parse_field_declaration(&parent, code, path, parent_guid, is_error));
            }
            "enum_constant" => {
                symbols.extend(self.parse_enum_field_declaration(&parent, code, path, parent_guid, is_error));
            }
            "identifier" => {
                let mut usage = VariableUsage::default();
                usage.ast_fields.name = code.slice(parent.byte_range()).to_string();
                usage.ast_fields.language = LanguageId::Java;
                usage.ast_fields.full_range = parent.range();
                usage.ast_fields.file_path = path.clone();
                usage.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
                usage.ast_fields.parent_guid = Some(parent_guid.clone());
                usage.ast_fields.guid = get_guid();
                usage.ast_fields.is_error = is_error;
                symbols.push(Arc::new(RwLock::new(usage)));
            }
            "field_access" => {
                let object = parent.child_by_field_name("object").unwrap();
                let usages = self.parse_usages(&object, code, path, parent_guid, is_error);
                let field = parent.child_by_field_name("field").unwrap();
                let mut usage = VariableUsage::default();
                usage.ast_fields.name = code.slice(field.byte_range()).to_string();
                usage.ast_fields.language = LanguageId::Java;
                usage.ast_fields.full_range = parent.range();
                usage.ast_fields.file_path = path.clone();
                usage.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
                usage.ast_fields.parent_guid = Some(parent_guid.clone());
                usage.ast_fields.caller_guid = Some(usages.last().expect("the data might be broken").read().unwrap().guid().to_string());
                symbols.extend(usages);
                symbols.push(Arc::new(RwLock::new(usage)));
            }
            "block_comment" | "line_comment" => {
                let mut def = CommentDefinition::default();
                def.ast_fields.language = LanguageId::Java;
                def.ast_fields.full_range = parent.range();
                def.ast_fields.file_path = path.clone();
                def.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
                def.ast_fields.parent_guid = Some(parent_guid.clone());
                def.ast_fields.guid = get_guid();
                def.ast_fields.is_error = is_error;
                symbols.push(Arc::new(RwLock::new(def)));
            }
            "ERROR" => {
                for i in 0..parent.child_count() {
                    let child = parent.child(i).unwrap();
                    symbols.extend(self.parse_usages(&child, code, path, parent_guid, true));
                }
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

    pub fn parse_function_declaration(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &String, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = FunctionDeclaration::default();
        decl.ast_fields.language = LanguageId::Java;
        decl.ast_fields.full_range = parent.range();
        decl.ast_fields.declaration_range = parent.range();
        decl.ast_fields.definition_range = parent.range();
        decl.ast_fields.file_path = path.clone();
        decl.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
        decl.ast_fields.parent_guid = Some(parent_guid.clone());
        decl.ast_fields.is_error = is_error;
        decl.ast_fields.guid = get_guid();

        if let Some(name_node) = parent.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name_node.byte_range()).to_string();
        }

        if let Some(parameters_node) = parent.child_by_field_name("parameters") {
            decl.ast_fields.declaration_range = Range {
                start_byte: decl.ast_fields.full_range.start_byte,
                end_byte: parameters_node.end_byte(),
                start_point: decl.ast_fields.full_range.start_point,
                end_point: parameters_node.end_position(),
            };

            let params_len = parameters_node.child_count();
            let mut function_args = vec![];
            for idx in 0..params_len {
                let child = parameters_node.child(idx).unwrap();
                function_args.push(parse_function_arg(&child, code));
            }
            decl.args = function_args;
        }
        if let Some(return_type) = parent.child_by_field_name("type") {
            decl.return_type = parse_type(&return_type, code);
        }

        if let Some(body_node) = parent.child_by_field_name("body") {
            decl.ast_fields.definition_range = body_node.range();
            decl.ast_fields.declaration_range = Range {
                start_byte: decl.ast_fields.full_range.start_byte,
                end_byte: decl.ast_fields.definition_range.start_byte,
                start_point: decl.ast_fields.full_range.start_point,
                end_point: decl.ast_fields.definition_range.start_point,
            }
        }
        if let Some(body_node) = parent.child_by_field_name("body") {
            symbols.extend(self.parse_usages(&body_node, code, path, &decl.ast_fields.guid, is_error));
        }

        decl.ast_fields.childs_guid = get_children_guids(&decl.ast_fields.guid, &symbols);
        symbols.push(Arc::new(RwLock::new(decl)));
        symbols
    }

    pub fn parse_call_expression(&mut self, parent: &Node, code: &str, path: &PathBuf, parent_guid: &String, is_error: bool) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = FunctionCall::default();
        decl.ast_fields.language = LanguageId::Python;
        decl.ast_fields.full_range = parent.range();
        decl.ast_fields.file_path = path.clone();
        decl.ast_fields.content_hash = str_hash(&code.slice(parent.byte_range()).to_string());
        decl.ast_fields.parent_guid = Some(parent_guid.clone());
        decl.ast_fields.guid = get_guid();
        decl.ast_fields.is_error = is_error;

        if let Some(name) = parent.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name.byte_range()).to_string();
        }
        if let Some(name) = parent.child_by_field_name("type") {
            decl.ast_fields.name = code.slice(name.byte_range()).to_string();
        }
        if let Some(arguments) = parent.child_by_field_name("arguments") {
            for i in 0..arguments.child_count() {
                let child = arguments.child(i).unwrap();
                symbols.extend(self.parse_usages(&child, code, path, parent_guid, is_error));
            }
        }
        if let Some(object) = parent.child_by_field_name("object") {
            let usages = self.parse_usages(&object, code, path, parent_guid, is_error);
            decl.ast_fields.caller_guid = usages.last().unwrap().read().expect("the data might be broken").fields().parent_guid.clone();
            symbols.extend(usages);
        }

        decl.ast_fields.childs_guid = get_children_guids(&decl.ast_fields.guid, &symbols);
        symbols.push(Arc::new(RwLock::new(decl)));
        symbols
    }
}

impl AstLanguageParser for JavaParser {
    fn parse(&mut self, code: &str, path: &PathBuf) -> Vec<AstSymbolInstanceArc> {
        let tree = self.parser.parse(code, None).unwrap();
        let parent_guid = get_guid();
        let symbols = self.parse_usages(&tree.root_node(), code, path, &parent_guid, false);
        symbols
    }
}
