//! Índice de código: descubre archivos, los hashea y mantiene sincronizados de
//! forma incremental dos almacenes — símbolos (SQLite) y texto completo (tantivy).

pub mod chunker;
pub mod db;
pub mod embed;
pub mod search;

use ignore::WalkBuilder;
use rusqlite::Connection;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use db::SymbolRow;
use embed::Embedder;
use search::SearchIndex;

// Solo nombre + firma + primeras líneas: ahí está la señal semántica. Embeber el
// cuerpo completo (2000 chars) hunde el throughput a ~8 símb/s; con ~256 chars
// sube a ~100 símb/s (el costo de inferencia crece fuerte con la longitud).
const EMBED_TEXT_MAX_CHARS: usize = 256;

/// Tope de símbolos por pasada de `init` (sin límite de tiempo).
const MAX_EMBED_PER_CALL: usize = 800;

/// Tope dentro de una BÚSQUEDA. Muy bajo a propósito: cubre lo que acabas de
/// editar, no el backlog — de ese se encarga el warm-up ocioso.
const SEARCH_EMBED_MAX: usize = 32;

/// Tope por paso de warm-up ocioso.
const IDLE_EMBED_MAX: usize = 64;

/// Presupuesto de TIEMPO para calentar el índice dentro de una búsqueda. El
/// usuario espera su resultado, no el warm-up: sobre un repo con decenas de
/// miles de símbolos pendientes, el tope por conteo hacía que cada búsqueda
/// pagara ~8 s de vectorización ajena. Con deadline, la búsqueda responde y el
/// backlog se calienta poco a poco (o de golpe con `rtk-index init`).
const WARMUP_BUDGET_MS: u64 = 300;

/// Presupuesto por paso de calentamiento ocioso (entre peticiones MCP). Corto
/// a propósito: si llega una petición a medias, solo espera lo que reste del
/// lote en curso.
const IDLE_BUDGET_MS: u64 = 400;

/// Tamaño de lote de embebido: acota la granularidad con la que se comprueba
/// el deadline sin penalizar el throughput del modelo.
const EMBED_BATCH: usize = 64;

/// Resultado de calentar el índice semántico en una llamada.
pub struct VectorStats {
    #[allow(dead_code)]
    pub embedded_now: usize, // símbolos embebidos en esta llamada
    pub remaining: usize,    // símbolos aún sin vectorizar (backlog)
}

pub struct SyncStats {
    pub reindexed: usize,
    pub total_files: usize,
    pub removed: usize,
}

/// Archivo pendiente de vectorizar, con sus símbolos ya leídos de SQLite.
struct Pending {
    path: String,
    hash: String,
    symbols: Vec<SymbolRow>,
}

/// Candidato de búsqueda (sin score): identidad = (path, start_line).
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
}

impl Candidate {
    pub fn key(&self) -> String {
        format!("{}:{}", self.path, self.start_line)
    }
}

/// Un workspace indexado: SQLite (símbolos + vectores) + tantivy (BM25),
/// bajo `<root>/.rtk-index/`.
pub struct Workspace {
    root: String,
    conn: Connection,
    search: SearchIndex,
    embedder: Option<Embedder>,
    /// Vectores en memoria. Deserializarlos desde SQLite en cada búsqueda son
    /// ~30 MB y cientos de ms sobre un repo grande; aquí se cargan una vez y se
    /// actualizan en sitio cuando se re-vectoriza un archivo.
    vec_cache: Option<Vec<(db::VecRow, Vec<f32>)>>,
}

thread_local! {
    /// Workspaces abiertos, por raíz. El proceso del servidor MCP es de larga
    /// vida y atiende en serie: reabrir el índice en cada llamada obligaba a
    /// recargar el modelo ONNX (~1,4 s) una y otra vez.
    static WORKSPACES: std::cell::RefCell<std::collections::HashMap<String, Workspace>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Ejecuta `f` sobre el workspace de `root`, abriéndolo solo la primera vez.
/// El modelo de embeddings y los índices quedan calientes entre llamadas.
pub fn with_workspace<T>(
    root: &str,
    f: impl FnOnce(&mut Workspace) -> Result<T, String>,
) -> Result<T, String> {
    WORKSPACES.with(|cache| {
        let mut map = cache.borrow_mut();
        if !map.contains_key(root) {
            map.insert(root.to_string(), Workspace::open(root)?);
        }
        f(map.get_mut(root).expect("recién insertado"))
    })
}

impl Workspace {
    pub fn open(root: &str) -> Result<Workspace, String> {
        let _t = crate::trace::span("Workspace::open");
        let dir = Path::new(root).join(".rtk-index");
        fs::create_dir_all(&dir).map_err(|e| format!("create index dir: {}", e))?;
        let conn = db::open(&dir.join("index.db"))?;
        let search = SearchIndex::open(&dir.join("tantivy"))?;
        Ok(Workspace {
            root: root.to_string(),
            conn,
            search,
            embedder: None,
            vec_cache: None,
        })
    }

    /// Re-indexa incrementalmente. Si el índice tantivy está vacío pero SQLite ya
    /// tiene datos (p. ej. índice creado por una versión previa), fuerza un
    /// reindexado completo para poblar ambos almacenes de forma consistente.
    pub fn sync(&mut self) -> Result<SyncStats, String> {
        let _t = crate::trace::span("sync (walk+hash)");
        let force_all = self.search.num_docs() == 0;
        let writer = self.search.writer()?;
        // Todo el estado del índice en una consulta: preguntar archivo por
        // archivo costaba ~30 µs × miles de archivos, el grueso del `sync`.
        let stored_all = if force_all {
            std::collections::HashMap::new()
        } else {
            db::all_file_meta(&self.conn)?
        };

        let mut seen: HashSet<String> = HashSet::new();
        let mut reindexed = 0;
        let mut total = 0;
        let mut dirty = false;

        // Descubrimiento en paralelo: recorrer y hacer `stat` a miles de
        // archivos en un hilo era el grueso del `sync`. Solo se recolecta aquí;
        // SQLite y tantivy se tocan después, en serie.
        let candidates = walk_files(&self.root);

        for (path_buf, mtime) in candidates {
            let path = path_buf.as_path();
            let path_str = path.to_string_lossy().to_string();
            seen.insert(path_str.clone());
            total += 1;

            let stored = stored_all.get(&path_str);

            // Atajo por mtime: un `stat` en vez de leer y hashear el archivo.
            // Sobre un repo grande esto es la diferencia entre segundos y
            // milisegundos por llamada. Si el mtime cambió sí leemos, y el
            // hash decide (un `touch` o un checkout no re-indexan de balde).
            if let Some((_, stored_mtime)) = stored {
                if *stored_mtime == mtime {
                    continue;
                }
            }

            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let hash = blake3::hash(content.as_bytes()).to_hex().to_string();

            if let Some((stored_hash, _)) = stored {
                if *stored_hash == hash {
                    // Mismo contenido con otro mtime: refresca solo el mtime.
                    db::touch_mtime(&self.conn, &path_str, mtime);
                    continue;
                }
            }

            let symbols = chunker::parse(path, &content);

            // SQLite (símbolos)
            db::replace_file(&mut self.conn, &path_str, &hash, mtime, &symbols)?;

            // tantivy (texto completo): borrar docs previos de este archivo y re-insertar.
            self.search.delete_file(&writer, &path_str);
            let lines: Vec<&str> = content.lines().collect();
            for s in &symbols {
                let body = slice_lines(&lines, s.start_line, s.end_line);
                self.search.add_chunk(
                    &writer,
                    &path_str,
                    &s.name,
                    &s.kind,
                    s.start_line,
                    s.end_line,
                    &body,
                )?;
            }

            reindexed += 1;
            dirty = true;
        }

        // Purga de archivos borrados en ambos almacenes.
        let mut removed = 0;
        for stored_path in db::all_paths(&self.conn)? {
            if !seen.contains(&stored_path) {
                db::remove_file(&self.conn, &stored_path)?;
                self.search.delete_file(&writer, &stored_path);
                removed += 1;
                dirty = true;
            }
        }

        if dirty {
            self.search.commit_and_reload(writer)?;
        }

        Ok(SyncStats {
            reindexed,
            total_files: total,
            removed,
        })
    }

    pub fn lookup_symbol(&self, name: &str) -> Result<Vec<SymbolRow>, String> {
        db::lookup(&self.conn, name)
    }

    /// BM25 → candidatos en orden de relevancia.
    pub fn search_bm25(&self, query: &str, n: usize) -> Result<Vec<Candidate>, String> {
        let _t = crate::trace::span("search_bm25");
        Ok(self
            .search
            .query(query, n)?
            .into_iter()
            .map(|h| Candidate {
                path: h.path,
                name: h.name,
                kind: h.kind,
                start_line: h.start_line,
                end_line: h.end_line,
            })
            .collect())
    }

    fn ensure_embedder(&mut self) -> Result<&Embedder, String> {
        if self.embedder.is_none() {
            let _t = crate::trace::span("carga modelo ONNX");
            self.embedder = Some(Embedder::new()?);
        }
        Ok(self.embedder.as_ref().unwrap())
    }

    /// Calienta el índice semántico durante una BÚSQUEDA. Presupuesto mínimo:
    /// el grueso del trabajo lo hace `ensure_vectors_idle` entre peticiones, así
    /// que aquí solo se cubre lo recién editado.
    pub fn ensure_vectors(&mut self) -> Result<VectorStats, String> {
        self.ensure_vectors_budget(SEARCH_EMBED_MAX, Some(Duration::from_millis(WARMUP_BUDGET_MS)))
    }

    /// Calienta en tiempo ocioso, entre peticiones MCP. El lote es corto para
    /// que una petición que llegue a media vectorización no espere apenas.
    pub fn ensure_vectors_idle(&mut self) -> Result<VectorStats, String> {
        self.ensure_vectors_budget(IDLE_EMBED_MAX, Some(Duration::from_millis(IDLE_BUDGET_MS)))
    }

    /// Calienta sin límite de tiempo: para `init`, que sí viene a terminar el
    /// trabajo y muestra progreso mientras tanto.
    pub fn ensure_vectors_full(&mut self) -> Result<VectorStats, String> {
        self.ensure_vectors_budget(MAX_EMBED_PER_CALL, None)
    }

    /// Vectoriza los símbolos de archivos cuyo hash cambió, por lotes, hasta
    /// agotar el presupuesto (tope de símbolos y, opcionalmente, de tiempo).
    /// Devuelve cuántos quedan pendientes para las siguientes llamadas.
    fn ensure_vectors_budget(
        &mut self,
        max_symbols: usize,
        deadline: Option<Duration>,
    ) -> Result<VectorStats, String> {
        let _t = crate::trace::span("ensure_vectors");
        let t0 = Instant::now();

        // Purga primero los vectores de archivos ya borrados.
        let files = db::all_files(&self.conn)?;
        let present: HashSet<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        let vec_meta = db::all_vec_meta(&self.conn)?;
        for vp in vec_meta.keys() {
            if !present.contains(vp.as_str()) {
                db::remove_vectors(&self.conn, vp)?;
                self.vec_cache = None;
            }
        }

        let mut batch: Vec<Pending> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut embedded_now = 0usize;
        let mut remaining = 0usize;
        let mut spent = false; // presupuesto agotado: solo contamos backlog

        for (path, hash) in &files {
            if vec_meta.get(path).map(String::as_str) == Some(hash.as_str()) {
                continue;
            }
            let symbols = db::symbols_of(&self.conn, path)?;
            if symbols.is_empty() {
                db::replace_vectors(&mut self.conn, path, hash, &[])?;
                continue;
            }
            if spent {
                remaining += symbols.len();
                continue;
            }

            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let lines: Vec<&str> = content.lines().collect();
            for s in &symbols {
                let body = slice_lines(&lines, s.start_line, s.end_line);
                let mut t = format!("{}\n{}", s.name, body);
                if t.len() > EMBED_TEXT_MAX_CHARS {
                    // truncate() cuenta bytes y exige un límite de carácter válido;
                    // retrocede hasta uno para no partir un multibyte (á, ñ, emoji…).
                    let mut end = EMBED_TEXT_MAX_CHARS;
                    while !t.is_char_boundary(end) {
                        end -= 1;
                    }
                    t.truncate(end);
                }
                texts.push(t);
            }
            batch.push(Pending {
                path: path.clone(),
                hash: hash.clone(),
                symbols,
            });

            // Lote completo: embeber ya, y decidir si queda presupuesto. Cortar
            // por lotes (y no por archivo) permite respetar un deadline sin
            // dejar ningún archivo a medio vectorizar.
            if texts.len() >= EMBED_BATCH.min(max_symbols) {
                let t_batch = Instant::now();
                embedded_now += self.flush_batch(&batch, std::mem::take(&mut texts))?;
                batch.clear();
                // Predictivo: si el próximo lote (que costará como el anterior)
                // se pasaría del presupuesto, no lo empezamos. Comprobar solo
                // "ya me pasé" siempre lo excedía en un lote entero.
                let over_time = deadline
                    .map_or(false, |d| t0.elapsed() + t_batch.elapsed() > d);
                if over_time || embedded_now >= max_symbols {
                    spent = true;
                }
            }
        }

        if !batch.is_empty() {
            embedded_now += self.flush_batch(&batch, texts)?;
        }

        Ok(VectorStats {
            embedded_now,
            remaining,
        })
    }

    /// Carga los vectores a memoria si aún no están.
    fn load_vec_cache(&mut self) -> Result<(), String> {
        if self.vec_cache.is_some() {
            return Ok(());
        }
        let _t = crate::trace::span("  cargar vectores a RAM");
        let rows = db::load_vectors(&self.conn)?;
        self.vec_cache = Some(
            rows.into_iter()
                .map(|(m, blob)| (m, embed::from_blob(&blob)))
                .collect(),
        );
        Ok(())
    }

    /// Embebe un lote de archivos completos y persiste sus vectores. Devuelve
    /// cuántos símbolos se vectorizaron.
    fn flush_batch(&mut self, batch: &[Pending], texts: Vec<String>) -> Result<usize, String> {
        if batch.is_empty() {
            return Ok(0);
        }
        let n_texts = texts.len();
        self.ensure_embedder()?;
        let vecs = self.embedder.as_ref().unwrap().embed_docs(texts)?;

        let mut offset = 0;
        for p in batch {
            let n = p.symbols.len();
            let slice = &vecs[offset..offset + n];
            offset += n;
            let rows: Vec<(db::VecRow, Vec<u8>)> = p
                .symbols
                .iter()
                .zip(slice.iter())
                .map(|(s, v)| {
                    (
                        db::VecRow {
                            path: p.path.clone(),
                            name: s.name.clone(),
                            kind: s.kind.clone(),
                            start_line: s.start_line,
                            end_line: s.end_line,
                        },
                        embed::to_blob(v),
                    )
                })
                .collect();
            db::replace_vectors(&mut self.conn, &p.path, &p.hash, &rows)?;
        }

        // Mantener el caché al día sin recargarlo entero: fuera los vectores
        // viejos de estos archivos, dentro los recién calculados.
        if let Some(cache) = self.vec_cache.as_mut() {
            let touched: HashSet<&str> = batch.iter().map(|p| p.path.as_str()).collect();
            cache.retain(|(m, _)| !touched.contains(m.path.as_str()));
            let mut offset = 0;
            for p in batch {
                for s in &p.symbols {
                    cache.push((
                        db::VecRow {
                            path: p.path.clone(),
                            name: s.name.clone(),
                            kind: s.kind.clone(),
                            start_line: s.start_line,
                            end_line: s.end_line,
                        },
                        vecs[offset].clone(),
                    ));
                    offset += 1;
                }
            }
        }
        Ok(n_texts)
    }

    /// Búsqueda semántica por fuerza bruta (coseno) sobre los vectores del repo.
    pub fn search_semantic(&mut self, query: &str, n: usize) -> Result<Vec<Candidate>, String> {
        let _t = crate::trace::span("search_semantic (total)");
        self.ensure_embedder()?;
        let qv = {
            let _t = crate::trace::span("  embed query");
            self.embedder.as_ref().unwrap().embed_query(query)?
        };

        self.load_vec_cache()?;
        let _t2 = crate::trace::span("  coseno sobre vectores");
        let vectors = self.vec_cache.as_ref().expect("caché recién cargada");
        let mut scored: Vec<(f32, &db::VecRow)> = vectors
            .iter()
            .map(|(m, v)| (embed::cosine(&qv, v), m))
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(n);
        Ok(scored
            .into_iter()
            .map(|(_, m)| Candidate {
                path: m.path.clone(),
                name: m.name.clone(),
                kind: m.kind.clone(),
                start_line: m.start_line,
                end_line: m.end_line,
            })
            .collect())
    }

    pub fn symbol_count(&self) -> i64 {
        db::symbol_count(&self.conn)
    }

    pub fn vector_count(&self) -> i64 {
        db::vector_count(&self.conn)
    }
}

/// Fusión Reciprocal Rank Fusion de varias listas rankeadas → top-k candidatos.
pub fn rrf(lists: &[Vec<Candidate>], k: usize) -> Vec<Candidate> {
    use std::collections::HashMap;
    const K0: f64 = 60.0;
    let mut score: HashMap<String, f64> = HashMap::new();
    let mut meta: HashMap<String, Candidate> = HashMap::new();
    for list in lists {
        for (rank, c) in list.iter().enumerate() {
            let key = c.key();
            *score.entry(key.clone()).or_insert(0.0) += 1.0 / (K0 + rank as f64 + 1.0);
            meta.entry(key).or_insert_with(|| c.clone());
        }
    }
    let mut ranked: Vec<(String, f64)> = score.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(k);
    ranked
        .into_iter()
        .filter_map(|(key, _)| meta.get(&key).cloned())
        .collect()
}

/// Recorre el árbol en paralelo y devuelve los archivos indexables con su
/// mtime. Respeta `.gitignore` (vía `ignore`) y salta el propio índice y el
/// histórico archivado de OpenSpec.
fn walk_files(root: &str) -> Vec<(PathBuf, i64)> {
    let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, i64)>();
    WalkBuilder::new(root)
        .hidden(false)
        .build_parallel()
        .run(|| {
            let tx = tx.clone();
            Box::new(move |result| {
                if let Ok(entry) = result {
                    if entry.file_type().map_or(false, |ft| ft.is_file()) {
                        let path = entry.path();
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        let skip = !chunker::is_supported(ext)
                            || path.components().any(|c| c.as_os_str() == ".rtk-index")
                            || path.to_string_lossy().contains("openspec/changes/archive/");
                        if !skip {
                            let mtime = entry
                                .metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            let _ = tx.send((entry.into_path(), mtime));
                        }
                    }
                }
                ignore::WalkState::Continue
            })
        });
    drop(tx); // cierra el canal: sin esto, `rx` esperaría para siempre
    rx.into_iter().collect()
}

fn slice_lines(lines: &[&str], start: usize, end: usize) -> String {
    if start == 0 || start > lines.len() {
        return String::new();
    }
    let end = end.min(lines.len());
    lines[start - 1..end].join("\n")
}

/// Lee un rango de líneas [start, end] (1-indexed, inclusivo) con números de línea.
pub fn read_line_range(path: &str, start: usize, end: usize) -> Result<String, String> {
    let content = fs::read_to_string(PathBuf::from(path))
        .map_err(|e| format!("read {}: {}", path, e))?;
    let lines: Vec<&str> = content.lines().collect();
    if start == 0 || start > lines.len() {
        return Err(format!("rango fuera de límites en {}", path));
    }
    let end = end.min(lines.len());
    let mut out = String::new();
    for (i, line) in lines[start - 1..end].iter().enumerate() {
        out.push_str(&format!("{:>5}\t{}\n", start + i, line));
    }
    Ok(out)
}
