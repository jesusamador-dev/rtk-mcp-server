//! file_outline(path): firmas de un archivo (sin cuerpos), para orientarse
//! sin pagar el costo de leer el archivo completo.

use crate::index::chunker;
use crate::tools::ToolResult;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'path' argument")?;

    let content = fs::read_to_string(path).map_err(|e| format!("Could not read file: {}", e))?;
    let symbols = chunker::parse(Path::new(path), &content);

    // Baseline: sin la herramienta habría que leer el archivo entero.
    let baseline = content.len();

    if symbols.is_empty() {
        return Ok(ToolResult::text(
            format!(
                "No symbols found in {} (lenguaje no soportado o archivo sin símbolos).",
                path
            ),
            baseline,
            path,
        ));
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
    Ok(ToolResult::text(lines.join("\n"), baseline, path))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}…", cut)
    }
}
