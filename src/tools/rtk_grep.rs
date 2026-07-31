//! rtk_grep(query, path?): búsqueda comprimida delegada en `rtk grep`.
//!
//! Tres detalles que hacían fallar la llamada en silencio (salida vacía con
//! código 0, indistinguible de "no hay coincidencias") y que aquí se corrigen:
//! `-r` cuando el destino es un directorio, `--` para que un patrón que empieza
//! por `-` no se lea como flag, y propagar el error de `rtk` en vez de tragarlo.

use crate::tools::ToolResult;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'query' argument")?;
    let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or(".");

    if !Path::new(path).exists() {
        return Err(format!("La ruta no existe: {}", path));
    }
    let recursive = Path::new(path).is_dir();

    let mut cmd = Command::new("rtk");
    cmd.arg("grep");
    if recursive {
        cmd.arg("-r");
    }
    // `--` cierra las opciones: sin esto, un patrón como "- [ ]" se interpreta
    // como flag y grep aborta.
    cmd.arg("--").arg(query).arg(path);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute rtk grep: {}", e))?;

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    // grep: 0 = coincidencias, 1 = ninguna, >1 = error real.
    let code = output.status.code().unwrap_or(-1);
    if result.is_empty() {
        if code > 1 || (code != 0 && code != 1 && !stderr.is_empty()) {
            return Err(format!(
                "rtk grep falló (código {}): {}",
                code,
                if stderr.is_empty() { "sin detalle" } else { &stderr }
            ));
        }
        return Ok(ToolResult::text_no_gain(
            format!("Sin coincidencias para '{}' en {}.", query, path),
            query,
        ));
    }

    let baseline = raw_grep_chars(query, path, recursive).unwrap_or(result.len());
    Ok(ToolResult::text(result, baseline, "grep -rn crudo", query))
}

/// Baseline: lo que habría devuelto un `grep -rn` sin comprimir. Solo se paga
/// este proceso extra si la telemetría está activa.
fn raw_grep_chars(query: &str, path: &str, recursive: bool) -> Option<usize> {
    if !crate::telemetry::enabled() {
        return None;
    }
    let mut cmd = Command::new("grep");
    cmd.args(["-nIE"]);
    if recursive {
        cmd.args([
            "-r",
            "--exclude-dir=.git",
            "--exclude-dir=node_modules",
            "--exclude-dir=target",
            "--exclude-dir=dist",
            "--exclude-dir=.venv",
        ]);
    }
    let out = cmd.args(["-e", query, path]).output().ok()?;
    let raw = String::from_utf8_lossy(&out.stdout).trim().len();
    if raw == 0 {
        None
    } else {
        Some(raw)
    }
}
