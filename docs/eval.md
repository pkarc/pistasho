# Mediciones de pistasho

Registro fechado de rendimiento y precisión. Todo se mide con el binario `--release`.
Máquina de las mediciones: 16 hilos. Índice: 34 878 entradas (tldr `en` + `es`, plataformas `common` + `linux`).

## Fase 1 · línea base del buscador difuso (2026-11-19)

### Precisión (`pistasho eval`, 40 preguntas en `eval/questions.toml`)

| Variante | top-1 | top-3 |
|---|---|---|
| Línea base de la fase 1 (se busca en todo `alt`) | 18% | 25% |
| `alt` recortado a 150 caracteres (**actual**) | **18%** | **28%** |
| `alt` recortado a 60 caracteres | 15% | 20% |
| sin `alt` | 12% | 20% |

**Línea base de la fase 1: top-1 18%, top-3 28%.** Es baja a propósito: el buscador difuso exige que cada palabra coincida, así que con frases completas en español casi no encuentra nada. Es el número que la fase 2 (BM25 + embeddings) tiene que superar claramente. Son 40 preguntas, así que un punto de diferencia es ruido; la meta sigue siendo ~100 preguntas con comandos reales del usuario.

### Rendimiento (`pistasho query --timing`)

Apertura: lo que se espera desde que se abre el buscador hasta ver resultados.

| Paso | Antes | Ahora |
|---|---|---|
| Cargar `index.json` | 49 ms | 50 ms |
| Construir el buscador | 8 ms | 17 ms |
| **Total** | **57 ms** | **67 ms** |

La meta era menos de ~150 ms: se cumple, así que no hace falta cambiar el formato del índice (la carga de JSON son 50 ms). El buscador cuesta 9 ms más porque ahora recorta `alt`.

Cada tecla (un `rank` por prefijo escrito, el mismo que hace la interfaz, con tope de 300 resultados):

| Consulta | Antes | Ahora |
|---|---|---|
| `tar czf archive directory` (25 teclas) | media 175 ms, máx. 206 ms | media 28 ms, máx. 42 ms |
| `comprimir carpeta en zip` (24 teclas) | — | media 22 ms, máx. 46 ms |
| `ver puertos escuchando` (22 teclas) | — | media 20 ms, máx. 25 ms |

Qué lo bajó (de ~175 a ~25 ms):
1. **Reducir candidatos al escribir.** Si la consulta nueva extiende la anterior, solo se vuelven a puntuar las entradas que ya coincidían (una pila de prefijos, que también cubre borrar con Backspace). No se usa con los operadores `! ' ^ $ \` porque `!ta` excluye más que `!tar`. Un test comprueba que escribir tecla a tecla da exactamente el mismo orden que buscar desde cero.
2. **Repartir el cálculo en hilos** (hasta 8) cuando hay más de 4096 candidatos.
3. **Recortar `alt` a 150 caracteres** al construir el texto de búsqueda. La cola de `alt` es el resumen de la página, repetido en cada ejemplo de la página, y duplicaba el coste sin mejorar la precisión.

Límites conocidos:
- La mejora de los hilos depende de los núcleos: con menos de 16 las teclas con muchos candidatos serán más lentas. El caso peor medido aquí es ~46 ms.
- Con pocas letras (`t`, `ta`) casi todo coincide, así que la primera tecla cuesta ~20–25 ms aunque se reparta en hilos. La fase 2 (FTS5 + BM25) cambia este perfil de coste.

## Tarea 2.1 · índice SQLite + FTS5 (2026-11-19)

| Medida | Antes (`index.json`) | Ahora (`pistasho.db`) |
|---|---|---|
| Cargar el índice | ~50 ms | ~45 ms (con 2216 comandos de historial incluidos) |
| Apertura total (carga + buscador) | ~67 ms | 62–66 ms |
| `eval` top-1 / top-3 | 18% / 28% | 18% / 28% (sin cambios, como se esperaba: el buscador difuso es el mismo) |
| Tamaño en disco | 10.9 MB | 15.9 MB (incluye el índice de texto completo) |

El ranking FTS5 / BM25 aún no se mide con `eval`: lo usa `pistasho ask`, que es la tarea 2.2. Primer vistazo sobre la base real (consultas armadas con `fts_query`): `compresión` → `compress`, `lzop`; `comprimir carpeta` → `carp`; `puertos escuchando` → `rcat`, `mysqld`, `socat` (ruido). Con palabras vacías y pesos por columna esto debería mejorar; se mide en 2.2.

## Tarea 2.2 · `pistasho ask` con BM25 (2026-11-19)

**Conjunto de preguntas.** Con 40 preguntas una pregunta vale 2.5 puntos y las comparaciones eran ruido (una variante parecía mejorar el top-3 de 38% a 30% y en realidad era al revés). Se amplió `eval/questions.toml` a **101** con 61 tareas genéricas de Linux. Las 40 originales se siguen midiendo aparte como referencia de la línea base de la fase 1. Ninguna de las 61 nuevas son comandos "reales del usuario" todavía (tarea 1.4, continua).

### Resultado

| Conjunto | Difuso (fase 1) | BM25 (`ask`) |
|---|---|---|
| 40 originales (línea base de la 1.4) | 18% / 28% | **28% / 45%** |
| 101 preguntas | 28% / 41% | **37% / 51%** |

(top-1 / top-3). Sobre la línea base de la fase 1 son +10 puntos de top-1 y +17 de top-3. `ask` tarda lo que un proceso en arrancar (≈ 0 s medibles, sin cargar todas las entradas en memoria).

### Qué se probó y qué decidió

Todo con `pistasho eval --mode bm25` (y `--weights` para los pesos), en rejillas de 84, 324 y 108 configuraciones:

| Decisión | Resultado |
|---|---|
| Prefijo también sobre el nombre de la herramienta | **Sí.** Restringir la herramienta a coincidencia exacta parecía mejor con 40 preguntas y fue peor con 101 (top-3 50% → 39%). |
| Raíz de las palabras: dejar enteras las de hasta 6 letras, recortar 3 a las más largas | **Elegido.** Mínimos más cortos (4–5) fueron peores: los prefijos cortos coinciden con herramientas sin relación (`carpeta` → `carp`, `partición` → `particle`). |
| Pesos de columnas (`description`, `alt`, `tool`, `cmd_words`) | **3, 2, 4, 2.** La superficie es plana: la mediana de la rejilla es 83 (top-1 + top-3) y el mejor punto 88. No tiene sentido buscar decimales. |
| Indexar las palabras del comando sin `{{marcadores}}` (`cmd_words`) | Necesario para el historial: `ssh root@host` solo se encontraría por `ssh`, nunca por `root`. |
| Unir las traducciones por la **forma** del comando (sin los nombres de los marcadores) | Quitó ~4000 ejemplos duplicados (34 878 → 30 932 de tldr) y deja el texto en español en `alt` en vez de en una entrada aparte. |
| Palabras vacías (español e inglés) | Sin medir por separado; evitan que "qué es eso" encuentre algo. |

### Qué falla todavía

Los fallos restantes son sobre todo la **distancia semántica español → descripción en inglés** que BM25 no puede salvar: `montar una partición` (→ `mount`), `formatear una usb en ext4` (→ `mkfs`), `contar líneas` (→ `wc`), `ver logs de un servicio` (→ `journalctl`). Eso es lo que atacan los embeddings de la 2.9. El segundo grupo es ruido de palabras muy frecuentes (`archivos`, `texto`) que coinciden con descripciones cortas (`patool`). Los pesos por columna no lo arreglan; son candidatos a probar con los embeddings.
