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

    for entry in WalkBuilder::new(path)
        .hidden(false)
        .require_git(false) // respeta .gitignore aunque no sea un repo git
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{args, text_of, TmpRepo};
    use serde_json::json;

    /// Antes devolvía "No files found" en un directorio con 325 archivos.
    #[test]
    fn sin_patron_lista_todo_agrupado_por_directorio() {
        let repo = TmpRepo::new("find");
        repo.write("a.md", "x");
        repo.write("src/b.rs", "x");
        repo.write("src/c.rs", "x");

        let out = text_of(&execute(&args(json!({"path": repo.root_str()}))).unwrap());
        assert!(out.contains("3 archivo(s)"));
        assert!(out.contains("b.rs, c.rs"), "agrupa por directorio: {}", out);
        assert_eq!(out.matches("src").count(), 1, "el prefijo no se repite por archivo");
    }

    #[test]
    fn el_glob_filtra_por_nombre() {
        let repo = TmpRepo::new("find-glob");
        repo.write("a.md", "x");
        repo.write("src/b.rs", "x");
        let out = text_of(&execute(&args(json!({"path": repo.root_str(), "name": "*.md"}))).unwrap());
        assert!(out.contains("a.md") && !out.contains("b.rs"));
    }

    #[test]
    fn excluye_el_indice_git_y_el_archivo_de_openspec() {
        let repo = TmpRepo::new("find-scope");
        repo.write("vivo.md", "x");
        repo.write(".rtk-index/index.db", "x");
        repo.write(".git/config", "x");
        repo.write("openspec/changes/archive/viejo/tasks.md", "x");

        let out = text_of(&execute(&args(json!({"path": repo.root_str()}))).unwrap());
        assert!(out.contains("vivo.md"));
        assert!(!out.contains(".rtk-index") && !out.contains(".git") && !out.contains("archive"));
    }

    #[test]
    fn respeta_gitignore() {
        let repo = TmpRepo::new("find-ignore");
        repo.write(".gitignore", "ignorado/\n");
        repo.write("visible.rs", "x");
        repo.write("ignorado/oculto.rs", "x");
        let out = text_of(&execute(&args(json!({"path": repo.root_str()}))).unwrap());
        assert!(out.contains("visible.rs"));
        assert!(!out.contains("oculto.rs"));
    }

    /// Nunca truncar en silencio: hay que decir cuántos quedaron fuera.
    #[test]
    fn el_tope_se_anuncia() {
        let repo = TmpRepo::new("find-tope");
        for i in 0..(MAX_FILES + 10) {
            repo.write(&format!("f{}.rs", i), "x");
        }
        let out = text_of(&execute(&args(json!({"path": repo.root_str()}))).unwrap());
        assert!(out.contains(&format!("{} archivo(s)", MAX_FILES + 10)));
        assert!(out.contains("+10 archivos no listados"));
    }

    #[test]
    fn ruta_inexistente_es_error_no_lista_vacia() {
        assert!(execute(&args(json!({"path": "/ruta/que/no/existe"}))).is_err());
    }
}
