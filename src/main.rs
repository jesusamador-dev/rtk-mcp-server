mod cli;
mod context;
mod gain;
mod index;
mod telemetry;
mod tools;
mod trace;
mod update;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

/// Peticiones encoladas sin atender. El warm-up ocioso la consulta entre lotes
/// para cortar en cuanto haya trabajo real que hacer.
static PENDING: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize, Deserialize, Debug)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

fn send_response(id: Value, result: Value) {
    let resp = RpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    };
    if let Ok(json_str) = serde_json::to_string(&resp) {
        println!("{}", json_str);
    }
}

fn send_error(id: Value, code: i32, message: &str) {
    let resp = RpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(json!({
            "code": code,
            "message": message
        })),
    };
    if let Ok(json_str) = serde_json::to_string(&resp) {
        println!("{}", json_str);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("init") => {
            let root = args.get(2).map(|s| s.as_str()).unwrap_or(".");
            std::process::exit(cli::run_init(root));
        }
        Some("check") => {
            std::process::exit(cli::run_check());
        }
        Some("gain") => {
            std::process::exit(gain::run(&args[2..]));
        }
        Some("update") => {
            std::process::exit(update::run(&args[2..]));
        }
        Some("--version") | Some("-V") | Some("version") => {
            println!("rtk-index {}", update::VERSION);
        }
        Some("serve") => {
            // --root <ruta>: raíz por defecto para las herramientas de índice.
            if let Some(i) = args.iter().position(|a| a == "--root") {
                if let Some(r) = args.get(i + 1) {
                    let _ = context::DEFAULT_ROOT.set(r.clone());
                }
            }
            run_server();
        }
        Some("--help") | Some("-h") | Some("help") => {
            eprintln!(
                "rtk-index {} — servidor MCP con índice de código\n\n\
                 USO:\n  \
                 rtk-index init [ruta]        Indexa el workspace (1 vez) y configura .mcp.json\n  \
                 rtk-index check              Verifica el entorno (rtk, modelo, git)\n  \
                 rtk-index gain [opciones]    Muestra el ahorro de tokens medido (--help)\n  \
                 rtk-index update [--check]   Actualiza el binario a la última versión\n  \
                 rtk-index serve [--root R]   Corre el servidor MCP (lo lanza Claude Code)\n  \
                 rtk-index                    Igual que 'serve' (compatibilidad)",
                update::VERSION
            );
        }
        None => run_server(),
        Some(other) => {
            eprintln!(
                "Comando desconocido: '{}'. Usa: init | check | gain | update | serve | --help",
                other
            );
            std::process::exit(2);
        }
    }
}

fn run_server() {
    eprintln!("RTK MCP Server running on stdio");

    // Un hilo lee stdin y encola; el principal atiende y, cuando la cola está
    // vacía, aprovecha el rato ocioso para calentar el índice semántico. Así
    // ninguna búsqueda paga la vectorización del backlog: para cuando llega la
    // consulta, el trabajo ya está hecho.
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let mut line = String::new();
        while handle.read_line(&mut line).unwrap_or(0) > 0 {
            PENDING.fetch_add(1, Ordering::Relaxed);
            if tx.send(std::mem::take(&mut line)).is_err() {
                return;
            }
        }
    });

    loop {
        match rx.try_recv() {
            Ok(line) => {
                PENDING.fetch_sub(1, Ordering::Relaxed);
                dispatch(&line)
            }
            Err(mpsc::TryRecvError::Empty) => {
                // Nada que atender: un paso de warm-up. Si no queda backlog,
                // dormimos en el canal hasta la próxima petición.
                if !warm_step() {
                    match rx.recv() {
                        Ok(line) => {
                            PENDING.fetch_sub(1, Ordering::Relaxed);
                            dispatch(&line)
                        }
                        Err(_) => break,
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
        }
    }
}

fn dispatch(line: &str) {
    if line.trim().is_empty() {
        return;
    }
    match serde_json::from_str::<RpcRequest>(line) {
        Ok(req) => handle_request(req),
        Err(e) => eprintln!("Failed to parse request: {} - Line: {}", e, line.trim()),
    }
}

/// Un paso de calentamiento en tiempo ocioso. Devuelve `true` si quedó backlog
/// (hay más que hacer), `false` si el índice ya está completo o no disponible.
fn warm_step() -> bool {
    static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FAILED.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    // El sync solo de vez en cuando: repetirlo en cada paso del warm-up sería
    // recorrer el árbol una y otra vez sin que nada haya cambiado. Las
    // peticiones reales lo hacen siempre, así que la frescura no depende de esto.
    static STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let step = STEP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = context::default_root();
    let outcome = index::with_workspace(&root, |ws| {
        if step % 50 == 0 {
            ws.sync()?;
        }
        ws.ensure_vectors_idle(&|| PENDING.load(Ordering::Relaxed) > 0)
    });
    match outcome {
        Ok(stats) if stats.remaining > 0 => true,
        Ok(_) => {
            // Sin backlog: aprovechar el ocio para dejar el modelo cargado y
            // los vectores en RAM, una sola vez. Después ya no hay nada que
            // hacer y el bucle se duerme esperando peticiones.
            static PREHEATED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if PREHEATED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                return false;
            }
            if let Err(e) = index::with_workspace(&root, |ws| ws.preheat()) {
                eprintln!("[warm-up] precalentado no disponible: {}", e);
            }
            false
        }
        Err(e) => {
            // Sin modelo o sin índice: no insistir en cada vuelta del bucle.
            eprintln!("[warm-up] desactivado: {}", e);
            FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
            false
        }
    }
}

/// Pie de la respuesta. El dato que importa va primero: lo que ESTA respuesta
/// cuesta. El ahorro va después y nombra su rival — un "-95 %" sin decir contra
/// qué invita a creer que se ahorró algo que quizá nadie iba a gastar.
fn footer(ms: u64, baseline_chars: usize, actual_chars: usize, vs: &str) -> String {
    let cost = telemetry::tokens(actual_chars);
    if baseline_chars > actual_chars {
        let diff = baseline_chars - actual_chars;
        format!(
            "[{} ms · ~{} tokens · −{}% vs {}]",
            ms,
            cost,
            diff * 100 / baseline_chars,
            vs
        )
    } else {
        format!("[{} ms · ~{} tokens]", ms, cost)
    }
}

fn handle_request(req: RpcRequest) {
    let method = req.method.as_str();
    let is_notification = req.id.is_none();
    let id = req.id.unwrap_or(Value::Null);

    match method {
        "initialize" => {
            if !is_notification {
                send_response(id, json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "rtk-mcp-server",
                        "version": "1.0.0"
                    }
                }));
            }
        }
        "notifications/initialized" => {}
        "tools/list" => {
            if !is_notification {
                send_response(id, json!({
                    "tools": [
                        {
                            "name": "bulk_read_files",
                            "description": "Lee múltiples archivos completos en paralelo. NO ahorra tokens frente a Read: devuelve el contenido íntegro. Úsala solo cuando de verdad necesitas los archivos enteros.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Rutas de archivos" }
                                },
                                "required": ["paths"]
                            }
                        },
                        {
                            "name": "get_minified_diff",
                            "description": "Obtiene los cambios (git diff) sin indexar de manera ultra comprimida con RTK.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {}
                            }
                        },
                        {
                            "name": "rtk_grep",
                            "description": "Busca un patrón (regex) respetando .gitignore, con el mismo alcance que rtk_find y el índice: excluye .git, .rtk-index y el histórico de OpenSpec. Devuelve las líneas con su ancla path:línea, o —si hay demasiadas— el conteo por archivo. Para patrones exactos; para buscar por concepto, codebase_search.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "query": { "type": "string", "description": "Término de búsqueda o regex" },
                                    "path": { "type": "string", "description": "Archivo o directorio donde buscar (por defecto '.')" },
                                    "count": { "type": "boolean", "description": "Solo el conteo por archivo, como `grep -c`. Para '¿cuántos quedan?' es mucho más barato que listar las líneas." }
                                },
                                "required": ["query"]
                            }
                        },
                        {
                            "name": "rtk_find",
                            "description": "Lista archivos agrupados por directorio, respetando .gitignore (excluye .git, .rtk-index y el archivo histórico de OpenSpec). Tope de 400 archivos, y dice cuántos quedaron fuera.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string", "description": "Directorio de búsqueda" },
                                    "name": { "type": "string", "description": "Glob opcional sobre el nombre (ej. *.ts). Sin él, lista todos." }
                                }
                            }
                        },
                        {
                            "name": "file_outline",
                            "description": "Devuelve las firmas (funciones, clases, structs) de un archivo con tree-sitter, sin leer el cuerpo. Para orientarse gastando pocos tokens.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string", "description": "Archivo a analizar" }
                                },
                                "required": ["path"]
                            }
                        },
                        {
                            "name": "symbol_lookup",
                            "description": "Devuelve la definición exacta de un símbolo (función/clase/struct) por nombre, con su ancla path:líneas, sin leer archivos completos. Usa un índice persistente e incremental.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string", "description": "Nombre del símbolo" },
                                    "path": { "type": "string", "description": "Raíz del workspace (opcional, por defecto '.')" }
                                },
                                "required": ["name"]
                            }
                        },
                        {
                            "name": "codebase_search",
                            "description": "Búsqueda BM25 sobre el código: devuelve los k fragmentos más relevantes con su ancla path:líneas y el código, en 1 llamada. Ideal para localizar qué tocar sin leer archivos completos.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "query": { "type": "string", "description": "Términos a buscar en el código" },
                                    "path": { "type": "string", "description": "Raíz del workspace (opcional, por defecto '.')" },
                                    "k": { "type": "number", "description": "Número de resultados (por defecto 8, máx 25)" }
                                },
                                "required": ["query"]
                            }
                        }
                    ]
                }));
            }
        }
        "tools/call" => {
            if !is_notification {
                let params = req.params.unwrap_or(Value::Null);
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                
                let start_time = std::time::Instant::now();
                
                let result = match name {
                    "bulk_read_files" => tools::bulk_read::execute(&params),
                    "get_minified_diff" => tools::git_diff::execute(&params),
                    "rtk_grep" => tools::rtk_grep::execute(&params),
                    "rtk_find" => tools::rtk_find::execute(&params),
                    "file_outline" => tools::file_outline::execute(&params),
                    "symbol_lookup" => tools::symbol_lookup::execute(&params),
                    "codebase_search" => tools::codebase_search::execute(&params),
                    _ => Err(format!("Unknown tool: {}", name))
                };
                
                let elapsed_ms = start_time.elapsed().as_millis() as u64;

                match result {
                    Ok(tr) => {
                        let mut res = tr.value;
                        let mut actual_chars = 0usize;
                        if let Some(content) = res.get_mut("content").and_then(|c| c.as_array_mut()) {
                            if let Some(first) = content.get_mut(0) {
                                if let Some(text) = first.get("text").and_then(|t| t.as_str()) {
                                    let footer = footer(
                                        elapsed_ms,
                                        tr.baseline_chars,
                                        text.len(),
                                        tr.baseline_label,
                                    );
                                    let new_text = format!("{}\n\n{}", text, footer);
                                    actual_chars = new_text.len();
                                    first["text"] = Value::String(new_text);
                                }
                            }
                        }
                        telemetry::record(
                            name,
                            true,
                            elapsed_ms,
                            tr.baseline_chars,
                            actual_chars,
                            tr.baseline_label,
                            tr.detail,
                        );
                        send_response(id, res)
                    },
                    Err(e) => {
                        // Una llamada fallida no ahorra nada: baseline = coste real.
                        let msg = format!("{} (Execution time: {} ms)", e, elapsed_ms);
                        telemetry::record(
                            name,
                            false,
                            elapsed_ms,
                            msg.len(),
                            msg.len(),
                            "llamada fallida",
                            Some(e),
                        );
                        send_error(id, -32603, &msg)
                    }
                }
            }
        }
        _ => {
            if !is_notification {
                send_error(id, -32601, &format!("Method not found: {}", method));
            }
        }
    }
}
