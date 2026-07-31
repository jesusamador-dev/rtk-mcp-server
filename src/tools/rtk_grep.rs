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

    for entry in WalkBuilder::new(path)
        .hidden(false)
        .require_git(false) // mismo criterio que rtk_find y el indexador
        .build()
        .flatten()
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{args, text_of, TmpRepo};
    use serde_json::json;

    fn repo_con_archive() -> TmpRepo {
        let r = TmpRepo::new("grep");
        r.write("openspec/changes/activo/tasks.md", "# T\n- [ ] pendiente uno\n- [x] hecha\n");
        r.write("openspec/changes/archive/viejo/tasks.md", "# V\n- [ ] ruido archivado\n");
        r.write("src/lib.rs", "// - [ ] tarea en código\n");
        r
    }

    /// El fallo de la v1.3: rtk_find veía 16 archivos y rtk_grep 56 sobre la
    /// MISMA ruta, porque grep se metía en el histórico archivado.
    #[test]
    fn mismo_alcance_que_rtk_find() {
        let repo = repo_con_archive();
        let g = text_of(
            &execute(&args(json!({"query": r"- \[ \]", "path": repo.root_str()}))).unwrap(),
        );
        assert!(g.contains("activo/tasks.md"));
        assert!(g.contains("src/lib.rs"));
        assert!(!g.contains("archive"), "el histórico archivado no se busca");

        let f = text_of(
            &crate::tools::rtk_find::execute(&args(json!({"path": repo.root_str()}))).unwrap(),
        );
        assert!(!f.contains("archive"), "y rtk_find tampoco: un solo universo");
    }

    /// Antes: el `-` inicial lo comía grep como flag y devolvía vacío con rc=0.
    #[test]
    fn patron_que_empieza_por_guion() {
        let repo = repo_con_archive();
        let out = text_of(
            &execute(&args(json!({"query": r"- \[ \]", "path": repo.path("src/lib.rs")}))).unwrap(),
        );
        assert!(out.contains("tarea en código"));
    }

    #[test]
    fn modo_conteo_es_mucho_mas_barato_que_listar() {
        let repo = TmpRepo::new("grep-count");
        let muchas: String = (0..40).map(|i| format!("- [ ] tarea número {} con bastante texto para ocupar sitio\n", i)).collect();
        repo.write("tasks.md", &muchas);

        let listado = text_of(&execute(&args(json!({"query": r"- \[ \]", "path": repo.root_str()}))).unwrap());
        let conteo = text_of(&execute(&args(json!({"query": r"- \[ \]", "path": repo.root_str(), "count": true}))).unwrap());

        assert!(conteo.contains("40 coincidencias"));
        assert!(conteo.contains("tasks.md: 40"));
        assert!(conteo.len() < listado.len() / 2, "el conteo debe ser mucho menor");
        assert!(!conteo.contains("tarea número"), "el conteo no lista líneas");
    }

    /// El tope es por bytes: 43 líneas de tareas ocupan más que 50 de código.
    #[test]
    fn degrada_a_conteo_cuando_las_lineas_ocuparian_demasiado() {
        let repo = TmpRepo::new("grep-tope");
        let largas: String = (0..30)
            .map(|i| format!("- [ ] {} {}\n", i, "texto muy largo ".repeat(20)))
            .collect();
        repo.write("tasks.md", &largas);
        let out = text_of(&execute(&args(json!({"query": r"- \[ \]", "path": repo.root_str()}))).unwrap());
        assert!(out.contains("demasiadas para listarlas"));
        assert!(out.len() < MAX_OUTPUT_CHARS, "la respuesta queda acotada");
    }

    #[test]
    fn sin_coincidencias_y_errores_se_distinguen() {
        let repo = repo_con_archive();
        let vacio = text_of(&execute(&args(json!({"query": "cadena_que_no_existe", "path": repo.root_str()}))).unwrap());
        assert!(vacio.contains("Sin coincidencias"));

        // Antes esto devolvía "no encontrado"; ahora es un error explícito.
        assert!(execute(&args(json!({"query": "x", "path": "/ruta/que/no/existe"}))).is_err());
        assert!(execute(&args(json!({"query": "[[[", "path": repo.root_str()}))).is_err(), "regex inválida");
        assert!(execute(&args(json!({"path": repo.root_str()}))).is_err(), "falta query");
    }

    #[test]
    fn el_ahorro_no_se_reporta_cuando_la_informacion_no_es_equivalente() {
        let repo = TmpRepo::new("grep-metrica");
        repo.write("a.md", "- [ ] una\n- [ ] dos\n");
        let listado = execute(&args(json!({"query": r"- \[ \]", "path": repo.root_str()}))).unwrap();
        assert!(listado.baseline_chars > 0);
        assert_ne!(listado.baseline_label, "sin alternativa más barata");

        let conteo = execute(&args(json!({"query": r"- \[ \]", "path": repo.root_str(), "count": true}))).unwrap();
        let salida = text_of(&conteo).len();
        assert_eq!(
            conteo.baseline_chars, salida,
            "en modo conteo el baseline es la propia salida: 0% de ahorro"
        );
    }
}
