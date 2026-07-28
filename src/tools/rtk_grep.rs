use crate::tools::ToolResult;
use serde_json::Value;
use std::process::Command;

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let query = arguments.get("query").and_then(|v| v.as_str()).ok_or("Missing 'query' argument")?;
    let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or(".");

    let output = Command::new("rtk")
        .arg("grep")
        .arg(query)
        .arg(path)
        .output()
        .map_err(|e| format!("Failed to execute rtk grep: {}", e))?;

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if result.is_empty() {
        if !stderr.is_empty() {
            return Err(stderr);
        }
        return Ok(ToolResult::text_no_gain("No matches found.".to_string(), query));
    }

    let baseline = raw_grep_chars(query, path).unwrap_or(result.len());
    Ok(ToolResult::text(result, baseline, query))
}

/// Baseline: lo que habría devuelto un `grep -rn` sin comprimir. Solo se paga
/// este proceso extra si la telemetría está activa.
fn raw_grep_chars(query: &str, path: &str) -> Option<usize> {
    if !crate::telemetry::enabled() {
        return None;
    }
    let out = Command::new("grep")
        .args([
            "-rnIE",
            "--exclude-dir=.git",
            "--exclude-dir=node_modules",
            "--exclude-dir=target",
            "--exclude-dir=dist",
            "--exclude-dir=.venv",
            "-e",
        ])
        .arg(query)
        .arg(path)
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&out.stdout).trim().len();
    if raw == 0 {
        None
    } else {
        Some(raw)
    }
}
