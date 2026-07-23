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
    is_markdown(ext) || language_for(ext).is_some()
}

fn is_markdown(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdx")
}

pub fn parse(path: &Path, source: &str) -> Vec<Symbol> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Markdown (specs OpenSpec/Spec Kit, docs, README): secciones por encabezado.
    if is_markdown(ext) {
        return parse_markdown(source);
    }

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
        // Retrocede a un límite de carácter válido para no partir un multibyte.
        let mut end = 200;
        while !signature.is_char_boundary(end) {
            end -= 1;
        }
        signature.truncate(end);
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

// ---- Markdown: cada encabezado ATX es una sección/símbolo ----

struct Heading {
    line0: usize, // índice de línea 0-based del encabezado
    level: usize, // 1..=6
    text: String,
}

fn parse_markdown(source: &str) -> Vec<Symbol> {
    let lines: Vec<&str> = source.lines().collect();
    let total = lines.len();

    // 1) Recolectar encabezados, ignorando los que están dentro de fences ``` o ~~~.
    let mut heads: Vec<Heading> = Vec::new();
    let mut in_fence = false;
    for (i, raw) in lines.iter().enumerate() {
        let t = raw.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) {
            let rest = &t[hashes..];
            if rest.is_empty() || rest.starts_with(' ') {
                let text = rest.trim().trim_end_matches('#').trim().to_string();
                if !text.is_empty() {
                    heads.push(Heading {
                        line0: i,
                        level: hashes,
                        text,
                    });
                }
            }
        }
    }

    // 2) Cada encabezado → símbolo cuya sección termina antes del próximo
    //    encabezado de nivel igual o superior (o EOF).
    let mut out = Vec::with_capacity(heads.len());
    for (idx, h) in heads.iter().enumerate() {
        let mut end_line = total; // 1-based última línea si no hay siguiente
        for next in &heads[idx + 1..] {
            if next.level <= h.level {
                end_line = next.line0; // 0-based del siguiente == 1-based de la línea previa
                break;
            }
        }
        let start_line = h.line0 + 1;
        if end_line < start_line {
            end_line = start_line;
        }
        out.push(Symbol {
            kind: format!("h{}", h.level),
            name: h.text.clone(),
            signature: cap_chars(&format!("{} {}", "#".repeat(h.level), h.text), 200),
            start_line,
            end_line,
        });
    }
    out
}

fn cap_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
