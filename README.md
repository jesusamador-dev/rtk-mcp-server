# RTK MCP Server

Un servidor **Model Context Protocol (MCP)** de alto rendimiento escrito en Rust, diseñado para darle a tu asistente de IA (Claude Code, Cursor, Cline, etc.) **superpoderes de Inteligencia de Código (RAG local) y lectura ultrarrápida**.

Este proyecto nació como un puente para integrar *Rust Token Killer (RTK)*, pero evolucionó hacia un motor avanzado de búsqueda semántica y AST que evita el desperdicio masivo de tokens al proporcionar a la IA el contexto quirúrgico exacto en lugar de leer archivos completos.

---

## ⚡ Características Principales

*   **Motor RAG Incremental:** Indexa tu código de forma invisible. Utiliza `blake3` para hashear archivos y solo reindexa lo que ha cambiado, manteniendo el estado en SQLite.
*   **Búsqueda BM25 (Full-Text):** Indexa el código usando `tantivy` para búsquedas a la velocidad de la luz, devolviendo fragmentos exactos (`path:start_line-end_line`).
*   **AST nativo con Tree-Sitter:** Analiza la sintaxis de TypeScript/JS/TSX, Rust y Python para extraer las firmas exactas de clases, funciones y estructuras (sin leer el cuerpo completo).
*   **Búsqueda Semántica Local (Vectores):** Genera embeddings localmente con `fastembed` (ONNX) usando un modelo **multilingüe** (E5-small) — ideal para bases de código con identificadores y comentarios en español. Se fusiona con BM25 (Reciprocal Rank Fusion) en `codebase_search`.
*   **Indexado de specs (SDD):** Detecta si el repo usa **OpenSpec** (`openspec/`) o **Spec Kit** (`.specify/` · `memory/constitution.md`) e indexa sus specs markdown por encabezado (`#`/`##`…), excluyendo `openspec/changes/archive/`. Así `codebase_search` recupera secciones de spec **y** código juntos — clave para no quemar tokens releyendo specs enteras en flujos Spec-Driven Development.
*   **Integración con RTK:** `rtk_grep` delega en `rtk grep` (comprimido). `rtk_find` es nativo: usa el mismo recorrido que el indexador, así que respeta `.gitignore` y no te devuelve `node_modules` ni el histórico archivado.
*   **Menos tokens, no solo "más rápido":** El objetivo no es leer archivos a mayor velocidad (el I/O nativo apenas cambia el costo), sino **leer menos**: entregar a la IA el fragmento exacto en lugar del archivo completo. Para el caso en que sí necesitas varios archivos enteros, `bulk_read_files` usa lectura nativa multihilo (`std::fs`).
*   **Telemetría de ahorro medido:** cada llamada compara la respuesta real contra su *baseline* (leer los archivos completos, o la salida cruda de `grep`/`find`/`git diff`) y lo registra. `rtk-index gain` te muestra cuántos tokens llevas ahorrados, por herramienta y por proyecto.

---

## 🛠️ Herramientas Expuestas a la IA

El servidor expone el siguiente menú de herramientas a tu cliente MCP:

| Herramienta | Descripción |
| :--- | :--- |
| `codebase_search` | Búsqueda **híbrida (BM25 + semántica, fusión RRF)** sobre el código: devuelve los fragmentos más relevantes con su ancla `path:líneas`. Encuentra *qué tocar* a partir de una descripción, aunque no conozcas los nombres exactos. |
| `symbol_lookup` | Busca la definición exacta de un símbolo (función/clase) por nombre utilizando el índice de SQLite persistente. |
| `file_outline` | Extrae las firmas de un archivo usando `tree-sitter` (sin leer el cuerpo) gastando mínimos tokens. Ideal para orientarse en un archivo grande. |
| `rtk_grep` | `grep` comprimido vía `rtk`, con su ancla `path:línea`. Para patrones exactos; para buscar por concepto, `codebase_search`. |
| `rtk_find` | Lista archivos agrupados por directorio, **respetando `.gitignore`** (excluye `.git`, `.rtk-index` y el histórico de OpenSpec). Tope de 400, y dice cuántos quedaron fuera. |
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
Esto indexa AST + BM25, **vectoriza el proyecto completo una sola vez** y crea/actualiza `.mcp.json` con `alwaysLoad: true` (las herramientas quedan **siempre visibles** para el modelo, sin quedar diferidas por el *tool-search* aunque tengas muchos otros MCP conectados). Luego **reinicia Claude Code** y quedan calientes, sin esperas.

> Requiere **Claude Code v2.1.121+** (para `alwaysLoad`). Sin esa versión las herramientas siguen funcionando, solo que podrían diferirse.

> ⚠️ **Primera ejecución:** descarga una vez el modelo de embeddings multilingüe (~470 MB, requiere conexión) a `~/.cache/rtk-mcp-server/`. Vectorizar un monorepo grande tarda unos minutos; es un costo único — después `init` es incremental (solo re-vectoriza lo que cambió). La escritura de `.mcp.json` es segura: preserva los servidores que ya tuvieras.

**¿Dudas del entorno?** `rtk-index check` verifica que todo esté listo (rtk, modelo, git).

**¿Cuánto estás ahorrando?** `rtk-index gain` (ver abajo).

**Para actualizar:** `rtk-index update` — un solo comando, sin recordar cómo lo instalaste.

---

## 🔄 Actualizar: `rtk-index update`

Compara tu versión con la publicada en `main` y reinstala **con el mismo método con el que ya está instalado**: `cargo install --force` si el binario vive en el bin de cargo, o el `install.sh` del repo en cualquier otro caso (que a su vez usa el binario precompilado si existe para tu plataforma).

```bash
rtk-index update            # actualiza si hay versión nueva
rtk-index update --check    # solo comprueba; sale con 1 si hay una nueva (útil en scripts)
rtk-index update --force    # reinstala aunque estés al día
rtk-index update --cargo    # fuerza el método cargo
rtk-index update --script   # fuerza el método install.sh
rtk-index --version
```

El índice de tus proyectos, su `.mcp.json` y la telemetría **se conservan**: solo se reemplaza el binario. Tras actualizar, reinicia Claude Code para que recargue el servidor MCP.

---

## 📊 Medir el ahorro: `rtk-index gain`

Cada llamada MCP se registra con dos magnitudes: el **baseline** (lo que habría costado obtener la misma información sin la herramienta) y el coste **real** de la respuesta que recibe el modelo.

| Herramienta | Baseline con el que se compara |
| :--- | :--- |
| `codebase_search` | Los archivos completos donde caen los resultados (lo que habría costado abrirlos con `Read`) |
| `symbol_lookup` | Los archivos completos que contienen alguna definición del símbolo |
| `file_outline` | El archivo completo |
| `rtk_grep` | La salida cruda de `grep -rn` sobre el mismo alcance |
| `rtk_find` | Las mismas rutas sin agrupar, una por línea (mide la compresión, no la exclusión) |
| `get_minified_diff` | El `git diff` nativo |
| `bulk_read_files` | Su propia salida — devuelve los archivos enteros, **no ahorra** (0 %) |

Si un baseline no se puede medir (o la llamada falla), se usa el coste real: 0 % de ahorro. **Los ahorros negativos se reportan como tales**: si la respuesta salió más cara que su rival, lo verás con signo menos.

> ⚠️ **El baseline es un contrafactual, y puede no ser el tuyo.** El ahorro se mide contra leer el archivo completo o la salida cruda del comando. Si tu alternativa real era un `grep` dirigido o un `ls` de un solo directorio, el ahorro es menor —y puede ser negativo—. Un "−95 % vs find crudo" no significa que hayas ahorrado: significa que un `find` crudo habría costado eso. Por eso el pie de cada respuesta pone **primero lo que cuesta** y nombra su rival, y por eso existe `--worst`.

```bash
rtk-index gain                    # resumen global
rtk-index gain --project .        # solo este workspace
rtk-index gain --since 7d         # 30m · 24h · 7d · 2w
rtk-index gain --tool codebase_search
rtk-index gain --history 30       # últimas 30 llamadas
rtk-index gain --worst 15         # las 15 llamadas MÁS CARAS en tokens reales
rtk-index gain --json             # para scripts
rtk-index gain --reset            # borra el historial
```

```
  rtk-index gain — ahorro de tokens

  todo el histórico · rtk-mcp-server · 142 llamadas · desde hace 12d

  Baseline (archivos completos / salida cruda)         1.2M
  Real     (respuesta de rtk-index)                  148.0K
  ──────────────────────────────────────────────────────────
  AHORRO                                               1.1M   (88 %)

  Por herramienta
  HERRAMIENTA         LLAM.   BASELINE       REAL   AHORRO    MEDIA
  codebase_search        61     820.1K      71.2K      91%     380ms
  symbol_lookup          38     301.0K      22.4K      93%      42ms
```

Además, cada respuesta que la IA recibe lleva su propio pie, con el coste primero y el rival nombrado: `[42 ms · ~180 tokens · −91% vs leer el archivo completo]`.

`--worst` es la vista honesta: ordena por tokens realmente gastados, que es lo que un porcentaje alto puede esconder.

```
  COSTO     HERRAMIENTA          AHORRO     HACE      MS  DETALLE
  985       rtk_grep                -1%      26s      18  baseline
  89        rtk_find                14%      26s       9  . -name *.rs
```

**Dónde vive y cómo apagarlo.** Un JSONL append-only en `~/.local/share/rtk-index/telemetry.jsonl` (respeta `$XDG_DATA_HOME` y `$RTK_INDEX_DATA_DIR`), compartido entre proyectos y seguro con varios servidores MCP a la vez; rota a los 8 MB. Es 100 % local: no sale nada de tu máquina. Se desactiva con `RTK_INDEX_TELEMETRY=0` — eso también evita los procesos extra que miden el baseline de `rtk_grep`/`rtk_find`/`get_minified_diff`.

> Los tokens son una **estimación conservadora** (1 token ≈ 4 caracteres), no la cuenta exacta del tokenizador del modelo. El porcentaje de ahorro sí es exacto: es invariante a esa constante.

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
      "args": ["serve", "--root", "/ruta/a/tu/proyecto"],
      "alwaysLoad": true
    }
  }
}
```
> `"alwaysLoad": true` mantiene las herramientas siempre cargadas (no diferidas por *tool-search*). El `init` ya lo escribe por ti.

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

## ⚡ Rendimiento: qué cuesta una llamada

Medido en **release** sobre un monorepo de 3679 archivos, con `RTK_INDEX_TRACE=1` (las trazas van a stderr; no cuestan tokens):

| Fase | Antes | Ahora |
| :--- | ---: | ---: |
| `sync` (detectar cambios) | 8388 ms | **26 ms** |
| carga del modelo ONNX | 535 ms **por llamada** | 535 ms **una vez por sesión** |
| BM25 | 2 ms | 0 ms |
| embed de la consulta | 3 ms | 2 ms |
| cargar vectores + coseno | 41 ms **por llamada** | 0 ms (28 ms una vez) |
| warm-up del índice semántico | 8-11 s **dentro de la búsqueda** | 0 (ocurre en tiempo ocioso) |
| **total, búsqueda en caliente** | **~9 s** | **~30-50 ms** |

Cinco cambios lo explican:

1. **Workspace cacheado por proceso.** Antes cada llamada reabría el índice y recargaba el modelo ONNX entero.
2. **`sync` con una sola consulta y walk paralelo.** Antes preguntaba a SQLite archivo por archivo (~30 µs × miles) y leía y hasheaba todo el repo; ahora carga el estado de una vez, recorre el árbol en paralelo y compara `mtime` — solo lee lo que cambió.
3. **Vectores en RAM.** Antes se deserializaban ~30 MB desde SQLite en cada búsqueda. Ahora se cargan una vez y se actualizan en sitio cuando se re-vectoriza un archivo.
4. **Warm-up en tiempo ocioso.** Un hilo lee stdin y el principal vectoriza el backlog *entre* peticiones. Ninguna búsqueda paga la deuda de vectorización de otro archivo; cuando llega tu consulta, el trabajo ya está hecho.
5. **Presupuesto por contexto.** Dentro de una búsqueda se embeben como mucho 32 símbolos (lo que acabas de editar); el resto es trabajo de fondo.
6. **Precalentado.** En el primer rato ocioso —normalmente antes de que pidas nada— se carga el modelo y los vectores a RAM, así ni la primera búsqueda de la sesión paga esos ~535 ms.

Si el índice aún tiene backlog, una búsqueda cuesta ~350 ms en vez de ~40; el warm-up ocioso lo agota solo, y `rtk-index init .` lo hace de golpe.

---

## ⚙️ Arquitectura Interna (src/index)

La verdadera magia ocurre en la indexación de contexto local, que divide el código en dos almacenes sincronizados (`.rtk-index/`):
*   **Base de datos Léxica y Relacional (SQLite):** Hashes (Blake3), estado de archivos, y anclas exactas de los símbolos extraídos por Tree-sitter.
*   **Motor de Texto y Vectores (Tantivy + Fastembed):** Índices semánticos limitados inteligentemente a `MAX_EMBED_PER_CALL = 800` para garantizar que la primera ejecución de tu IA sobre un monorepo no cuelgue la máquina, sino que se caliente (warm-up) de manera incremental.
