//! symbol_lookup(name, path?): devuelve la definición exacta de un símbolo
//! (una función/clase/struct), con su ancla path:líneas, sin leer el archivo
//! entero. Usa el índice persistente y lo sincroniza incrementalmente antes.

use crate::index;
use crate::tools::{self, ToolResult};
use serde_json::Value;

const MAX_BODIES: usize = 6; // límite de cuerpos completos para acotar tokens

/// Presupuesto de salida. `MAX_BODIES` por sí solo no acota nada: seis cuerpos
/// grandes son 19 KB. Al agotarse, el resto se muestra como firma.
const OUTPUT_BUDGET: usize = 5_000;

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

    let (stats, rows, symbol_count) = index::with_workspace(root, |ws| {
        let stats = ws.sync()?;
        let rows = ws.lookup_symbol(name)?;
        let count = ws.symbol_count();
        Ok((stats, rows, count))
    })?;

    let freshness = format!(
        "[índice: {} archivos, {} re-indexados, {} eliminados]",
        stats.total_files, stats.reindexed, stats.removed
    );

    if rows.is_empty() {
        return Ok(ToolResult::text_no_gain(
            format!(
                "Ningún símbolo llamado '{}' encontrado.\n{} ({} símbolos totales)",
                name, freshness, symbol_count
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

    let mut used = 0usize;
    let mut only_signature = 0usize;
    for (i, r) in rows.iter().enumerate() {
        out.push(String::new());
        out.push(format!(
            "── {} · {} · {}:L{}-{}",
            r.name, r.kind, r.path, r.start_line, r.end_line
        ));
        if i < MAX_BODIES && used < OUTPUT_BUDGET {
            match index::read_line_range(&r.path, r.start_line, r.end_line) {
                Ok(code) => {
                    let code = code.trim_end().to_string();
                    used += code.len();
                    out.push(code);
                }
                Err(e) => out.push(format!("[no se pudo leer el cuerpo: {}]", e)),
            }
        } else {
            only_signature += 1;
            out.push(format!("   {}", r.signature));
        }
    }

    if only_signature > 0 {
        out.push(String::new());
        out.push(format!(
            "[+{} coincidencias solo como firma (presupuesto de salida); refina el nombre \
             o usa el ancla path:líneas para ver un cuerpo concreto]",
            only_signature
        ));
    }

    Ok(ToolResult::text(
        out.join("\n"),
        baseline,
        "leer enteros los archivos con el símbolo",
        name,
    ))
}
