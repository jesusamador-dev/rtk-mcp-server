//! Índice de texto completo (BM25) sobre los chunks de código, con tantivy.
//!
//! El cuerpo se indexa pero NO se almacena: para devolver el código releemos el
//! rango de líneas del archivo (igual que symbol_lookup), manteniendo el índice
//! pequeño. Se almacenan solo los metadatos necesarios para el ancla.

use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, Value, INDEXED, STORED, STRING, TEXT};
use tantivy::{doc, Index, IndexReader, IndexWriter, TantivyDocument, Term};

pub struct Hit {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    #[allow(dead_code)]
    pub score: f32,
}

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    f_path: Field,
    f_name: Field,
    f_kind: Field,
    f_start: Field,
    f_end: Field,
    f_body: Field,
}

impl SearchIndex {
    pub fn open(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("create tantivy dir: {}", e))?;

        let mut sb = Schema::builder();
        let f_path = sb.add_text_field("path", STRING | STORED);
        let f_name = sb.add_text_field("name", STRING | STORED);
        let f_kind = sb.add_text_field("kind", STRING | STORED);
        let f_start = sb.add_u64_field("start_line", INDEXED | STORED);
        let f_end = sb.add_u64_field("end_line", STORED);
        let f_body = sb.add_text_field("body", TEXT); // indexado para BM25, no almacenado
        let schema = sb.build();

        let mmap = MmapDirectory::open(dir).map_err(|e| format!("open tantivy dir: {}", e))?;
        let index = Index::open_or_create(mmap, schema).map_err(|e| format!("open index: {}", e))?;
        let reader = index.reader().map_err(|e| format!("reader: {}", e))?;

        Ok(SearchIndex {
            index,
            reader,
            f_path,
            f_name,
            f_kind,
            f_start,
            f_end,
            f_body,
        })
    }

    pub fn num_docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    pub fn writer(&self) -> Result<IndexWriter, String> {
        self.index
            .writer(50_000_000)
            .map_err(|e| format!("writer: {}", e))
    }

    pub fn delete_file(&self, writer: &IndexWriter, path: &str) {
        writer.delete_term(Term::from_field_text(self.f_path, path));
    }

    pub fn add_chunk(
        &self,
        writer: &IndexWriter,
        path: &str,
        name: &str,
        kind: &str,
        start_line: usize,
        end_line: usize,
        body: &str,
    ) -> Result<(), String> {
        // El nombre se antepone al cuerpo (pesa más), y todo se pasa por code_split
        // para que camelCase/snake_case se indexen como términos separados.
        let indexed_body = code_split(&format!("{} {}", name, body));
        writer
            .add_document(doc!(
                self.f_path => path,
                self.f_name => name,
                self.f_kind => kind,
                self.f_start => start_line as u64,
                self.f_end => end_line as u64,
                self.f_body => indexed_body,
            ))
            .map_err(|e| format!("add doc: {}", e))?;
        Ok(())
    }

    pub fn commit_and_reload(&self, mut writer: IndexWriter) -> Result<(), String> {
        writer.commit().map_err(|e| format!("commit: {}", e))?;
        self.reader.reload().map_err(|e| format!("reload: {}", e))?;
        Ok(())
    }

    pub fn query(&self, query_str: &str, k: usize) -> Result<Vec<Hit>, String> {
        let searcher = self.reader.searcher();
        let qp = QueryParser::for_index(&self.index, vec![self.f_body]);
        // Default OR (should): BM25 rankea; los fragmentos con más/mejores términos
        // suben. Mejor recall para consultas conceptuales de "qué tocar".

        // Mismo code_split que en indexado, para que camelCase de la query coincida.
        let split = code_split(query_str);
        let query = qp
            .parse_query(split.trim())
            .map_err(|e| format!("parse query: {}", e))?;

        let top = searcher
            .search(&query, &TopDocs::with_limit(k))
            .map_err(|e| format!("search: {}", e))?;

        let mut hits = Vec::new();
        for (score, addr) in top {
            let d: TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| format!("fetch doc: {}", e))?;
            let get_str = |f: Field| -> String {
                d.get_first(f)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let get_u64 = |f: Field| -> usize {
                d.get_first(f).and_then(|v| v.as_u64()).unwrap_or(0) as usize
            };
            hits.push(Hit {
                path: get_str(self.f_path),
                name: get_str(self.f_name),
                kind: get_str(self.f_kind),
                start_line: get_u64(self.f_start),
                end_line: get_u64(self.f_end),
                score,
            });
        }
        Ok(hits)
    }
}

/// Tokenización consciente de código: parte camelCase y acrónimos, y convierte
/// cualquier no-alfanumérico (incl. `_`, `.`, `-`) en espacio. `renewHeartbeat`
/// → "renew Heartbeat", `HTTPServer` → "HTTP Server", `token_hash` → "token hash".
/// El tokenizador por defecto de tantivy luego minúscula y separa por espacios.
fn code_split(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() * 2);
    for i in 0..chars.len() {
        let c = chars[i];
        if !c.is_alphanumeric() {
            out.push(' ');
            continue;
        }
        if i > 0 {
            let p = chars[i - 1];
            let camel = p.is_lowercase() && c.is_uppercase();
            let acronym = p.is_uppercase()
                && c.is_uppercase()
                && i + 1 < chars.len()
                && chars[i + 1].is_lowercase();
            if camel || acronym {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}
