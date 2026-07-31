//! Trazas de fase, opcionales y a stderr: `RTK_INDEX_TRACE=1`.
//!
//! Van a stderr, no a la respuesta MCP, así que no cuestan tokens. Sirven para
//! ver dónde se va el tiempo de una llamada (abrir el índice, sincronizar,
//! cargar el modelo de embeddings…) sin instrumentar a mano cada vez.

use std::sync::OnceLock;
use std::time::Instant;

pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        matches!(
            std::env::var("RTK_INDEX_TRACE").as_deref(),
            Ok("1") | Ok("on") | Ok("true") | Ok("yes")
        )
    })
}

pub struct Span {
    label: &'static str,
    t0: Instant,
}

/// Abre un tramo cronometrado; se imprime al soltarse. `None` (coste cero) si
/// las trazas están apagadas.
pub fn span(label: &'static str) -> Option<Span> {
    if enabled() {
        Some(Span { label, t0: Instant::now() })
    } else {
        None
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        eprintln!("[trace] {:<28} {:>7} ms", self.label, self.t0.elapsed().as_millis());
    }
}
