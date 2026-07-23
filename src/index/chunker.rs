//! Extracción de símbolos con tree-sitter.
//!
//! Parte el código en unidades con sentido (funciones, clases, structs…) en vez
//! de líneas fijas. Cada símbolo lleva su rango exacto de líneas para que la
//! recuperación alimente directo a una edición, sin necesidad de releer el archivo.

use std::path::Path;
use tree_sitter::{Language, Node, Parser};

#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub signature: String,
    pub start_line: usize, // 1-indexed
    pub end_line: usize,   // 1-indexed
}

// Tipos de nodo que consideramos "símbolo" por lenguaje. Los nodos sin campo
// "name" (p. ej. impl_item de Rust) se saltan solos, pero la recursión sigue
// descendiendo para capturar los métodos internos.
const RUST_KINDS: &[&str] = &[
    "function_item", "struct_item", "enum_item", "trait_item", "mod_item",
    "const_item", "static_item", "type_item", "macro_definition",
];

const TS_KINDS: &[&str] = &[
    "function_declaration", "method_definition", "class_declaration",
    "abstract_class_declaration", "interface_declaration", "type_alias_declaration",
    "enum_declaration", "public_field_definition",
];

const PY_KINDS: &[&str] = &["function_definition", "class_definition"];

fn language_for(ext: &str) -> Option<(Language, &'static [&'static str])> {
    match ext {
        "rs" => Some((tree_sitter_rust::LANGUAGE.into(), RUST_KINDS)),
        "ts" => Some((tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), TS_KINDS)),
        "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
            Some((tree_sitter_typescript::LANGUAGE_TSX.into(), TS_KINDS))
        }
        "py" | "pyi" => Some((tree_sitter_python::LANGUAGE.into(), PY_KINDS)),
        _ => None,
    }
}

pub fn is_supported(ext: &str) -> bool {
    language_for(ext).is_some()
}

pub fn parse(path: &Path, source: &str) -> Vec<Symbol> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let (lang, kinds) = match language_for(ext) {
        Some(x) => x,
        None => return Vec::new(),
    };

    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let src = source.as_bytes();
    let mut out = Vec::new();
    collect(tree.root_node(), src, kinds, &mut out);
    out
}

fn collect(node: Node, src: &[u8], kinds: &[&str], out: &mut Vec<Symbol>) {
    if kinds.contains(&node.kind()) {
        if let Some(sym) = to_symbol(node, src) {
            out.push(sym);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, src, kinds, out);
    }
}

fn to_symbol(node: Node, src: &[u8]) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(src).ok()?.to_string();
    let full = node.utf8_text(src).ok()?;
    // Firma = primera línea del nodo (encabezado sin cuerpo), acotada.
    let mut signature = full.lines().next().unwrap_or("").trim().to_string();
    if signature.len() > 200 {
        signature.truncate(200);
        signature.push('…');
    }
    Some(Symbol {
        kind: node.kind().to_string(),
        name,
        signature,
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
    })
}
