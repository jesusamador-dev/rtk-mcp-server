use serde_json::{json, Value};
use std::process::Command;

pub fn execute(params: &Value) -> Result<Value, String> {
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
    
    if result.is_empty() {
        if !stderr.is_empty() {
            return Err(stderr);
        }
        return Ok(json!({"content": [{"type": "text", "text": "No files found."}]}));
    }
    
    Ok(json!({"content": [{"type": "text", "text": result}]}))
}
