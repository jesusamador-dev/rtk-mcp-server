use serde_json::{json, Value};
use std::process::Command;

pub fn execute(_params: &Value) -> Result<Value, String> {
    let mut output = Command::new("rtk")
        .arg("git")
        .arg("diff")
        .output();
        
    if output.is_err() || !output.as_ref().unwrap().status.success() {
        // Fallback a git nativo
        output = Command::new("git").arg("diff").output();
    }
    
    match output {
        Ok(out) => {
            let diff = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if diff.is_empty() {
                Ok(json!({"content": [{"type": "text", "text": "No unstaged changes."}]}))
            } else {
                Ok(json!({"content": [{"type": "text", "text": diff}]}))
            }
        },
        Err(e) => Err(format!("Failed to execute git diff: {}", e))
    }
}
