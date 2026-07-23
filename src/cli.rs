//! Comando `init`: hace todo el trabajo pesado inicial una sola vez y deja el
//! MCP configurado en el workspace.

use crate::index::Workspace;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub fn run_init(root: &str) -> i32 {
    let root_abs = fs::canonicalize(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| root.to_string());
    eprintln!("rtk-index init → {}\n", root_abs);

    // 1) Símbolos + BM25 (tree-sitter + tantivy).
    eprintln!("[1/3] Indexando símbolos y texto (tree-sitter + BM25)…");
    let mut ws = match Workspace::open(&root_abs) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("      error: {}", e);
            return 1;
        }
    };
    let s = match ws.sync() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("      error: {}", e);
            return 1;
        }
    };
    eprintln!(
        "      {} archivos · {} símbolos indexados.",
        s.total_files,
        ws.symbol_count()
    );

    // 2) Vectores semánticos — completo (sin el tope por-llamada), con progreso.
    eprintln!("[2/3] Vectorizando (embeddings locales; solo la 1ª vez)…");
    loop {
        match ws.ensure_vectors() {
            Ok(vs) => {
                eprintln!(
                    "      {} vectorizados · {} pendientes…",
                    ws.vector_count(),
                    vs.remaining
                );
                if vs.remaining == 0 {
                    break;
                }
            }
            Err(e) => {
                eprintln!(
                    "      semántico no disponible ({}). Se continúa solo con BM25.",
                    e
                );
                break;
            }
        }
    }

    // 3) Configurar MCP.
    eprintln!("[3/3] Configurando MCP en .mcp.json…");
    if let Err(e) = write_mcp_config(&root_abs) {
        eprintln!("      error: {}", e);
        return 1;
    }

    eprintln!(
        "\n✓ Listo. Reinicia Claude Code en el proyecto y ya puedes usar:\n  \
         codebase_search · symbol_lookup · file_outline (índice caliente, sin esperas)."
    );
    0
}

fn write_mcp_config(root: &str) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("no se pudo resolver el binario: {}", e))?
        .to_string_lossy()
        .to_string();

    let cfg_path = Path::new(root).join(".mcp.json");
    let mut cfg: Value = if cfg_path.exists() {
        let txt = fs::read_to_string(&cfg_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&txt).map_err(|e| format!(".mcp.json inválido: {}", e))?
    } else {
        json!({ "mcpServers": {} })
    };

    if !cfg.get("mcpServers").map_or(false, |v| v.is_object()) {
        cfg["mcpServers"] = json!({});
    }
    cfg["mcpServers"]["rtk-index"] = json!({
        "command": exe,
        "args": ["serve", "--root", root]
    });

    let pretty = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(&cfg_path, pretty + "\n").map_err(|e| e.to_string())?;
    eprintln!("      registrado servidor 'rtk-index' → {}", cfg_path.display());
    Ok(())
}
