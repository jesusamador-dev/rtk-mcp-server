//! rtk_find(path?, name?): lista archivos de forma compacta.
//!
//! Implementación nativa con `ignore` — el mismo recorrido que usa el
//! indexador. Respeta `.gitignore`, salta `.rtk-index/` y el histórico de
//! OpenSpec, y agrupa por directorio para no repetir el prefijo en cada línea.
//! Delegar en `rtk find` traía el árbol completo (incluido `archive/`) y, sin
//! patrón, no devolvía nada.

use crate::tools::ToolResult;
use globset::GlobBuilder;
use ignore::WalkBuilder;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Tope de archivos listados. Si se supera, se dice explícitamente cuántos
/// quedaron fuera: un truncado silencioso se leería como "esto es todo".
const MAX_FILES: usize = 400;

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let name_pattern = arguments.get("name").and_then(|v| v.as_str());

    if !Path::new(path).exists() {
        return Err(format!("La ruta no existe: {}", path));
    }

    let matcher = match name_pattern {
        Some(p) => Some(
            GlobBuilder::new(p)
                .literal_separator(false)
                .build()
                .map_err(|e| format!("Patrón inválido '{}': {}", p, e))?
                .compile_matcher(),
        ),
        None => None,
    };

    // dir → archivos, para agrupar la salida.
    let mut by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut total = 0usize;
    let mut listed = 0usize;
    let mut raw_chars = 0usize; // baseline: una ruta completa por línea

    for entry in WalkBuilder::new(path).hidden(false).build().flatten() {
        if !entry.file_type().map_or(false, |ft| ft.is_file()) {
            continue;
        }
        let p = entry.path();
        if p.components().any(|c| c.as_os_str() == ".rtk-index" || c.as_os_str() == ".git") {
            continue;
        }
        if p.to_string_lossy().contains("openspec/changes/archive/") {
            continue;
        }
        let file_name = match p.file_name().and_then(|f| f.to_str()) {
            Some(f) => f,
            None => continue,
        };
        if let Some(m) = &matcher {
            if !m.is_match(file_name) {
                continue;
            }
        }

        total += 1;
        raw_chars += p.to_string_lossy().len() + 1;
        if listed >= MAX_FILES {
            continue;
        }
        listed += 1;
        let dir = p
            .parent()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        by_dir.entry(dir).or_default().push(file_name.to_string());
    }

    let detail = match name_pattern {
        Some(p) => format!("{} -name {}", path, p),
        None => path.to_string(),
    };

    if total == 0 {
        return Ok(ToolResult::text_no_gain(
            format!(
                "Sin archivos en '{}'{}.",
                path,
                name_pattern.map(|p| format!(" que coincidan con '{}'", p)).unwrap_or_default()
            ),
            detail,
        ));
    }

    let mut out = vec![format!(
        "{} archivo(s) en {}{}",
        total,
        path,
        name_pattern.map(|p| format!(" · patrón {}", p)).unwrap_or_default()
    )];
    for (dir, mut files) in by_dir {
        files.sort();
        out.push(format!("{}/: {}", dir.trim_start_matches("./"), files.join(", ")));
    }
    if total > listed {
        out.push(format!(
            "… +{} archivos no listados (tope {}); acota con `name` o un `path` más específico",
            total - listed,
            MAX_FILES
        ));
    }

    Ok(ToolResult::text(
        out.join("\n"),
        raw_chars,
        "una ruta por línea",
        detail,
    ))
}
