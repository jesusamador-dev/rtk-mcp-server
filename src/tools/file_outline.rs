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
            "leer el archivo completo",
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
    Ok(ToolResult::text(
        lines.join("\n"),
        baseline,
        "leer el archivo completo",
        path,
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

    #[test]
    fn devuelve_firmas_sin_cuerpos() {
        let repo = TmpRepo::new("outline");
        let cuerpo: String = (0..60).map(|i| format!("    let secreto_{} = {};\n", i, i)).collect();
        let fuente: String = (0..8)
            .map(|i| format!("pub fn funcion_{}() {{\n{}}}\n\n", i, cuerpo))
            .collect();
        repo.write("a.rs", &fuente);

        let r = execute(&args(json!({"path": repo.path("a.rs")}))).unwrap();
        let out = text_of(&r);
        assert!(out.contains("funcion_0") && out.contains("funcion_7"));
        assert!(!out.contains("secreto_"), "el cuerpo no viaja");
        assert_eq!(r.baseline_chars, fuente.len(), "el rival es el archivo entero");
        assert!(out.len() * 10 < r.baseline_chars, "y ahorra de sobra: {} vs {}", out.len(), r.baseline_chars);
    }

    /// En un archivo diminuto el outline no ahorra nada, y así debe reportarse:
    /// la métrica no puede inventar un ahorro que no existe.
    #[test]
    fn en_un_archivo_minusculo_no_hay_ahorro_que_presumir() {
        let repo = TmpRepo::new("outline-mini");
        repo.write("a.rs", "pub fn f() {}\n");
        let r = execute(&args(json!({"path": repo.path("a.rs")}))).unwrap();
        assert!(
            r.baseline_chars <= text_of(&r).len(),
            "leer el archivo entero era más barato: no se reporta ahorro"
        );
    }

    #[test]
    fn archivo_sin_simbolos_o_inexistente() {
        let repo = TmpRepo::new("outline-vacio");
        repo.write("datos.rs", "// solo un comentario\n");
        let out = text_of(&execute(&args(json!({"path": repo.path("datos.rs")}))).unwrap());
        assert!(out.contains("No symbols found"));
        assert!(execute(&args(json!({"path": repo.path("no-existe.rs")}))).is_err());
    }
}
