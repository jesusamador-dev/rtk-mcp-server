//! Embeddings locales con fastembed (ONNX, CPU). Modelo BGE-small-en-v1.5,
//! 384 dimensiones. El modelo se descarga una vez (~130MB) y queda cacheado.
//!
//! La inicialización es cara (carga del modelo), así que se hace de forma
//! perezosa: solo la búsqueda semántica la dispara, nunca symbol_lookup.

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::PathBuf;


/// Caché del modelo en una ruta ABSOLUTA y estable. Sin esto, fastembed cachea
/// en `./.fastembed_cache` relativo al CWD y re-descarga ~130MB en cada arranque
/// desde un directorio distinto.
fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".cache/rtk-mcp-server/fastembed")
}

pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    pub fn new() -> Result<Self, String> {
        let dir = cache_dir();
        std::fs::create_dir_all(&dir).ok();
        // Modelo MULTILINGÜE (E5-small): kuosel-core tiene identificadores y
        // comentarios en español, y las consultas también. Un modelo solo-inglés
        // genera vectores ruidosos para español y degrada la búsqueda.
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small).with_cache_dir(dir),
        )
        .map_err(|e| format!("init embedder: {}", e))?;
        Ok(Embedder { model })
    }

    /// Embebe documentos (chunks de código). E5 requiere el prefijo "passage: ".
    pub fn embed_docs(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        let prefixed: Vec<String> = texts.into_iter().map(|t| format!("passage: {}", t)).collect();
        self.embed_raw(prefixed)
    }

    /// Embebe una consulta. E5 requiere el prefijo "query: ".
    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>, String> {
        let mut v = self.embed_raw(vec![format!("query: {}", text)])?;
        v.pop().ok_or_else(|| "embed devolvió vacío".to_string())
    }

    fn embed_raw(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        let mut out = self
            .model
            .embed(texts, Some(256))
            .map_err(|e| format!("embed: {}", e))?;
        for v in out.iter_mut() {
            normalize(v);
        }
        Ok(out)
    }
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Producto punto entre dos vectores normalizados = similitud coseno.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Serializa un vector f32 a bytes little-endian para guardarlo como BLOB.
pub fn to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Reconstruye un vector f32 desde un BLOB little-endian.
pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
