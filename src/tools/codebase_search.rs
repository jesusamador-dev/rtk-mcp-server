//! codebase_search(query, path?, k?): recuperación BM25 sobre los chunks del
//! workspace. Devuelve los k fragmentos más relevantes con su ancla path:líneas
//! y el código, en una sola llamada — reemplaza el baile grep → read → read.

use crate::index;
use crate::tools::{self, ToolResult};
use serde_json::Value;

const DEFAULT_K: usize = 8;
const MAX_LINES_PER_HIT: usize = 60; // acota tokens por fragmento

/// Presupuesto de salida para el conjunto: con k=25 y 60 líneas por fragmento,
/// el tope por fragmento no impide una respuesta enorme. Agotado, los últimos
/// resultados se listan solo con su ancla.
const OUTPUT_BUDGET: usize = 8_000;

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'query' argument")?;
    let root = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(crate::context::default_root);
    let root = root.as_str();
    let k = arguments
        .get("k")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_K)
        .clamp(1, 25);

    const POOL: usize = 30; // candidatos por vía antes de fusionar

    let (hits, freshness) = index::with_workspace(root, |ws| {
        let stats = ws.sync()?;

        // Vía léxica (BM25) — siempre disponible.
        let bm25 = ws.search_bm25(query, POOL)?;

        // Vía semántica (embeddings) — perezosa y acotada; si falla, degradamos a BM25.
        let mut mode = String::from("híbrido (BM25 + semántico)");
        let semantic = match ws.ensure_vectors() {
            Ok(vstats) => {
                if vstats.remaining > 0 {
                    mode = format!(
                        "híbrido · índice semántico calentando ({} pendientes)",
                        vstats.remaining
                    );
                }
                ws.search_semantic(query, POOL).unwrap_or_default()
            }
            Err(e) => {
                eprintln!("[codebase_search] semántico desactivado: {}", e);
                mode = String::from("BM25 (semántico no disponible)");
                Vec::new()
            }
        };

        let freshness = format!(
            "[{} · {} archivos · {} símbolos vectorizados]",
            mode,
            stats.total_files,
            ws.vector_count()
        );
        Ok((index::rrf(&[bm25, semantic], k), freshness))
    })?;

    if hits.is_empty() {
        return Ok(ToolResult::text_no_gain(
            format!("Sin coincidencias para: \"{}\"\n{}", query, freshness),
            query,
        ));
    }

    // Baseline: sin la herramienta, localizar esto habría implicado abrir
    // enteros los archivos donde caen los resultados.
    let baseline = tools::full_files_chars(hits.iter().map(|h| h.path.as_str()));

    let mut out = vec![format!(
        "{} resultado(s) para \"{}\"  {}",
        hits.len(),
        query,
        freshness
    )];

    let mut used = 0usize;
    let mut anchors_only = 0usize;
    for (i, h) in hits.iter().enumerate() {
        out.push(String::new());
        out.push(format!(
            "#{} ── {} · {} · {}:L{}-{}",
            i + 1,
            h.name,
            h.kind,
            h.path,
            h.start_line,
            h.end_line
        ));
        if used >= OUTPUT_BUDGET {
            anchors_only += 1;
            continue; // ancla ya impresa arriba; el cuerpo no cabe
        }
        let capped_end = (h.start_line + MAX_LINES_PER_HIT).min(h.end_line);
        match index::read_line_range(&h.path, h.start_line, capped_end) {
            Ok(code) => {
                let code = code.trim_end().to_string();
                used += code.len();
                out.push(code);
                if capped_end < h.end_line {
                    out.push(format!(
                        "   … (+{} líneas; usa symbol_lookup(\"{}\") para el cuerpo completo)",
                        h.end_line - capped_end,
                        h.name
                    ));
                }
            }
            Err(e) => out.push(format!("[no se pudo leer: {}]", e)),
        }
    }

    if anchors_only > 0 {
        out.push(String::new());
        out.push(format!(
            "[{} resultado(s) más, solo con su ancla (presupuesto de salida): pídelos con \
             symbol_lookup o acota la consulta]",
            anchors_only
        ));
    }

    Ok(ToolResult::text(
        out.join("\n"),
        baseline,
        "leer enteros los archivos con resultados",
        query,
    ))
}
