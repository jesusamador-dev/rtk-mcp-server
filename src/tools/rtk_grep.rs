use serde_json::{json, Value};
use std::process::Command;

pub fn execute(params: &Value) -> Result<Value, String> {
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
        return Ok(json!({"content": [{"type": "text", "text": "No matches found."}]}));
    }
    
    Ok(json!({"content": [{"type": "text", "text": result}]}))
}
