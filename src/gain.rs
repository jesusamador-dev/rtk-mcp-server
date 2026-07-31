//! `rtk-index gain`: reporte de ahorro de tokens a partir del log de telemetría.

use crate::telemetry::{self, Event};
use serde_json::json;
use std::collections::HashMap;

pub fn run(args: &[String]) -> i32 {
    let mut history: Option<usize> = None;
    let mut worst: Option<usize> = None;
    let mut as_json = false;
    let mut since: Option<u64> = None;
    let mut project: Option<String> = None;
    let mut tool: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--history" => {
                // `--history` o `--history 30`
                match args.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) => {
                        history = Some(n);
                        i += 1;
                    }
                    None => history = Some(20),
                }
            }
            "--worst" => {
                match args.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) => {
                        worst = Some(n);
                        i += 1;
                    }
                    None => worst = Some(15),
                }
            }
            "--json" => as_json = true,
            "--since" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("--since requiere un valor (ej. 7d, 24h, 30m)");
                    return 2;
                };
                match parse_duration(v) {
                    Some(secs) => since = Some(secs),
                    None => {
                        eprintln!("--since inválido: '{}' (usa 30m, 24h, 7d, all)", v);
                        return 2;
                    }
                }
                i += 1;
            }
            "--project" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("--project requiere una ruta (o '.')");
                    return 2;
                };
                project = Some(
                    std::fs::canonicalize(v)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| v.clone()),
                );
                i += 1;
            }
            "--tool" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("--tool requiere un nombre de herramienta");
                    return 2;
                };
                tool = Some(v.clone());
                i += 1;
            }
            "--reset" => {
                return match telemetry::reset() {
                    Ok(()) => {
                        eprintln!("Telemetría borrada ({}).", telemetry::log_path().display());
                        0
                    }
                    Err(e) => {
                        eprintln!("No se pudo borrar la telemetría: {}", e);
                        1
                    }
                };
            }
            "--help" | "-h" => {
                print_help();
                return 0;
            }
            other => {
                eprintln!("Opción desconocida para gain: '{}'. Usa --help.", other);
                return 2;
            }
        }
        i += 1;
    }

    let events = telemetry::load(since, project.as_deref(), tool.as_deref());

    if as_json {
        println!("{}", serde_json::to_string_pretty(&summary_json(&events)).unwrap_or_default());
        return 0;
    }

    if events.is_empty() {
        println!("rtk-index gain — sin datos todavía.\n");
        println!(
            "  Log: {}\n  Las herramientas MCP registran su ahorro en cuanto Claude Code las usa.",
            telemetry::log_path().display()
        );
        if !telemetry::enabled() {
            println!("  ⚠ Telemetría desactivada por RTK_INDEX_TELEMETRY=0.");
        }
        return 0;
    }

    match (history, worst) {
        (Some(n), _) => print_history(&events, n),
        (_, Some(n)) => print_worst(&events, n),
        _ => print_summary(&events, since, project.as_deref()),
    }
    0
}

fn print_help() {
    eprintln!(
        "rtk-index gain — ahorro de tokens de las herramientas MCP\n\n\
         USO:\n  \
         rtk-index gain                    Resumen global (todo el histórico)\n  \
         rtk-index gain --history [N]      Últimas N llamadas (por defecto 20)\n  \
         rtk-index gain --worst [N]        Las N llamadas MÁS CARAS en tokens\n  \
         rtk-index gain --since 7d         Filtra por antigüedad (30m · 24h · 7d)\n  \
         rtk-index gain --project .        Solo este workspace\n  \
         rtk-index gain --tool codebase_search\n  \
         rtk-index gain --json             Salida en JSON\n  \
         rtk-index gain --reset            Borra el historial\n\n\
         El ahorro compara la respuesta real contra el baseline de cada herramienta:\n\
         leer los archivos completos, o la salida cruda de grep/find/diff. Si tu\n\
         alternativa real era un grep dirigido, el ahorro es menor —o negativo—.\n\
         Mira `--worst` para lo que de verdad cuesta. 1 token ≈ 4 caracteres."
    );
}

struct Agg {
    calls: u64,
    baseline: u64,
    actual: u64,
    ms: u64,
    errors: u64,
}

impl Agg {
    fn new() -> Self {
        Agg { calls: 0, baseline: 0, actual: 0, ms: 0, errors: 0 }
    }
    fn add(&mut self, e: &Event) {
        self.calls += 1;
        self.baseline += e.baseline_tokens;
        self.actual += e.actual_tokens;
        self.ms += e.ms;
        if !e.ok {
            self.errors += 1;
        }
    }
    fn saved(&self) -> i64 {
        self.baseline as i64 - self.actual as i64
    }
    fn pct(&self) -> f64 {
        if self.baseline == 0 {
            0.0
        } else {
            self.saved() as f64 * 100.0 / self.baseline as f64
        }
    }
}

fn print_summary(events: &[Event], since: Option<u64>, project: Option<&str>) {
    let mut total = Agg::new();
    let mut by_tool: HashMap<&str, Agg> = HashMap::new();
    let mut by_project: HashMap<&str, Agg> = HashMap::new();

    for e in events {
        total.add(e);
        by_tool.entry(&e.tool).or_insert_with(Agg::new).add(e);
        by_project.entry(&e.project).or_insert_with(Agg::new).add(e);
    }

    let periodo = match since {
        Some(s) => format!("últimos {}", human_duration(s)),
        None => "todo el histórico".to_string(),
    };
    let ambito = match project {
        Some(p) => short_project(p),
        None => "todos los proyectos".to_string(),
    };
    let antiguedad = events
        .first()
        .map(|e| human_ago(e.ts))
        .unwrap_or_else(|| "—".to_string());

    println!("\n  rtk-index gain — ahorro de tokens\n");
    println!(
        "  {} · {} · {} llamadas · desde hace {}",
        periodo,
        ambito,
        total.calls,
        antiguedad
    );
    println!();
    println!("  Baseline (archivos completos / salida cruda)   {:>10}", fmt(total.baseline));
    println!("  Real     (respuesta de rtk-index)              {:>10}", fmt(total.actual));
    println!("  {}", "─".repeat(58));
    println!(
        "  AHORRO                                         {:>10}   ({:.0} %)",
        fmt(total.saved().max(0) as u64),
        total.pct()
    );
    if total.errors > 0 {
        println!("  ({} llamada(s) con error incluidas)", total.errors);
    }

    println!("\n  Por herramienta");
    println!(
        "  {:<18} {:>6} {:>10} {:>10} {:>8} {:>8}",
        "HERRAMIENTA", "LLAM.", "BASELINE", "REAL", "AHORRO", "MEDIA"
    );
    let mut tools: Vec<_> = by_tool.into_iter().collect();
    tools.sort_by_key(|(_, a)| -a.saved());
    for (name, a) in tools {
        println!(
            "  {:<18} {:>6} {:>10} {:>10} {:>7.0}% {:>7}ms",
            name,
            a.calls,
            fmt(a.baseline),
            fmt(a.actual),
            a.pct(),
            a.ms / a.calls.max(1)
        );
    }

    if by_project.len() > 1 {
        println!("\n  Por proyecto");
        let mut projects: Vec<_> = by_project.into_iter().collect();
        projects.sort_by_key(|(_, a)| -a.saved());
        for (p, a) in projects {
            println!(
                "  {:<18} {:>6} {:>10} {:>10} {:>7.0}%",
                truncate_left(&short_project(p), 18),
                a.calls,
                fmt(a.baseline),
                fmt(a.actual),
                a.pct()
            );
        }
    }

    println!(
        "\n  El baseline es el rival de cada herramienta (archivo completo, salida cruda):\n  \
         si tu alternativa real era un grep dirigido, el ahorro es menor o negativo.\n  \
         `--worst` lista lo que más cuesta. 1 token ≈ 4 caracteres · log: {}",
        telemetry::log_path().display()
    );
    println!();
}

fn print_history(events: &[Event], n: usize) {
    let start = events.len().saturating_sub(n);
    let slice = &events[start..];
    println!("\n  rtk-index gain --history · últimas {} llamadas\n", slice.len());
    println!(
        "  {:<10} {:<18} {:>9} {:>8} {:>7}  {}",
        "HACE", "HERRAMIENTA", "AHORRO", "TOKENS", "MS", "DETALLE"
    );
    for e in slice {
        let pct = if e.baseline_tokens == 0 {
            0.0
        } else {
            e.saved() as f64 * 100.0 / e.baseline_tokens as f64
        };
        println!(
            "  {:<10} {:<18} {:>8.0}% {:>8} {:>7}  {}{}",
            human_ago(e.ts),
            e.tool,
            pct,
            fmt(e.saved().max(0) as u64),
            e.ms,
            if e.ok { "" } else { "[error] " },
            e.detail.clone().unwrap_or_default()
        );
    }
    println!();
}

/// Las llamadas más caras en tokens reales. El % de ahorro puede esconder una
/// respuesta enorme: esto la saca a la luz.
fn print_worst(events: &[Event], n: usize) {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.actual_tokens));
    sorted.truncate(n);
    println!("\n  rtk-index gain --worst · las {} llamadas más caras\n", sorted.len());
    println!(
        "  {:<9} {:<18} {:>8} {:>8} {:>7}  {}",
        "COSTO", "HERRAMIENTA", "AHORRO", "HACE", "MS", "DETALLE"
    );
    for e in sorted {
        let pct = if e.baseline_tokens == 0 {
            0.0
        } else {
            e.saved() as f64 * 100.0 / e.baseline_tokens as f64
        };
        println!(
            "  {:<9} {:<18} {:>7.0}% {:>8} {:>7}  {}",
            fmt(e.actual_tokens),
            e.tool,
            pct,
            human_ago(e.ts),
            e.ms,
            e.detail.clone().unwrap_or_default()
        );
    }
    println!();
}

fn summary_json(events: &[Event]) -> serde_json::Value {
    let mut total = Agg::new();
    let mut by_tool: HashMap<&str, Agg> = HashMap::new();
    for e in events {
        total.add(e);
        by_tool.entry(&e.tool).or_insert_with(Agg::new).add(e);
    }
    let tools: HashMap<&str, serde_json::Value> = by_tool
        .iter()
        .map(|(k, a)| {
            (
                *k,
                json!({
                    "calls": a.calls,
                    "baseline_tokens": a.baseline,
                    "actual_tokens": a.actual,
                    "saved_tokens": a.saved(),
                    "saved_pct": (a.pct() * 10.0).round() / 10.0,
                    "avg_ms": a.ms / a.calls.max(1),
                    "errors": a.errors,
                }),
            )
        })
        .collect();
    json!({
        "calls": total.calls,
        "baseline_tokens": total.baseline,
        "actual_tokens": total.actual,
        "saved_tokens": total.saved(),
        "saved_pct": (total.pct() * 10.0).round() / 10.0,
        "errors": total.errors,
        "by_tool": tools,
        "log": telemetry::log_path().to_string_lossy(),
    })
}

/// "7d" → 604800. "all" → None-equivalente (u64::MAX no filtra nada útil, así
/// que lo tratamos como "sin filtro" devolviendo un horizonte enorme).
fn parse_duration(s: &str) -> Option<u64> {
    if s == "all" || s == "todo" {
        return Some(u64::MAX / 2);
    }
    let (num, unit) = s.split_at(s.find(|c: char| c.is_alphabetic())?);
    let n: u64 = num.parse().ok()?;
    let mult = match unit {
        "m" | "min" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    Some(n * mult)
}

fn human_duration(secs: u64) -> String {
    if secs >= u64::MAX / 4 {
        "todo el histórico".to_string()
    } else if secs % 604_800 == 0 {
        format!("{} semana(s)", secs / 604_800)
    } else if secs % 86_400 == 0 {
        format!("{} día(s)", secs / 86_400)
    } else if secs % 3600 == 0 {
        format!("{} hora(s)", secs / 3600)
    } else {
        format!("{} min", secs / 60)
    }
}

/// Antigüedad relativa: evita depender de zonas horarias/calendario.
fn human_ago(ts: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let d = now.saturating_sub(ts);
    if d < 60 {
        format!("{}s", d)
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86_400)
    }
}

fn fmt(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn short_project(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

fn truncate_left(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().skip(s.chars().count() - max + 1).collect();
        format!("…{}", cut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duraciones_relativas() {
        assert_eq!(parse_duration("30m"), Some(1_800));
        assert_eq!(parse_duration("24h"), Some(86_400));
        assert_eq!(parse_duration("7d"), Some(604_800));
        assert_eq!(parse_duration("2w"), Some(1_209_600));
        assert_eq!(parse_duration("5x"), None);
        assert_eq!(parse_duration("d"), None);
        assert!(parse_duration("all").is_some());
    }

    #[test]
    fn formato_compacto_de_cifras() {
        assert_eq!(fmt(999), "999");
        assert_eq!(fmt(1_500), "1.5K");
        assert_eq!(fmt(2_400_000), "2.4M");
    }

    #[test]
    fn el_agregado_suma_y_calcula_el_porcentaje() {
        let ev = |b: u64, a: u64, ok: bool| Event {
            ts: 0, tool: "t".into(), project: "/p".into(), ok, ms: 10,
            baseline_tokens: b, actual_tokens: a, vs: "x".into(), detail: None,
        };
        let mut agg = Agg::new();
        agg.add(&ev(1000, 100, true));
        agg.add(&ev(1000, 900, false));
        assert_eq!(agg.calls, 2);
        assert_eq!(agg.saved(), 1000);
        assert_eq!(agg.pct().round(), 50.0);
        assert_eq!(agg.errors, 1, "los errores se cuentan y se muestran aparte");
    }

    #[test]
    fn un_agregado_sin_baseline_no_divide_por_cero() {
        let agg = Agg::new();
        assert_eq!(agg.pct(), 0.0);
    }
}
