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

## 🚀 Instalación y Uso (CLI)

Este proyecto incluye una interfaz de línea de comandos (CLI) que hace todo el trabajo pesado de indexación y configuración por ti.

1. Clona el repositorio y compila en modo Release:
   ```bash
   git clone git@github.com:jesusamador-dev/rtk-mcp-server.git
   cd rtk-mcp-server
   cargo build --release
   ```
   *Opcional: puedes mover el binario resultante a tu `$PATH` (ej. `/usr/local/bin/rtk-mcp-server`).*

2. **Inicializa tu proyecto (Recomendado para Claude Code):**
   Navega a la raíz del proyecto de código que deseas que la IA analice y ejecuta el comando `init`. Esto indexa el AST y BM25, **vectoriza el proyecto completo de una sola vez** (sin el tope por-llamada) y crea/actualiza automáticamente el archivo `.mcp.json`.
   ```bash
   /ruta/a/rtk-mcp-server/target/release/rtk-mcp-server init .
   ```
   > ⚠️ **Primera ejecución:** descarga una vez el modelo de embeddings (~130 MB, requiere conexión) y lo cachea en `~/.cache/rtk-mcp-server/`. La vectorización de un monorepo grande puede tardar unos minutos; es un costo único — después `init` es incremental (solo re-vectoriza lo que cambió).

   *¡Listo! Reinicia Claude Code en ese directorio y las herramientas quedan calientes, sin esperas. La escritura de `.mcp.json` es segura: preserva los servidores que ya tuvieras configurados.*

3. **Arrancar el servidor (uso interno de clientes):**
   El cliente MCP lanzará automáticamente el servidor en segundo plano usando el comando `serve`:
   ```bash
   rtk-mcp-server serve --root .
   ```

---

## 🔌 Configuración Manual en otros clientes MCP (Cursor, Claude Desktop)

Los servidores MCP que utilizan el protocolo `stdio` no se ejecutan como un demonio (daemon) tradicional. Tu cliente de IA iniciará el servidor automáticamente al arrancar. Solo debes añadir la ruta absoluta de tu ejecutable.

### En Claude Desktop
Edita el archivo de configuración `claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "rtk-index": {
      "command": "/ruta/absoluta/a/tu/rtk-mcp-server/target/release/rtk-mcp-server",
      "args": ["serve", "--root", "/ruta/a/tu/proyecto"]
    }
  }
}
```

### En Cursor IDE
1. Ve a **Cursor Settings > Features > MCP Servers**.
2. Da click en **+ Add New MCP Server**.
3. **Type:** `command`
4. **Name:** `rtk-server`
5. **Command:** `/ruta/absoluta/a/tu/rtk-mcp-server/target/release/rtk-mcp-server`

### En Claude Code (CLI)
Lo más simple es correr `init` (configura `.mcp.json` por ti). Si prefieres registrarlo a mano, pasa los argumentos tras `--` para no perder la raíz del proyecto:
```bash
claude mcp add rtk-index -- /ruta/absoluta/a/rtk-mcp-server/target/release/rtk-mcp-server serve --root /ruta/a/tu/proyecto
```

---

## ⚙️ Arquitectura Interna (src/index)

La verdadera magia ocurre en la indexación de contexto local, que divide el código en dos almacenes sincronizados (`.rtk-index/`):
*   **Base de datos Léxica y Relacional (SQLite):** Hashes (Blake3), estado de archivos, y anclas exactas de los símbolos extraídos por Tree-sitter.
*   **Motor de Texto y Vectores (Tantivy + Fastembed):** Índices semánticos limitados inteligentemente a `MAX_EMBED_PER_CALL = 800` para garantizar que la primera ejecución de tu IA sobre un monorepo no cuelgue la máquina, sino que se caliente (warm-up) de manera incremental.
