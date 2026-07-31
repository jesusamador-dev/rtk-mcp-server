//! rtk_grep(query, path?): búsqueda por patrón, nativa.
//!
//! Mismo recorrido que el indexador y que `rtk_find` — respeta `.gitignore` y
//! salta `.git`, `.rtk-index` y el histórico archivado de OpenSpec. Delegar en
//! `rtk grep` daba un universo distinto al de `rtk_find` sobre la MISMA ruta
//! (56 archivos frente a 16), y sin tope: una búsqueda amplia devolvía 29 KB.
//!
//! Cuando hay demasiadas coincidencias para listarlas, degrada a un resumen
//! `archivo: n` en vez de vomitar todas las líneas.

use crate::tools::ToolResult;
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde_json::Value;
use std::path::Path;

/// Por encima de esto no se listan líneas: se devuelve el conteo por archivo.
const MAX_LINES: usize = 60;
/// …y lo mismo si las líneas ocupan demasiado. El tope real es el tamaño de la
/// respuesta, no el número de coincidencias: 43 líneas de tareas pueden ocupar
/// 5 KB mientras 50 líneas de código caben en 2.
const MAX_OUTPUT_CHARS: usize = 4_000;
/// Archivos listados en el modo resumen.
const MAX_FILES_SUMMARY: usize = 30;
/// Recorte de líneas muy largas (minificados, datos embebidos).
const MAX_LINE_CHARS: usize = 200;

struct FileHits {
    path: String,
    hits: Vec<(usize, String)>,
}

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'query' argument")?;
    let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    // `count: true` pide directamente el conteo por archivo — el equivalente a
    // `grep -c`, que para "¿cuánto queda por hacer?" es lo único que importa.
    let count_only = arguments
        .get("count")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !Path::new(path).exists() {
        return Err(format!("La ruta no existe: {}", path));
    }

    let re = RegexBuilder::new(query)
        .build()
        .map_err(|e| format!("Patrón inválido '{}': {}", query, e))?;

    let mut files: Vec<FileHits> = Vec::new();
    let mut total_hits = 0usize;
    // Baseline: lo que habrían ocupado las MISMAS coincidencias en formato
    // `path:línea:texto`, una por línea — o sea, la salida de `grep -rn` sobre
    // este mismo universo. Se calcula aquí, sin lanzar otro proceso.
    let mut raw_chars = 0usize;

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
        let Ok(content) = std::fs::read_to_string(p) else {
            continue; // binario o sin permisos
        };

        let path_str = p.to_string_lossy().trim_start_matches("./").to_string();
        let mut hits = Vec::new();
        for (i, line) in content.lines().enumerate() {
            if re.is_match(line) {
                let text = truncate(line.trim_end(), MAX_LINE_CHARS);
                raw_chars += path_str.len() + text.len() + 8; // "path:NN:texto\n"
                hits.push((i + 1, text));
                total_hits += 1;
            }
        }
        if !hits.is_empty() {
            files.push(FileHits { path: path_str, hits });
        }
    }

    if total_hits == 0 {
        return Ok(ToolResult::text_no_gain(
            format!("Sin coincidencias para '{}' en {}.", query, path),
            query,
        ));
    }

    let lines_chars: usize = files
        .iter()
        .map(|f| f.path.len() + f.hits.iter().map(|(_, t)| t.len() + 8).sum::<usize>())
        .sum();

    // Conteo por archivo: pedido explícitamente, o porque listar las líneas
    // saldría caro. NO se reporta ahorro — la información no es equivalente a
    // listar las líneas, así que compararla sería tramposo.
    if count_only || total_hits > MAX_LINES || lines_chars > MAX_OUTPUT_CHARS {
        files.sort_by_key(|f| std::cmp::Reverse(f.hits.len()));
        let shown = files.len().min(MAX_FILES_SUMMARY);
        let mut out = vec![format!(
            "{} coincidencias en {} archivo(s) para '{}'{}",
            total_hits,
            files.len(),
            query,
            if count_only {
                " — conteo por archivo:"
            } else {
                " — demasiadas para listarlas, aquí el conteo:"
            }
        )];
        for f in files.iter().take(shown) {
            out.push(format!("{}: {}", f.path, f.hits.len()));
        }
        if files.len() > shown {
            out.push(format!("[+{} archivos más]", files.len() - shown));
        }
        if !count_only {
            out.push(
                "Acota con un `path` más específico o un patrón más preciso para ver las líneas."
                    .to_string(),
            );
        }
        return Ok(ToolResult::text_no_gain(out.join("\n"), query));
    }

    // Pocas: las líneas, con la ruta una sola vez por archivo.
    let mut out = vec![format!(
        "{} coincidencia(s) en {} archivo(s) para '{}'",
        total_hits,
        files.len(),
        query
    )];
    for f in &files {
        out.push(f.path.clone());
        for (n, text) in &f.hits {
            out.push(format!("  {}: {}", n, text));
        }
    }

    Ok(ToolResult::text(
        out.join("\n"),
        raw_chars,
        "grep -rn (mismas líneas, ruta repetida)",
        query,
    ))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}…", cut)
    }
}
