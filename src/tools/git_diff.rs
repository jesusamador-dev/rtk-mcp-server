use crate::tools::ToolResult;
use serde_json::Value;
use std::process::Command;

pub fn execute(_params: &Value) -> Result<ToolResult, String> {
    let rtk = Command::new("rtk").arg("git").arg("diff").output();
    let used_rtk = rtk.as_ref().map(|o| o.status.success()).unwrap_or(false);

    let output = if used_rtk {
        rtk
    } else {
        // Fallback a git nativo.
        Command::new("git").arg("diff").output()
    };

    match output {
        Ok(out) => {
            let diff = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if diff.is_empty() {
                return Ok(ToolResult::text_no_gain(
                    "No unstaged changes.".to_string(),
                    "git diff",
                ));
            }
            // Baseline: el `git diff` nativo. Si ya fue el fallback, no hay ahorro.
            let baseline = if used_rtk {
                raw_git_diff_chars().unwrap_or(diff.len())
            } else {
                diff.len()
            };
            Ok(ToolResult::text(diff, baseline, "git diff"))
        }
        Err(e) => Err(format!("Failed to execute git diff: {}", e)),
    }
}

fn raw_git_diff_chars() -> Option<usize> {
    if !crate::telemetry::enabled() {
        return None;
    }
    let out = Command::new("git").arg("diff").output().ok()?;
    let raw = String::from_utf8_lossy(&out.stdout).trim().len();
    if raw == 0 {
        None
    } else {
        Some(raw)
    }
}
