//! symbol_lookup(name, path?): devuelve la definición exacta de un símbolo
//! (una función/clase/struct), con su ancla path:líneas, sin leer el archivo
//! entero. Usa el índice persistente y lo sincroniza incrementalmente antes.

use crate::index;
use crate::tools::{self, ToolResult};
use serde_json::Value;

const MAX_BODIES: usize = 6; // límite de cuerpos completos para acotar tokens

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let name = arguments
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'name' argument")?;
    let root = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(crate::context::default_root);
    let root = root.as_str();

    let mut ws = index::Workspace::open(root)?;
    let stats = ws.sync()?;
    let rows = ws.lookup_symbol(name)?;

    let freshness = format!(
        "[índice: {} archivos, {} re-indexados, {} eliminados]",
        stats.total_files, stats.reindexed, stats.removed
    );

    if rows.is_empty() {
        return Ok(ToolResult::text_no_gain(
            format!(
                "Ningún símbolo llamado '{}' encontrado.\n{} ({} símbolos totales)",
                name,
                freshness,
                ws.symbol_count()
            ),
            name,
        ));
    }

    // Baseline: sin el índice habría que abrir enteros todos los archivos que
    // contienen alguna definición del símbolo.
    let baseline = tools::full_files_chars(rows.iter().map(|r| r.path.as_str()));

    let mut out = vec![format!(
        "{} coincidencia(s) para '{}'  {}",
        rows.len(),
        name,
        freshness
    )];

    for (i, r) in rows.iter().enumerate() {
        out.push(String::new());
        out.push(format!(
            "── {} · {} · {}:L{}-{}",
            r.name, r.kind, r.path, r.start_line, r.end_line
        ));
        if i < MAX_BODIES {
            match index::read_line_range(&r.path, r.start_line, r.end_line) {
                Ok(code) => out.push(code.trim_end().to_string()),
                Err(e) => out.push(format!("[no se pudo leer el cuerpo: {}]", e)),
            }
        } else {
            out.push(format!("   {}", r.signature));
        }
    }

    if rows.len() > MAX_BODIES {
        out.push(String::new());
        out.push(format!(
            "[+{} coincidencias mostradas solo como firma; refina el nombre para ver su cuerpo]",
            rows.len() - MAX_BODIES
        ));
    }

    Ok(ToolResult::text(out.join("\n"), baseline, name))
}
