//! Utilidades para las pruebas: un repo temporal desechable.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

/// Directorio temporal con archivos, que se borra solo al terminar la prueba.
pub struct TmpRepo {
    pub root: PathBuf,
}

impl TmpRepo {
    pub fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rtk-test-{}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
            tag
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("crear repo temporal");
        TmpRepo { root }
    }

    /// Escribe un archivo (creando los directorios que hagan falta).
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.root.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).expect("crear directorio");
        }
        fs::write(&p, content).expect("escribir archivo");
        p
    }

    /// Ruta absoluta, como `String`, para pasarla en los argumentos JSON.
    pub fn path(&self, rel: &str) -> String {
        self.root.join(rel).to_string_lossy().to_string()
    }

    pub fn root_str(&self) -> String {
        self.root.to_string_lossy().to_string()
    }
}

impl Drop for TmpRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Envuelve argumentos en la forma que reciben las herramientas MCP.
pub fn args(v: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "arguments": v })
}

/// Texto de la respuesta de una herramienta.
pub fn text_of(r: &crate::tools::ToolResult) -> String {
    r.value["content"][0]["text"]
        .as_str()
        .expect("respuesta de texto")
        .to_string()
}
