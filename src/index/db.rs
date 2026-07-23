//! Índice persistente de símbolos en SQLite.
//!
//! Clave por hash blake3 del contenido de cada archivo → sincronización
//! incremental estilo merkle: solo se re-indexa lo que cambió.

use rusqlite::{params, Connection};
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS files (
    path  TEXT PRIMARY KEY,
    hash  TEXT NOT NULL,
    mtime INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS symbols (
    id         INTEGER PRIMARY KEY,
    path       TEXT NOT NULL,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    signature  TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_path ON symbols(path);

CREATE TABLE IF NOT EXISTS vec_meta (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS vectors (
    id         INTEGER PRIMARY KEY,
    path       TEXT NOT NULL,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line   INTEGER NOT NULL,
    vec        BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_vectors_path ON vectors(path);
";

#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub fn open(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("open db: {}", e))?;
    // WAL + sincronización relajada: es un caché reconstruible, priorizamos velocidad.
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    conn.execute_batch(SCHEMA)
        .map_err(|e| format!("init schema: {}", e))?;
    Ok(conn)
}

/// hash+mtime almacenados para un archivo (para decidir si re-indexar).
pub fn stored_meta(conn: &Connection, path: &str) -> Option<(String, i64)> {
    conn.query_row(
        "SELECT hash, mtime FROM files WHERE path = ?1",
        params![path],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )
    .ok()
}

pub fn all_paths(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT path FROM files")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// (path, hash) de todos los archivos indexados — para decidir qué re-embeber.
pub fn all_files(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT path, hash FROM files")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Símbolos de un archivo concreto (para re-embeber sus chunks).
pub fn symbols_of(conn: &Connection, path: &str) -> Result<Vec<SymbolRow>, String> {
    query_rows(
        conn,
        "SELECT path, name, kind, signature, start_line, end_line
         FROM symbols WHERE path = ?1 ORDER BY start_line",
        params![path],
    )
}

/// Reemplaza los símbolos de un archivo y actualiza su metadata en una sola transacción.
pub fn replace_file(
    conn: &mut Connection,
    path: &str,
    hash: &str,
    mtime: i64,
    symbols: &[crate::index::chunker::Symbol],
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM symbols WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    {
        let mut ins = tx
            .prepare(
                "INSERT INTO symbols (path, name, kind, signature, start_line, end_line)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| e.to_string())?;
        for s in symbols {
            ins.execute(params![
                path,
                s.name,
                s.kind,
                s.signature,
                s.start_line as i64,
                s.end_line as i64
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.execute(
        "INSERT INTO files (path, hash, mtime) VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET hash = ?2, mtime = ?3",
        params![path, hash, mtime],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_file(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("DELETE FROM symbols WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM files WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Búsqueda por nombre: primero exacta, y si no hay nada, subcadena.
pub fn lookup(conn: &Connection, name: &str) -> Result<Vec<SymbolRow>, String> {
    let exact = query_rows(
        conn,
        "SELECT path, name, kind, signature, start_line, end_line
         FROM symbols WHERE name = ?1 ORDER BY path, start_line",
        params![name],
    )?;
    if !exact.is_empty() {
        return Ok(exact);
    }
    let like = format!("%{}%", name);
    query_rows(
        conn,
        "SELECT path, name, kind, signature, start_line, end_line
         FROM symbols WHERE name LIKE ?1 ORDER BY path, start_line LIMIT 50",
        params![like],
    )
}

fn query_rows(
    conn: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<SymbolRow>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(p, |r| {
            Ok(SymbolRow {
                path: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                signature: r.get(3)?,
                start_line: r.get::<_, i64>(4)? as usize,
                end_line: r.get::<_, i64>(5)? as usize,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn symbol_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
        .unwrap_or(0)
}

// ---- Vectores (búsqueda semántica) ----

/// Metadatos de un chunk con vector, para reconstruir el ancla en la búsqueda.
#[derive(Debug, Clone)]
pub struct VecRow {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub fn vec_meta_hash(conn: &Connection, path: &str) -> Option<String> {
    conn.query_row(
        "SELECT hash FROM vec_meta WHERE path = ?1",
        params![path],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

pub fn all_vec_paths(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT path FROM vec_meta")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Reemplaza los vectores de un archivo y su hash, en una transacción.
pub fn replace_vectors(
    conn: &mut Connection,
    path: &str,
    hash: &str,
    rows: &[(VecRow, Vec<u8>)],
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM vectors WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    {
        let mut ins = tx
            .prepare(
                "INSERT INTO vectors (path, name, kind, start_line, end_line, vec)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| e.to_string())?;
        for (m, blob) in rows {
            ins.execute(params![
                m.path,
                m.name,
                m.kind,
                m.start_line as i64,
                m.end_line as i64,
                blob
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.execute(
        "INSERT INTO vec_meta (path, hash) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET hash = ?2",
        params![path, hash],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_vectors(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("DELETE FROM vectors WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM vec_meta WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Carga todos los vectores (metadatos + BLOB) para la búsqueda por fuerza bruta.
pub fn load_vectors(conn: &Connection) -> Result<Vec<(VecRow, Vec<u8>)>, String> {
    let mut stmt = conn
        .prepare("SELECT path, name, kind, start_line, end_line, vec FROM vectors")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                VecRow {
                    path: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    start_line: r.get::<_, i64>(3)? as usize,
                    end_line: r.get::<_, i64>(4)? as usize,
                },
                r.get::<_, Vec<u8>>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn vector_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get(0))
        .unwrap_or(0)
}
