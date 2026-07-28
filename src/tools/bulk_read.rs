use crate::tools::ToolResult;
use serde_json::Value;
use std::fs;
use std::thread;

pub fn execute(params: &Value) -> Result<ToolResult, String> {
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let paths_val = arguments.get("paths");

    if let Some(paths_array) = paths_val.and_then(|v| v.as_array()) {
        if paths_array.is_empty() {
            return Err("No paths provided".to_string());
        }

        let mut handles = Vec::new();
        let mut count = 0usize;

        for path_val in paths_array {
            if let Some(path_str) = path_val.as_str() {
                let path = path_str.to_string();
                count += 1;
                let handle = thread::spawn(move || {
                    match fs::read_to_string(&path) {
                        Ok(content) => {
                            let mut numbered_content = Vec::new();
                            for (i, line) in content.lines().enumerate() {
                                // Mismo formato que cat -n: justificado a 6 espacios + tabulador
                                numbered_content.push(format!("{:6}\t{}", i + 1, line));
                            }
                            format!("--- FILE: {} ---\n{}", path, numbered_content.join("\n"))
                        }
                        Err(e) => {
                            format!("--- FILE: {} ---\n[ERROR] Failed to read file: {}", path, e)
                        }
                    }
                });
                handles.push(handle);
            }
        }

        let mut results = Vec::new();
        for handle in handles {
            if let Ok(res) = handle.join() {
                results.push(res);
            }
        }

        let combined_text = results.join("\n\n");
        // Devuelve el contenido completo: no hay ahorro de tokens sobre `Read`.
        Ok(ToolResult::text_no_gain(
            combined_text,
            format!("{} archivo(s)", count),
        ))
    } else {
        Err("Invalid arguments: paths must be an array of strings".to_string())
    }
}
