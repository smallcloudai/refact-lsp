#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::ast::treesitter::parsers::AstLanguageParser;
    use crate::ast::treesitter::parsers::csharp::CSharpParser;
    use crate::ast::treesitter::parsers::tests::base_test;

    const MAIN_CSHARP_CODE: &str = include_str!("cases/csharp/main.cs");
    const MAIN_CSHARP_SYMBOLS: &str = include_str!("cases/csharp/main.cs.json");

    #[test]
    fn test_query_csharp_function() {
        let mut parser: Box<dyn AstLanguageParser> = Box::new(CSharpParser::new().expect("CSharpParser::new"));
        let path = PathBuf::from("file:///main.cs");
        base_test(&mut parser, &path, MAIN_CSHARP_CODE, MAIN_CSHARP_SYMBOLS);
    }
}
