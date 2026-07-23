# RTK MCP Server

Un servidor **Model Context Protocol (MCP)** de alto rendimiento escrito en Rust, diseñado para darle a tu asistente de IA (Claude Code, Cursor, Cline, etc.) **superpoderes de Inteligencia de Código (RAG local) y lectura ultrarrápida**.

Este proyecto nació como un puente para integrar *Rust Token Killer (RTK)*, pero evolucionó hacia un motor avanzado de búsqueda semántica y AST que evita el desperdicio masivo de tokens al proporcionar a la IA el contexto quirúrgico exacto en lugar de leer archivos completos.

---

## ⚡ Características Principales

*   **Motor RAG Incremental:** Indexa tu código de forma invisible. Utiliza `blake3` para hashear archivos y solo reindexa lo que ha cambiado, manteniendo el estado en SQLite.
*   **Búsqueda BM25 (Full-Text):** Indexa el código usando `tantivy` para búsquedas a la velocidad de la luz, devolviendo fragmentos exactos (`path:start_line-end_line`).
*   **AST nativo con Tree-Sitter:** Analiza la sintaxis de TypeScript/JS/TSX, Rust y Python para extraer las firmas exactas de clases, funciones y estructuras (sin leer el cuerpo completo).
*   **Búsqueda Semántica Local (Vectores):** Genera embeddings localmente con `fastembed` (ONNX) usando un modelo **multilingüe** (E5-small) — ideal para bases de código con identificadores y comentarios en español. Se fusiona con BM25 (Reciprocal Rank Fusion) en `codebase_search`.
*   **Integración con RTK:** Wrappers nativos para ejecutar `rtk grep` y `rtk find` logrando ahorros de tokens del 60% al 80%.
*   **Menos tokens, no solo "más rápido":** El objetivo no es leer archivos a mayor velocidad (el I/O nativo apenas cambia el costo), sino **leer menos**: entregar a la IA el fragmento exacto en lugar del archivo completo. Para el caso en que sí necesitas varios archivos enteros, `bulk_read_files` usa lectura nativa multihilo (`std::fs`).

---

## 🛠️ Herramientas Expuestas a la IA

El servidor expone el siguiente menú de herramientas a tu cliente MCP:

| Herramienta | Descripción |
| :--- | :--- |
| `codebase_search` | Búsqueda **híbrida (BM25 + semántica, fusión RRF)** sobre el código: devuelve los fragmentos más relevantes con su ancla `path:líneas`. Encuentra *qué tocar* a partir de una descripción, aunque no conozcas los nombres exactos. |
| `symbol_lookup` | Busca la definición exacta de un símbolo (función/clase) por nombre utilizando el índice de SQLite persistente. |
| `file_outline` | Extrae las firmas de un archivo usando `tree-sitter` (sin leer el cuerpo) gastando mínimos tokens. Ideal para orientarse en un archivo grande. |
| `rtk_grep` | Búsqueda ultra-rápida vía `rtk grep` comprimiendo radicalmente los resultados (hasta 70% de ahorro de tokens). |
| `rtk_find` | Búsqueda y listado comprimido de estructura de directorios vía `rtk find`. |
| `bulk_read_files` | Lectura masiva nativa y paralela que enumera las líneas estilo `cat -n`. Extremadamente veloz gracias al paralelismo de hilos de Rust. |
| `get_minified_diff` | Ejecuta `git diff` canalizando los cambios de manera ultra-comprimida. |

---

## 🚀 Inicio Rápido (2 pasos)

**1. Instala el comando `rtk-index`.** Elige una:
```bash
# Con cargo (requiere Rust; funciona hoy en cualquier plataforma):
cargo install --git https://github.com/jesusamador-dev/rtk-mcp-server --bin rtk-index

# O con el script (binario precompilado si existe para tu plataforma; si no, cae a cargo):
curl -fsSL https://raw.githubusercontent.com/jesusamador-dev/rtk-mcp-server/main/install.sh | sh
```
Cualquiera deja `rtk-index` en tu `PATH` (`~/.cargo/bin` con cargo, o `~/.local/bin` con el script).

**2. Inicializa tu proyecto** (desde su raíz):
```bash
cd tu-proyecto
rtk-index init .
```
Esto indexa AST + BM25, **vectoriza el proyecto completo una sola vez** y crea/actualiza `.mcp.json`. Luego **reinicia Claude Code** y las herramientas quedan calientes, sin esperas.

> ⚠️ **Primera ejecución:** descarga una vez el modelo de embeddings multilingüe (~470 MB, requiere conexión) a `~/.cache/rtk-mcp-server/`. Vectorizar un monorepo grande tarda unos minutos; es un costo único — después `init` es incremental (solo re-vectoriza lo que cambió). La escritura de `.mcp.json` es segura: preserva los servidores que ya tuvieras.

**¿Dudas del entorno?** `rtk-index check` verifica que todo esté listo (rtk, modelo, git).

<details>
<summary>Compilar desde el repo (alternativa)</summary>

```bash
git clone git@github.com:jesusamador-dev/rtk-mcp-server.git
cd rtk-mcp-server && cargo build --release   # binario en target/release/rtk-index
```
</details>

---

## 🔌 Configuración Manual en otros clientes MCP (Cursor, Claude Desktop)

Los servidores MCP que utilizan el protocolo `stdio` no se ejecutan como un demonio (daemon) tradicional. Tu cliente de IA iniciará el servidor automáticamente al arrancar. Solo debes añadir la ruta absoluta de tu ejecutable.

> Tras instalar, el comando queda como `rtk-index` en tu `PATH` (ej. `~/.local/bin/rtk-index` o `~/.cargo/bin/rtk-index`). Usa esa ruta abajo.

### En Claude Desktop
Edita el archivo de configuración `claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "rtk-index": {
      "command": "/ruta/absoluta/a/rtk-index",
      "args": ["serve", "--root", "/ruta/a/tu/proyecto"]
    }
  }
}
```

### En Cursor IDE
1. Ve a **Cursor Settings > Features > MCP Servers**.
2. Da click en **+ Add New MCP Server**.
3. **Type:** `command`
4. **Name:** `rtk-index`
5. **Command:** `/ruta/absoluta/a/rtk-index`

### En Claude Code (CLI)
Lo más simple es correr `rtk-index init .` (configura `.mcp.json` por ti). Si prefieres registrarlo a mano, pasa los argumentos tras `--` para no perder la raíz del proyecto:
```bash
claude mcp add rtk-index -- /ruta/absoluta/a/rtk-index serve --root /ruta/a/tu/proyecto
```

---

## ⚙️ Arquitectura Interna (src/index)

La verdadera magia ocurre en la indexación de contexto local, que divide el código en dos almacenes sincronizados (`.rtk-index/`):
*   **Base de datos Léxica y Relacional (SQLite):** Hashes (Blake3), estado de archivos, y anclas exactas de los símbolos extraídos por Tree-sitter.
*   **Motor de Texto y Vectores (Tantivy + Fastembed):** Índices semánticos limitados inteligentemente a `MAX_EMBED_PER_CALL = 800` para garantizar que la primera ejecución de tu IA sobre un monorepo no cuelgue la máquina, sino que se caliente (warm-up) de manera incremental.
