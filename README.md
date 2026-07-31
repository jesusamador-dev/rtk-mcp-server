# RTK MCP Server — proyecto cancelado

> **No uses esto.** Un benchmark ciego con dos agentes en paralelo demostró que
> para el trabajo real —un agente libre de componer comandos— `grep`, `find` y
> pipes salen **más baratos y encuentran más** que estas herramientas. El
> proyecto se archiva. Lo que sigue es el resultado y por qué, que es lo único
> que aquí vale la pena conservar.

## El resultado

Dos agentes auditaron el mismo módulo (`src/billing`, 164 archivos, 14 564
líneas) a ciegas, en paralelo, con la misma consigna. Uno solo podía usar las
herramientas MCP de este repo; el otro solo Bash.

| | A — solo rtk-index | B — solo Bash |
| :--- | ---: | ---: |
| Llamadas a herramientas | 13 | 9 |
| Tokens de salida de las herramientas | ~9 998 | ~5 789 |
| **Tokens totales del agente** | **72 722** | **63 750** |
| Duración | 111 s | 135 s |
| Preguntas sin responder | 0 | 0 |
| **Hallazgos exclusivos** | **0** | **3** |

Bash ganó en coste (−12,3 %) y en calidad. Los tres hallazgos que solo encontró
el camino B fueron: seis `as unknown as` concentrados en un adaptador, la
ausencia *verificada* de `@ts-ignore`/`eslint-disable` (A nunca comprobó si el
`any` estaba silenciado por otra vía), y la razón por la que un command carece
de `idempotencyKey` — A vio la ausencia, B explicó por qué era intencional.

## Por qué las mediciones anteriores decían lo contrario

Las medidas previas de este repo (hasta −71 % de tokens) forzaban una
equivalencia 1:1: una llamada MCP por cada `grep`. **Un agente libre no trabaja
así.** B compuso `grep -c | sort -rn | head` en un solo comando y rankeó 164
archivos de una vez; A necesitó tres `rtk_grep --count` separados (~2 955
tokens) para lo mismo. La ventaja de una herramienta atómica se evapora en
cuanto el operador puede encadenar pipes.

Es el sesgo central: se midió la herramienta contra una versión artificialmente
restringida de su alternativa.

## Lo que sí quedó demostrado, en contra

- **La llamada más cara del experimento fue `codebase_search`**: 912 ms y 3 070
  tokens, el 31 % del presupuesto de herramientas de A, para responder algo que
  `grep` contestó mejor.
- **Abaratar puede empeorar la respuesta.** El modo conteo de `rtk_grep`
  devuelve `archivo: N` sin la línea. Salió barato e indujo al agente A a dar
  por buenos 8 de 11 handlers apoyándose en coincidencias de una palabra, sin
  leer el contexto. Una herramienta barata que te hace afirmar lo que no
  comprobaste es peor que una cara.
- **`file_outline`** gana 17× contra leer el archivo entero, pero contra un
  `grep -n` de firmas con `awk` la ventaja desaparece.
- **`symbol_lookup` nunca llegó a medirse**: ningún agente lo eligió por su
  cuenta.

## Lo único que sobrevive

`codebase_search` para "no sé dónde está esto" en un repo que no conoces:
recuperación híbrida (BM25 + semántica) que devuelve fragmentos con su ancla
`path:líneas` a partir de una descripción. Es el caso que `grep` no cubre,
porque exige conocer un nombre o un patrón. Todo lo demás —`rtk_grep`,
`rtk_find`, `file_outline`, `bulk_read_files`, `get_minified_diff`— lo hace
Bash igual o mejor.

## Lo que sí funcionó, como notas de ingeniería

El trabajo de optimización fue real, aunque resultara irrelevante frente a la
alternativa correcta. Sobre un monorepo de 3679 archivos, una búsqueda pasó de
~9 s a ~40 ms:

| Fase | Antes | Después |
| :--- | ---: | ---: |
| `sync` (detectar cambios) | 8388 ms | 26 ms |
| carga del modelo ONNX | 535 ms **por llamada** | una vez por sesión, en ocio |
| cargar vectores + coseno | 41 ms por llamada | 0 ms (28 ms una vez) |
| warm-up semántico | 8-11 s **dentro de la búsqueda** | fuera de la ruta crítica |

Lo que lo explica: cachear el workspace por proceso, cargar el estado del índice
en una consulta en vez de una por archivo, comparar `mtime` antes de leer y
hashear, mantener los vectores en RAM, y mover el calentamiento a los ratos
ociosos entre peticiones. Y la lección más cara: **medir la latencia
end-to-end**, no la interna de la herramienta — el pie de la respuesta decía
9 ms mientras el cliente esperaba 657.

Dos regresiones llegaron a producción durante ese trabajo, ambas por falta de
pruebas: un `rtk_grep` que recorría un universo distinto al de `rtk_find` (56
archivos frente a 16) y, peor, un error de E/S transitorio al recorrer el árbol
que hacía purgar del índice archivos que solo "no se habían visto" — 2742
vectorizaciones perdidas. Las 38 pruebas de `cargo test` salieron de ahí, no de
buscar cobertura.

---

<details>
<summary>Documentación original (histórica)</summary>

Servidor MCP en Rust con índice de código local: símbolos vía tree-sitter en
SQLite, texto completo con tantivy (BM25), y embeddings locales con fastembed
(ONNX, modelo multilingüe E5-small) fusionados por Reciprocal Rank Fusion.
Indexa también specs markdown de OpenSpec/Spec Kit por encabezado.

```bash
cargo install --git https://github.com/jesusamador-dev/rtk-mcp-server --bin rtk-index
rtk-index init .     # indexa y registra el servidor en .mcp.json
rtk-index check      # verifica el entorno
rtk-index gain       # telemetría de ahorro (ver la advertencia de abajo)
rtk-index update     # actualiza el binario
```

Sobre `rtk-index gain`: mide el ahorro contra un baseline por herramienta (leer
el archivo completo, la salida cruda de `grep`/`find`). Ese contrafactual **no
es el de un operador competente**, que habría escrito un `grep` dirigido. El
propio benchmark de arriba es la refutación de esos porcentajes.

</details>
