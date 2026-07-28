//! `rtk-index update`: actualiza el binario en su sitio, sin que tengas que
//! recordar cómo lo instalaste.
//!
//! Compara la versión local con la del `Cargo.toml` de `main` en GitHub y,
//! si hay una nueva, reinstala con el mismo método con el que ya está
//! instalado: `cargo install --force` si el binario vive en el bin de cargo,
//! o el `install.sh` del repo en cualquier otro caso.

use std::path::{Path, PathBuf};
use std::process::Command;

const REPO: &str = "jesusamador-dev/rtk-mcp-server";
const BIN: &str = "rtk-index";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(args: &[String]) -> i32 {
    let mut check_only = false;
    let mut force = false;
    let mut method: Option<Method> = None;

    for a in args {
        match a.as_str() {
            "--check" => check_only = true,
            "--force" | "-f" => force = true,
            "--cargo" => method = Some(Method::Cargo),
            "--script" => method = Some(Method::Script),
            "--help" | "-h" => {
                print_help();
                return 0;
            }
            other => {
                eprintln!("Opción desconocida para update: '{}'. Usa --help.", other);
                return 2;
            }
        }
    }

    eprintln!("rtk-index update — versión instalada: {}", VERSION);

    let remote = remote_version();
    match &remote {
        Some(v) => eprintln!("  última publicada en {}: {}", REPO, v),
        None => eprintln!("  ⚠ no se pudo consultar la última versión (¿sin red?)"),
    }

    let outdated = match &remote {
        Some(v) => is_newer(v, VERSION),
        // Sin información remota no adivinamos: solo se actúa con --force.
        None => false,
    };

    if check_only {
        return if outdated {
            eprintln!("\n→ Hay una actualización disponible. Ejecuta: rtk-index update");
            1
        } else {
            eprintln!("\n✓ Estás en la última versión.");
            0
        };
    }

    if !outdated && !force {
        if remote.is_some() {
            eprintln!("\n✓ Ya estás en la última versión. Reinstala igualmente con --force.");
            0
        } else {
            eprintln!("\nNo hay nada que comparar. Reinstala igualmente con --force.");
            1
        }
    } else {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(BIN));
        let method = method.unwrap_or_else(|| detect_method(&exe));
        eprintln!(
            "\n→ Actualizando {} vía {}…",
            exe.display(),
            method.label()
        );
        match method.install() {
            Ok(()) => {
                let now = installed_version(&exe).unwrap_or_else(|| "?".to_string());
                eprintln!("\n✓ Listo — rtk-index {} → {}", VERSION, now);
                eprintln!(
                    "  Reinicia Claude Code para que recargue el servidor MCP.\n  \
                     El índice de tus proyectos se conserva; `rtk-index check` verifica el entorno."
                );
                0
            }
            Err(e) => {
                eprintln!("\n✗ No se pudo actualizar: {}", e);
                eprintln!(
                    "  Alternativa manual:\n    \
                     cargo install --git https://github.com/{} --bin {} --force",
                    REPO, BIN
                );
                1
            }
        }
    }
}

fn print_help() {
    eprintln!(
        "rtk-index update — actualiza el binario a la última versión\n\n\
         USO:\n  \
         rtk-index update            Actualiza si hay una versión nueva\n  \
         rtk-index update --check    Solo comprueba (sale con 1 si hay una nueva)\n  \
         rtk-index update --force    Reinstala aunque ya estés al día\n  \
         rtk-index update --cargo    Fuerza `cargo install --git … --force`\n  \
         rtk-index update --script   Fuerza el instalador install.sh del repo\n\n\
         Por defecto detecta el método por la ruta del binario en uso. El índice\n\
         y la telemetría de tus proyectos se conservan."
    );
}

#[derive(Clone, Copy)]
enum Method {
    Cargo,
    Script,
}

impl Method {
    fn label(self) -> &'static str {
        match self {
            Method::Cargo => "cargo install --force",
            Method::Script => "install.sh",
        }
    }

    fn install(self) -> Result<(), String> {
        match self {
            Method::Cargo => run_stream(
                Command::new("cargo").args([
                    "install",
                    "--git",
                    &format!("https://github.com/{}", REPO),
                    "--bin",
                    BIN,
                    "--force",
                ]),
            ),
            Method::Script => {
                // Instalación donde ya vive el binario, no en el default del script.
                let dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(Path::to_path_buf));
                let url = format!(
                    "https://raw.githubusercontent.com/{}/main/install.sh",
                    REPO
                );
                let mut cmd = Command::new("sh");
                cmd.arg("-c").arg(format!("curl -fsSL {} | sh", url));
                if let Some(d) = dir {
                    cmd.env("RTK_INSTALL_DIR", d);
                }
                run_stream(&mut cmd)
            }
        }
    }
}

fn run_stream(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("no se pudo ejecutar ({}). ¿Está instalado?", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("el instalador terminó con estado {}", status))
    }
}

/// Si el binario vive en el `bin` de cargo, se actualiza con cargo; si no, con
/// el script (que a su vez cae a cargo si no hay binario precompilado).
fn detect_method(exe: &Path) -> Method {
    let cargo_bin = std::env::var("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cargo")
        })
        .join("bin");
    if exe.starts_with(&cargo_bin) && which("cargo") {
        Method::Cargo
    } else if which("curl") {
        Method::Script
    } else if which("cargo") {
        Method::Cargo
    } else {
        Method::Script
    }
}

fn which(bin: &str) -> bool {
    std::env::var("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Versión publicada: la del `Cargo.toml` de `main` en GitHub.
fn remote_version() -> Option<String> {
    let url = format!(
        "https://raw.githubusercontent.com/{}/main/Cargo.toml",
        REPO
    );
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "10", &url])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let txt = String::from_utf8_lossy(&out.stdout);
    parse_version(&txt)
}

/// Primera clave `version = "x.y.z"` del manifiesto (la de `[package]`).
fn parse_version(manifest: &str) -> Option<String> {
    manifest
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("version") && l.contains('='))
        .and_then(|l| l.split('"').nth(1))
        .map(str::to_string)
}

/// ¿`a` es más nueva que `b`? Compara semver numérico simple (x.y.z).
fn is_newer(a: &str, b: &str) -> bool {
    parts(a) > parts(b)
}

fn parts(v: &str) -> (u64, u64, u64) {
    let mut it = v
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// Versión del binario ya instalado (tras actualizar, el proceso en curso
/// sigue siendo el viejo).
fn installed_version(exe: &Path) -> Option<String> {
    let out = Command::new(exe).arg("--version").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.split_whitespace().last().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_del_manifiesto() {
        let m = "[package]\nname = \"rtk-mcp-server\"\nversion = \"1.2.3\"\n";
        assert_eq!(parse_version(m).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn comparacion_semver() {
        assert!(is_newer("1.2.0", "1.1.9"));
        assert!(is_newer("1.1.10", "1.1.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
    }
}
