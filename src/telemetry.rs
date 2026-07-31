//! Telemetría de ahorro de tokens.
//!
//! Cada llamada a una herramienta MCP registra dos magnitudes:
//!
//! - **baseline**: lo que habría costado obtener la misma información *sin*
//!   rtk-index (leer los archivos completos, la salida cruda de grep/find/diff…).
//! - **real**: el texto que efectivamente se devuelve al modelo.
//!
//! La diferencia es el ahorro. Se escribe una línea JSON por llamada en un log
//! append-only (seguro con varios servidores MCP a la vez) y `rtk-index gain`
//! lo agrega. Nunca falla hacia el llamador: si el log no se puede escribir, se
//! ignora en silencio — la telemetría jamás debe romper una herramienta.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Estimación conservadora de tokens. Los tokenizadores BPE promedian ~3.5-4
/// caracteres por token sobre código; usamos 4 para no inflar el ahorro.
const CHARS_PER_TOKEN: f64 = 4.0;

/// A partir de este tamaño el log se rota (se conserva una generación).
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Event {
    /// Segundos desde epoch.
    pub ts: u64,
    pub tool: String,
    /// Raíz del workspace (ruta absoluta) desde la que se sirvió la llamada.
    pub project: String,
    pub ok: bool,
    pub ms: u64,
    pub baseline_tokens: u64,
    pub actual_tokens: u64,
    /// Contra qué se comparó el baseline ("leer el archivo completo"…).
    #[serde(default)]
    pub vs: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Event {
    pub fn saved(&self) -> i64 {
        self.baseline_tokens as i64 - self.actual_tokens as i64
    }
}

/// Tokens estimados para un texto.
pub fn tokens(chars: usize) -> u64 {
    (chars as f64 / CHARS_PER_TOKEN).ceil() as u64
}

pub fn enabled() -> bool {
    !matches!(
        std::env::var("RTK_INDEX_TELEMETRY").as_deref(),
        Ok("0") | Ok("off") | Ok("false") | Ok("no")
    )
}

/// Directorio de datos: `$RTK_INDEX_DATA_DIR`, `$XDG_DATA_HOME/rtk-index` o
/// `~/.local/share/rtk-index`.
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("RTK_INDEX_DATA_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("XDG_DATA_HOME") {
        if !p.is_empty() {
            return PathBuf::from(p).join("rtk-index");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/rtk-index")
}

pub fn log_path() -> PathBuf {
    data_dir().join("telemetry.jsonl")
}

/// Raíz del workspace actual, resuelta una sola vez por proceso.
fn project() -> &'static str {
    static PROJECT: OnceLock<String> = OnceLock::new();
    PROJECT.get_or_init(|| {
        let root = crate::context::default_root();
        fs::canonicalize(&root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(root)
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Registra una llamada. Silencioso ante cualquier error de E/S.
#[allow(clippy::too_many_arguments)]
pub fn record(
    tool: &str,
    ok: bool,
    ms: u64,
    baseline_chars: usize,
    actual_chars: usize,
    vs: &str,
    detail: Option<String>,
) {
    if !enabled() {
        return;
    }
    let ev = Event {
        ts: now(),
        tool: tool.to_string(),
        project: project().to_string(),
        ok,
        ms,
        baseline_tokens: tokens(baseline_chars),
        actual_tokens: tokens(actual_chars),
        vs: vs.to_string(),
        detail: detail.map(|d| truncate(&d, 80)),
    };
    let _ = append(&ev);
}

fn append(ev: &Event) -> std::io::Result<()> {
    let path = log_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
        let _ = fs::rename(&path, path.with_extension("jsonl.1"));
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    let line = serde_json::to_string(ev)?;
    // Una sola escritura: con O_APPEND las líneas cortas no se entrelazan entre
    // procesos, así que varios servidores MCP pueden compartir el mismo log.
    f.write_all(format!("{}\n", line).as_bytes())
}

/// Lee los eventos del log (y de la generación rotada, si existe), filtrados.
pub fn load(since_secs: Option<u64>, project_filter: Option<&str>, tool_filter: Option<&str>) -> Vec<Event> {
    let cutoff = since_secs.map(|s| now().saturating_sub(s));
    let mut out = Vec::new();
    let main = log_path();
    let rotated = main.with_extension("jsonl.1");
    for p in [rotated, main] {
        let Ok(txt) = fs::read_to_string(&p) else { continue };
        for line in txt.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(ev) = serde_json::from_str::<Event>(line) else { continue };
            if cutoff.map_or(false, |c| ev.ts < c) {
                continue;
            }
            if project_filter.map_or(false, |f| ev.project != f) {
                continue;
            }
            if tool_filter.map_or(false, |f| ev.tool != f) {
                continue;
            }
            out.push(ev);
        }
    }
    out
}

/// Borra el historial de telemetría.
pub fn reset() -> std::io::Result<()> {
    let main = log_path();
    let rotated = main.with_extension("jsonl.1");
    for p in [main, rotated] {
        if p.exists() {
            fs::remove_file(p)?;
        }
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}…", cut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_tokens_se_estiman_al_alza() {
        assert_eq!(tokens(0), 0);
        assert_eq!(tokens(1), 1, "nunca redondea a cero un texto no vacío");
        assert_eq!(tokens(4), 1);
        assert_eq!(tokens(5), 2);
        assert_eq!(tokens(4000), 1000);
    }

    #[test]
    fn el_ahorro_puede_ser_negativo_y_se_conserva_el_signo() {
        let caro = Event {
            ts: 0, tool: "rtk_grep".into(), project: "/p".into(), ok: true, ms: 1,
            baseline_tokens: 100, actual_tokens: 150, vs: "grep -rn".into(), detail: None,
        };
        assert_eq!(caro.saved(), -50, "una respuesta más cara no se disfraza de ahorro");
    }

    #[test]
    fn el_evento_sobrevive_al_viaje_por_jsonl() {
        let ev = Event {
            ts: 1_700_000_000, tool: "codebase_search".into(), project: "/repo".into(),
            ok: true, ms: 42, baseline_tokens: 900, actual_tokens: 100,
            vs: "leer el archivo completo".into(), detail: Some("consulta".into()),
        };
        let linea = serde_json::to_string(&ev).unwrap();
        assert!(!linea.contains('\n'), "una sola línea por evento");
        let vuelta: Event = serde_json::from_str(&linea).unwrap();
        assert_eq!(vuelta.saved(), 800);
        assert_eq!(vuelta.vs, "leer el archivo completo");
    }

    /// Los logs escritos antes de que existiera el campo `vs` deben seguir leyéndose.
    #[test]
    fn tolera_eventos_de_versiones_anteriores() {
        let viejo = r#"{"ts":1,"tool":"rtk_find","project":"/p","ok":true,"ms":3,"baseline_tokens":10,"actual_tokens":4}"#;
        let ev: Event = serde_json::from_str(viejo).unwrap();
        assert_eq!(ev.saved(), 6);
        assert_eq!(ev.vs, "");
    }

    #[test]
    fn el_detalle_se_recorta_y_no_rompe_la_linea() {
        let largo = format!("línea uno\nlínea dos {}", "x".repeat(200));
        let t = truncate(&largo, 80);
        assert!(!t.contains('\n'));
        assert!(t.chars().count() <= 81, "80 caracteres más la elipsis");
    }
}
