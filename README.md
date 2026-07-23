# RTK MCP Server

Un servidor **Model Context Protocol (MCP)** de alto rendimiento escrito en Rust, diseñado para darle a tu asistente de IA (Claude Code, Cursor, Cline, etc.) **superpoderes de Inteligencia de Código (RAG local) y lectura ultrarrápida**.

Este proyecto nació como un puente para integrar *Rust Token Killer (RTK)*, pero evolucionó hacia un motor avanzado de búsqueda semántica y AST que evita el desperdicio masivo de tokens al proporcionar a la IA el contexto quirúrgico exacto en lugar de leer archivos completos.

---

## ⚡ Características Principales

*   **Motor RAG Incremental:** Indexa tu código de forma invisible. Utiliza `blake3` para hashear archivos y solo reindexa lo que ha cambiado, manteniendo el estado en SQLite.
*   **Búsqueda BM25 (Full-Text):** Indexa el código usando `tantivy` para búsquedas a la velocidad de la luz, devolviendo fragmentos exactos (`path:start_line-end_line`).
*   **AST nativo con Tree-Sitter:** Analiza la sintaxis de TypeScript, Rust y Python para extraer las firmas exactas de clases, funciones y estructuras (sin leer el código muerto).
*   **Búsqueda Semántica Local (Vectores):** Genera embeddings semánticos localmente utilizando `fastembed` y los modelos ONNX.
*   **Integración con RTK:** Wrappers nativos para ejecutar `rtk grep` y `rtk find` logrando ahorros de tokens del 60% al 80%.
*   **Velocidad Pura en Rust:** Lectura nativa multihilo (`std::fs`) que elude por completo los cuellos de botella de subprocesos de shell de las herramientas tradicionales.

---

## 🛠️ Herramientas Expuestas a la IA

El servidor expone el siguiente menú de herramientas a tu cliente MCP:

| Herramienta | Descripción |
| :--- | :--- |
| `codebase_search` | Realiza una búsqueda BM25 sobre el código, devolviendo fragmentos relevantes con su ancla de líneas. Ideal para orientarse sin leer archivos. |
| `symbol_lookup` | Busca la definición exacta de un símbolo (función/clase) por nombre utilizando el índice de SQLite persistente. |
| `file_outline` | Extrae las firmas de un archivo usando `tree-sitter` (sin leer el cuerpo) gastando mínimos tokens. |
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
   Navega a la raíz del proyecto de código que deseas que la IA analice y ejecuta el comando `init`. Esto pre-calculará los vectores semánticos, el árbol AST y creará automáticamente el archivo `.mcp.json`.
   ```bash
   /ruta/a/rtk-mcp-server/target/release/rtk-mcp-server init .
   ```
   *¡Listo! Reinicia Claude Code en ese directorio y las herramientas ya estarán inyectadas a máxima velocidad.*

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
```bash
claude mcp add rtk-server /ruta/absoluta/a/tu/rtk-mcp-server/target/release/rtk-mcp-server
```

---

## ⚙️ Arquitectura Interna (src/index)

La verdadera magia ocurre en la indexación de contexto local, que divide el código en dos almacenes sincronizados (`.rtk-index/`):
*   **Base de datos Léxica y Relacional (SQLite):** Hashes (Blake3), estado de archivos, y anclas exactas de los símbolos extraídos por Tree-sitter.
*   **Motor de Texto y Vectores (Tantivy + Fastembed):** Índices semánticos limitados inteligentemente a `MAX_EMBED_PER_CALL = 800` para garantizar que la primera ejecución de tu IA sobre un monorepo no cuelgue la máquina, sino que se caliente (warm-up) de manera incremental.
