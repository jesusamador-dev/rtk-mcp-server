pub mod bulk_read;
pub mod git_diff;
pub mod rtk_grep;
pub mod rtk_find;
pub mod file_outline;
pub mod symbol_lookup;
pub mod codebase_search;

use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;

/// Resultado de una herramienta más lo necesario para medir su ahorro.
pub struct ToolResult {
    /// Payload MCP (`{"content": [...]}`).
    pub value: Value,
    /// Caracteres que habría costado obtener lo mismo sin la herramienta:
    /// los archivos completos, la salida cruda de grep/find/diff… Cuando no se
    /// puede medir se usa el tamaño de la propia respuesta (0 % de ahorro), de
    /// modo que la telemetría nunca infle el resultado.
    pub baseline_chars: usize,
    /// Etiqueta corta para el historial (la query, el símbolo, el archivo…).
    pub detail: Option<String>,
}

impl ToolResult {
    /// Respuesta de texto con un baseline explícito.
    pub fn text(body: String, baseline_chars: usize, detail: impl Into<String>) -> Self {
        ToolResult {
            value: json!({ "content": [{ "type": "text", "text": body }] }),
            baseline_chars,
            detail: Some(detail.into()),
        }
    }

    /// Respuesta de texto sin ahorro medible: el baseline es la propia salida.
    pub fn text_no_gain(body: String, detail: impl Into<String>) -> Self {
        let baseline = body.len();
        ToolResult::text(body, baseline, detail)
    }
}

/// Suma el tamaño en disco de un conjunto de archivos (sin repetir), sin
/// leerlos: es lo que habría costado abrirlos enteros con `Read`.
pub fn full_files_chars<'a>(paths: impl IntoIterator<Item = &'a str>) -> usize {
    let mut seen = HashSet::new();
    let mut total = 0usize;
    for p in paths {
        if !seen.insert(p.to_string()) {
            continue;
        }
        if let Ok(m) = fs::metadata(p) {
            total += m.len() as usize;
        }
    }
    total
}
