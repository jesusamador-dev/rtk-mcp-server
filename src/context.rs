//! Raíz de workspace por defecto para las herramientas de índice.
//!
//! `init` la fija en `.mcp.json` (`serve --root <ruta>`), así las herramientas
//! funcionan sin que el llamador tenga que pasar `path` cada vez.

use std::sync::OnceLock;

pub static DEFAULT_ROOT: OnceLock<String> = OnceLock::new();

pub fn default_root() -> String {
    DEFAULT_ROOT
        .get()
        .cloned()
        .unwrap_or_else(|| ".".to_string())
}
