//! file_outline(path): firmas de un archivo (sin cuerpos), para orientarse
//! sin pagar el costo de leer el archivo completo.

use crate::index::chunker;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub fn execute(params: &Value) -> Result<Value, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'path' argument")?;

    let content = fs::read_to_string(path).map_err(|e| format!("Could not read file: {}", e))?;
    let symbols = chunker::parse(Path::new(path), &content);

    if symbols.is_empty() {
        return Ok(text(format!(
            "No symbols found in {} (lenguaje no soportado o archivo sin símbolos).",
            path
        )));
    }

    let mut lines = vec![format!("Outline: {} ({} símbolos)", path, symbols.len())];
    for s in &symbols {
        lines.push(format!(
            "L{:<5} {:<26} {}",
            s.start_line,
            s.kind,
            truncate(&s.signature, 100)
        ));
    }
    Ok(text(lines.join("\n")))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}…", cut)
    }
}

fn text(body: String) -> Value {
    json!({ "content": [{ "type": "text", "text": body }] })
}
