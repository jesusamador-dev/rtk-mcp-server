use crate::tools::ToolResult;
use serde_json::Value;
use std::process::Command;

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let name_pattern = arguments.get("name").and_then(|v| v.as_str());

    let mut cmd = Command::new("rtk");
    cmd.arg("find").arg(path);

    if let Some(pattern) = name_pattern {
        cmd.arg("-name").arg(pattern);
    }

    let output = cmd.output().map_err(|e| format!("Failed to execute rtk find: {}", e))?;

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    let detail = match name_pattern {
        Some(p) => format!("{} -name {}", path, p),
        None => path.to_string(),
    };

    if result.is_empty() {
        if !stderr.is_empty() {
            return Err(stderr);
        }
        return Ok(ToolResult::text_no_gain("No files found.".to_string(), detail));
    }

    let baseline = raw_find_chars(path, name_pattern).unwrap_or(result.len());
    Ok(ToolResult::text(result, baseline, detail))
}

/// Baseline: la salida cruda de `find`, un path por línea sin agrupar.
fn raw_find_chars(path: &str, name_pattern: Option<&str>) -> Option<usize> {
    if !crate::telemetry::enabled() {
        return None;
    }
    let mut cmd = Command::new("find");
    cmd.arg(path);
    if let Some(p) = name_pattern {
        cmd.arg("-name").arg(p);
    }
    let out = cmd.output().ok()?;
    let raw = String::from_utf8_lossy(&out.stdout).trim().len();
    if raw == 0 {
        None
    } else {
        Some(raw)
    }
}
