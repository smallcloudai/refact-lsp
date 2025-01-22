use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::string::ToString;
use std::sync::Arc;
#[allow(unused_imports)]
use itertools::Itertools;
use parking_lot::RwLock;

use tree_sitter::{Node, Parser, Range};
use tree_sitter_php::language as php_language;
use uuid::Uuid;

use crate::ast::treesitter::ast_instance_structs::{
    AstSymbolFields, AstSymbolInstanceArc, ClassFieldDeclaration, CommentDefinition,
    FunctionArg, FunctionCall, FunctionDeclaration, StructDeclaration, TypeDef,
    VariableDefinition, VariableUsage
};
use crate::ast::treesitter::language_id::LanguageId;
use crate::ast::treesitter::parsers::{AstLanguageParser, internal_error, ParserError};
use crate::ast::treesitter::parsers::utils::{CandidateInfo, get_guid};

pub(crate) struct PHPParser {
    pub parser: Parser,
}

pub fn parse_type(parent: &Node, code: &str) -> Option<TypeDef> {
    let kind = parent.kind();
    let text = code.slice(parent.byte_range()).to_string();
    match kind {
        "type_identifier" | "qualified_name" | "identifier" => {
            return Some(TypeDef {
                name: Some(text),
                inference_info: None,
                inference_info_guid: None,
                is_pod: false,
                namespace: "".to_string(),
                guid: None,
                nested_types: vec![],
            });
        }
        _ => {}
    }
    None
}

impl PHPParser {
    pub fn new() -> Result<Self, ParserError> {
        let mut parser = Parser::new();
        parser
            .set_language(&php_language())
            .map_err(internal_error)?;
        Ok(Self { parser })
    }

    pub fn parse_struct_declaration<'a>(
        &mut self,
        info: &CandidateInfo<'a>,
        code: &str,
        candidates: &mut VecDeque<CandidateInfo<'a>>
    ) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = StructDeclaration::default();

        decl.ast_fields = AstSymbolFields::from_fields(&info.ast_fields);
        decl.ast_fields.full_range = info.node.range();
        decl.ast_fields.declaration_range = info.node.range();
        decl.ast_fields.definition_range = info.node.range();
        decl.ast_fields.parent_guid = Some(info.parent_guid.clone());
        decl.ast_fields.guid = get_guid();

        if let Some(name) = info.node.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name.byte_range()).to_string();
        } else {
            decl.ast_fields.name = format!("anon-{}", decl.ast_fields.guid);
        }

        if let Some(base_class) = info.node.child_by_field_name("base_class") {
            if let Some(dtype) = parse_type(&base_class, code) {
                decl.inherited_types.push(dtype);
            }
        }

        if let Some(body) = info.node.child_by_field_name("body") {
            decl.ast_fields.definition_range = body.range();
            candidates.push_back(CandidateInfo {
                ast_fields: decl.ast_fields.clone(),
                node: body,
                parent_guid: decl.ast_fields.guid.clone(),
            });
        }

        symbols.push(Arc::new(RwLock::new(Box::new(decl))));
        symbols
    }

    fn parse_variable_definition<'a>(
        &mut self,
        info: &CandidateInfo<'a>,
        code: &str,
        _: &mut VecDeque<CandidateInfo<'a>>
    ) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];

        let mut decl = VariableDefinition::default();
        decl.ast_fields = AstSymbolFields::from_fields(&info.ast_fields);
        decl.ast_fields.full_range = info.node.range();
        decl.ast_fields.declaration_range = info.node.range();
        decl.ast_fields.definition_range = info.node.range();
        decl.ast_fields.parent_guid = Some(info.parent_guid.clone());
        decl.ast_fields.guid = get_guid();

        if let Some(name) = info.node.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name.byte_range()).to_string();
        }

        if let Some(type_node) = info.node.child_by_field_name("type") {
            if let Some(type_) = parse_type(&type_node, code) {
                decl.type_ = type_;
            }
        }

        symbols.push(Arc::new(RwLock::new(Box::new(decl))));
        symbols
    }

    pub fn parse_function_declaration<'a>(
        &mut self,
        info: &CandidateInfo<'a>,
        code: &str,
        candidates: &mut VecDeque<CandidateInfo<'a>>
    ) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut decl = FunctionDeclaration::default();
        decl.ast_fields = AstSymbolFields::from_fields(&info.ast_fields);
        decl.ast_fields.full_range = info.node.range();
        decl.ast_fields.declaration_range = info.node.range();
        decl.ast_fields.definition_range = info.node.range();
        decl.ast_fields.parent_guid = Some(info.parent_guid.clone());
        decl.ast_fields.guid = get_guid();

        if let Some(name) = info.node.child_by_field_name("name") {
            decl.ast_fields.name = code.slice(name.byte_range()).to_string();
        }

        if let Some(parameters) = info.node.child_by_field_name("parameters") {
            for i in 0..parameters.child_count() {
                let child = parameters.child(i).unwrap();
                let mut arg = FunctionArg::default();
                if let Some(name) = child.child_by_field_name("name") {
                    arg.name = code.slice(name.byte_range()).to_string();
                }
                if let Some(type_) = child.child_by_field_name("type") {
                    arg.type_ = parse_type(&type_, code);
                }
                decl.args.push(arg);
            }
        }

        if let Some(return_type) = info.node.child_by_field_name("return_type") {
            decl.return_type = parse_type(&return_type, code);
        }

        if let Some(body_node) = info.node.child_by_field_name("body") {
            decl.ast_fields.definition_range = body_node.range();
            candidates.push_back(CandidateInfo {
                ast_fields: decl.ast_fields.clone(),
                node: body_node,
                parent_guid: decl.ast_fields.guid.clone(),
            });
        }

        symbols.push(Arc::new(RwLock::new(Box::new(decl))));
        symbols
    }

    fn parse_usages<'a>(
        &mut self,
        info: &CandidateInfo<'a>,
        code: &str,
        candidates: &mut VecDeque<CandidateInfo<'a>>
    ) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = vec![];

        let kind = info.node.kind();
        match kind {
            "class_declaration" | "interface_declaration" => {
                symbols.extend(self.parse_struct_declaration(info, code, candidates));
            }
            "variable_declaration" => {
                symbols.extend(self.parse_variable_definition(info, code, candidates));
            }
            "function_declaration" => {
                symbols.extend(self.parse_function_declaration(info, code, candidates));
            }
            _ => {
                for i in 0..info.node.child_count() {
                    let child = info.node.child(i).unwrap();
                    candidates.push_back(CandidateInfo {
                        ast_fields: info.ast_fields.clone(),
                        node: child,
                        parent_guid: info.parent_guid.clone(),
                    });
                }
            }
        }
        symbols
    }

    fn parse_(&mut self, parent: &Node, code: &str, path: &PathBuf) -> Vec<AstSymbolInstanceArc> {
        let mut symbols: Vec<AstSymbolInstanceArc> = Default::default();
        let mut ast_fields = AstSymbolFields::default();
        ast_fields.file_path = path.clone();
        ast_fields.language = LanguageId::PHP;

        let mut candidates = VecDeque::from(vec![CandidateInfo {
            ast_fields,
            node: parent.clone(),
            parent_guid: get_guid(),
        }]);

        while let Some(candidate) = candidates.pop_front() {
            symbols.extend(self.parse_usages(&candidate, code, &mut candidates));
        }

        symbols
    }
}

impl AstLanguageParser for PHPParser {
    fn parse(&mut self, code: &str, path: &PathBuf) -> Vec<AstSymbolInstanceArc> {
        let tree = self.parser.parse(code, None).unwrap();
        self.parse_(&tree.root_node(), code, path)
    }
}
