use std::path::Path;

use anyhow::{Result, bail};
use streaming_iterator::StreamingIterator;
use tracing::debug;

/// A code symbol extracted via tree-sitter.
#[derive(Debug, Clone)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Class,
    Struct,
    Enum,
    Interface,
    Module,
    Constant,
    Method,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Function => write!(f, "function"),
            Self::Class => write!(f, "class"),
            Self::Struct => write!(f, "struct"),
            Self::Enum => write!(f, "enum"),
            Self::Interface => write!(f, "interface"),
            Self::Module => write!(f, "module"),
            Self::Constant => write!(f, "constant"),
            Self::Method => write!(f, "method"),
        }
    }
}

/// Every file extension the indexer can extract symbols from.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "c", "h", "cpp", "cc", "cxx", "hpp",
];

/// True if [`extract_symbols`] knows how to parse this path.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| SUPPORTED_EXTENSIONS.contains(&ext))
}

/// Resolve a file extension to its grammar and symbol query.
fn language_for(ext: &str) -> Option<(tree_sitter::Language, &'static str)> {
    let pair = match ext {
        "rs" => (tree_sitter_rust::LANGUAGE.into(), rust_queries()),
        "py" => (tree_sitter_python::LANGUAGE.into(), python_queries()),
        "js" | "jsx" => (tree_sitter_javascript::LANGUAGE.into(), js_queries()),
        "ts" | "tsx" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            ts_queries(),
        ),
        "go" => (tree_sitter_go::LANGUAGE.into(), go_queries()),
        "java" => (tree_sitter_java::LANGUAGE.into(), java_queries()),
        "c" | "h" => (tree_sitter_c::LANGUAGE.into(), c_queries()),
        "cpp" | "cc" | "cxx" | "hpp" => (tree_sitter_cpp::LANGUAGE.into(), cpp_queries()),
        _ => return None,
    };
    Some(pair)
}

/// Determine the language from file extension and extract symbols.
pub fn extract_symbols(path: &Path) -> Result<Vec<CodeSymbol>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let Some((language, queries)) = language_for(ext) else {
        debug!("unsupported file extension: {ext}");
        bail!("unsupported language: {ext}")
    };

    let source = std::fs::read_to_string(path)?;
    extract_with_language(language, &source, queries)
}

fn extract_with_language(
    language: tree_sitter::Language,
    source: &str,
    queries: &str,
) -> Result<Vec<CodeSymbol>> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language)?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse source"))?;

    // A malformed query is a bug in this file, not a property of the source
    // being parsed — surface it rather than reporting "no symbols" forever.
    let query = tree_sitter::Query::new(&language, queries)
        .map_err(|e| anyhow::anyhow!("doctrack symbol query failed to compile: {e:?}"))?;
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let mut symbols = Vec::new();
    let capture_names = query.capture_names();

    while let Some(m) = matches.next() {
        let mut name = None;
        let mut kind = None;
        let mut start_line = 0;
        let mut end_line = 0;

        for capture in m.captures {
            let capture_name = &capture_names[capture.index as usize];
            let text = &source[capture.node.byte_range()];

            match *capture_name {
                "name" => {
                    name = Some(text.to_string());
                    start_line = capture.node.start_position().row as u32;
                    end_line = capture.node.end_position().row as u32;
                }
                "kind" => {
                    kind = Some(parse_kind(text));
                }
                _ => {}
            }
        }

        if let (Some(name), Some(kind)) = (name, kind) {
            symbols.push(CodeSymbol {
                name,
                kind,
                start_line,
                end_line,
            });
        }
    }

    Ok(symbols)
}

fn parse_kind(text: &str) -> SymbolKind {
    match text {
        "fn" | "func" | "def" | "function" => SymbolKind::Function,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "interface" | "trait" => SymbolKind::Interface,
        "mod" | "module" | "package" => SymbolKind::Module,
        "const" | "static" => SymbolKind::Constant,
        _ => SymbolKind::Function,
    }
}

// --- Tree-sitter queries per language ---

fn rust_queries() -> &'static str {
    r#"
    (function_item name: (identifier) @name (#set! kind "fn")) @kind
    (struct_item name: (type_identifier) @name (#set! kind "struct")) @kind
    (enum_item name: (type_identifier) @name (#set! kind "enum")) @kind
    (trait_item name: (type_identifier) @name (#set! kind "trait")) @kind
    (impl_item trait: (type_identifier) @name (#set! kind "trait")) @kind
    (mod_item name: (identifier) @name (#set! kind "mod")) @kind
    (const_item name: (identifier) @name (#set! kind "const")) @kind
    "#
}

fn python_queries() -> &'static str {
    r#"
    (function_definition name: (identifier) @name (#set! kind "def")) @kind
    (class_definition name: (identifier) @name (#set! kind "class")) @kind
    "#
}

fn js_queries() -> &'static str {
    r#"
    (function_declaration name: (identifier) @name (#set! kind "function")) @kind
    (class_declaration name: (identifier) @name (#set! kind "class")) @kind
    (variable_declarator name: (identifier) @name value: (arrow_function)) @kind
    "#
}

fn ts_queries() -> &'static str {
    // Note: in the TypeScript grammar a class name is a `type_identifier`, not an
    // `identifier` as it is in JavaScript. Getting this wrong doesn't just drop
    // classes — it makes the whole query fail to compile, which silently reduced
    // TypeScript support to zero. `queries_compile` guards against a repeat.
    r#"
    (function_declaration name: (identifier) @name (#set! kind "function")) @kind
    (class_declaration name: (type_identifier) @name (#set! kind "class")) @kind
    (abstract_class_declaration name: (type_identifier) @name (#set! kind "class")) @kind
    (interface_declaration name: (type_identifier) @name (#set! kind "interface")) @kind
    (enum_declaration name: (identifier) @name (#set! kind "enum")) @kind
    (type_alias_declaration name: (type_identifier) @name (#set! kind "interface")) @kind
    "#
}

fn go_queries() -> &'static str {
    r#"
    (function_declaration name: (identifier) @name (#set! kind "func")) @kind
    (method_declaration name: (field_identifier) @name (#set! kind "func")) @kind
    (type_declaration (type_spec name: (type_identifier) @name (#set! kind "struct"))) @kind
    "#
}

fn java_queries() -> &'static str {
    r#"
    (method_declaration name: (identifier) @name (#set! kind "function")) @kind
    (class_declaration name: (identifier) @name (#set! kind "class")) @kind
    (interface_declaration name: (identifier) @name (#set! kind "interface")) @kind
    (enum_declaration name: (identifier) @name (#set! kind "enum")) @kind
    "#
}

fn c_queries() -> &'static str {
    r#"
    (function_definition declarator: (function_declarator declarator: (identifier) @name) (#set! kind "function")) @kind
    (struct_specifier name: (type_identifier) @name (#set! kind "struct")) @kind
    (enum_specifier name: (type_identifier) @name (#set! kind "enum")) @kind
    "#
}

fn cpp_queries() -> &'static str {
    r#"
    (function_definition declarator: (function_declarator declarator: (qualified_identifier) @name) (#set! kind "function")) @kind
    (function_definition declarator: (function_declarator declarator: (identifier) @name) (#set! kind "function")) @kind
    (class_specifier name: (type_identifier) @name (#set! kind "class")) @kind
    (struct_specifier name: (type_identifier) @name (#set! kind "struct")) @kind
    (enum_specifier name: (type_identifier) @name (#set! kind "enum")) @kind
    "#
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str, ext: &str) -> Vec<String> {
        let (language, queries) = language_for(ext).expect("extension should be supported");
        let mut names: Vec<String> = extract_with_language(language, source, queries)
            .unwrap_or_else(|e| panic!("extraction failed for .{ext}: {e}"))
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Regression: the TypeScript query referenced `class_declaration name:
    /// (identifier)`, which isn't a valid node/field pair in that grammar. The
    /// whole query failed to compile, `extract_symbols` returned `Err` for every
    /// `.ts` file, and `Index::build` logged it at debug and moved on — so
    /// TypeScript projects indexed to nothing without any visible error.
    #[test]
    fn every_language_query_compiles() {
        for ext in SUPPORTED_EXTENSIONS {
            let (language, queries) =
                language_for(ext).unwrap_or_else(|| panic!(".{ext} has no grammar mapping"));
            if let Err(e) = tree_sitter::Query::new(&language, queries) {
                panic!("query for .{ext} does not compile: {e:?}");
            }
        }
    }

    #[test]
    fn extracts_typescript_symbols() {
        let names = names(
            "export interface Shape { area(): number }\n\
             export class Circle implements Shape { area() { return 1 } }\n\
             export abstract class Base {}\n\
             export enum Color { Red }\n\
             export type Alias = string;\n\
             export function build(): void {}\n",
            "ts",
        );
        assert_eq!(
            names,
            vec!["Alias", "Base", "Circle", "Color", "Shape", "build"]
        );
    }

    #[test]
    fn extracts_rust_symbols() {
        let names = names(
            "pub struct Session;\npub enum Mode { A }\npub trait Store {}\n\
             pub fn connect() {}\npub const LIMIT: u8 = 1;\npub mod inner {}\n",
            "rs",
        );
        assert_eq!(
            names,
            vec!["LIMIT", "Mode", "Session", "Store", "connect", "inner"]
        );
    }

    #[test]
    fn extracts_python_symbols() {
        assert_eq!(
            names(
                "class Widget:\n    def method(self): pass\n\ndef build(): pass\n",
                "py"
            ),
            vec!["Widget", "build", "method"]
        );
    }

    #[test]
    fn extracts_javascript_symbols() {
        assert_eq!(
            names("export class Circle {}\nexport function build() {}\n", "js"),
            vec!["Circle", "build"]
        );
    }

    #[test]
    fn extracts_go_symbols() {
        assert_eq!(
            names(
                "package main\n\ntype Store struct{}\n\nfunc Run() {}\n",
                "go"
            ),
            vec!["Run", "Store"]
        );
    }

    #[test]
    fn extracts_java_symbols() {
        assert_eq!(
            names(
                "public class Session { public void open() {} }\n\
                 interface Store {}\nenum Mode { A }\n",
                "java"
            ),
            vec!["Mode", "Session", "Store", "open"]
        );
    }

    #[test]
    fn extracts_c_and_cpp_symbols() {
        assert_eq!(
            names(
                "struct Node {};\nenum Mode { A };\nint run(void) { return 0; }\n",
                "c"
            ),
            vec!["Mode", "Node", "run"]
        );
        assert_eq!(
            names(
                "class Session {};\nstruct Node {};\nint run() { return 0; }\n",
                "cpp"
            ),
            vec!["Node", "Session", "run"]
        );
    }

    #[test]
    fn unsupported_extensions_are_rejected() {
        assert!(!is_supported(Path::new("notes.md")));
        assert!(!is_supported(Path::new("Makefile")));
        assert!(is_supported(Path::new("src/main.rs")));
        assert!(is_supported(Path::new("src/app.tsx")));
    }
}
