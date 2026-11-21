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

## Tarea 2.9a · Embeddings multilingües: elección del modelo (2026-11-20)

Medido antes de construir nada, con `scripts/embeddings_experiment.py` (Python + fastembed, solo estudio; no es dependencia del proyecto) sobre las 101 preguntas y las entradas de tldr y cheats de la base real (el historial queda fuera). Se compara BM25 (lo que hace `ask` hoy), solo coseno y la fusión por rangos (RRF, k=60), con el texto de cada entrada `herramienta. descripción. alt[:150]`.

### Calidad (top-1 / top-3, sobre 101 preguntas)

| Modelo | BM25 | Solo embeddings | Híbrido (RRF) | Índice completo |
|---|---|---|---|---|
| **multilingual-e5-small (int8)** | 35 / 50 | 44 / 54 | **50 / 58** | ≈ 6.5 min |
| paraphrase-multilingual-MiniLM-L12 | 35 / 50 | 23 / 36 | 39 / 58 | ≈ 9 min |
| potion-multilingual-128M (estático) | 35 / 50 | — | 40 / 53 | rápido |

(Números absolutos de aciertos sobre 101; el 35/50 de BM25 es el mismo `pistasho eval --mode bm25` con la base de esta máquina.) e5-small gana con claridad y es el único donde solo-embeddings ya supera a BM25. Sube el top-1 en 15 aciertos.

### Coste (máquina del usuario, release)

| Qué | Medido |
|---|---|
| Modelo en disco | 118 MB (`model_quantized.onnx` de Xenova) + tokenizador de 17 MB |
| Binario con `ort` (onnxruntime estático) | 6.4 → ≈ 38 MB |
| Vector de una pregunta (calentado) | 3–11 ms |
| Memoria con el modelo cargado | ≈ 570–790 MB de RSS |
| Cargar el tokenizador (`from_file`) | ≈ 350 ms |
| Crear la sesión ONNX | ≈ 130 ms con la optimización desactivada, ≈ 190 ms por defecto, ≈ 350 ms con `Level1` |
| Tokenizador + sesión en paralelo | ≈ 0.4–0.55 s (antes ≈ 1 s en serie con fastembed) |

### Decisión

- **Modelo:** `multilingual-e5-small` int8 (prefijos `query: ` / `passage: `, mean pooling, normalizado).
- **Vía:** `ort` directo (con `tokenizers`), no `fastembed`: así se fija el archivo exacto, se controla la optimización y la carga en paralelo.
- **Descarga:** en `update`, con sha256 fijado; nunca incluido en el binario.
- **Latencia:** cargar el modelo en cada proceso (≈ 0.4–0.5 s) rompe el criterio de ~150 ms si se hace después de la pregunta. Se **precarga en un hilo mientras el usuario escribe** la pregunta (`ask --prompt`), y queda `search.embeddings = false` para apagarlo. Un daemon se deja para la fase 3, cuando haga falta por Laya.
- Pendiente de 2.9b: velocidad de indexado en Rust por lotes, y coste de cargar/multiplicar los ~30k vectores desde SQLite (≈ 47 MB en f32; evaluar i8).

## Tarea 2.9b · Embeddings en `ask` (2026-11-20)

Implementado como estaba decidido en 2.9a: `ort` directo, vectores `i8` en SQLite (clave = hash del texto, así `update` solo embebe lo nuevo), RRF k=60 entre BM25 (con sus boost) y los 200 vectores más cercanos.

| `pistasho eval` (101 preguntas, tldr sin cheats ni historial) | top-1 | top-3 |
|---|---|---|
| `--mode fuzzy` (fase 1) | 28% | 41% |
| `--mode bm25` | 35% | 50% |
| `--mode hybrid` | **46%** | **58%** |

- El estudio en Python daba 50 / 58 en el híbrido. La diferencia de top-1 (4 preguntas) viene de la cuantización a i8, del tope de 128 tokens y de que aquel BM25 salía del `ask` instalado con cheats e historial reales; no se persiguió, porque el criterio ("`hybrid` supera a `bm25`") se cumple con holgura.
- Subir el pool de vecinos de 200 a 400 no cambió nada.
- **Costes medidos:** `update` completo ≈ 5 min 8 s (30 956 vectores, lotes de 32 ordenados por largo; 3 núcleos, 720 MB de RAM); la base queda en ≈ 29 MB (unos 12 MB son vectores); el binario release es de 32 MB; `ask` suelto ≈ 0.5 s y 475 MB de RAM (casi todo cargar el modelo). Con el modelo cargado, calcular la pregunta, leer los vectores y fusionar tarda ~30 ms (la evaluación de 101 preguntas, con carga única, tarda 3 s).
- **Criterio de latencia (~150 ms): no se cumple en un `ask` suelto.** La mitigación es la precarga mientras el usuario escribe (`embed::prefetch`), que debería esconder la carga si la pregunta se escribe en más de ~0.5 s. Falta medirlo dentro del widget con preguntas cortas; si no basta, el siguiente paso es un daemon (fase 3) o apagarlo con `search.embeddings = false`.
- Un hallazgo aparte: por significado, "montar una partición" devolvía `<n>` (la tecla de `fdisk` para crear una partición, que tldr lista como ejemplo). `ask` ya no sugiere comandos que empiezan por `<`.

## Tarea 2.10 · Cierre de la fase 2 (2026-11-21)

`pistasho eval` sobre las 101 preguntas, base recién construida (sin cheats ni historial):

| Modo | top-1 | top-3 |
|---|---|---|
| fuzzy (línea base de la fase 1) | 28% | 41% |
| bm25 | 35% | 50% |
| **hybrid (lo que hace `ask`)** | **46%** | **61%** |

(El top-3 del híbrido subió de 58% a 61% con el filtro de comandos que empiezan por `<`.) Sobre la fase 1 son +18 puntos de top-1 y +20 de top-3.

**Latencia dentro del widget** (zsh real en tmux, modelo en disco, Enter tras escribir `comprimir una carpeta en zip`; el mensaje `⚡ … N ms` mide lo que tarda en resolver tras el Enter):

| Tiempo entre `Ctrl-Space` y Enter | Resolver |
|---|---|
| ≥ 0.5 s (escribir normal) | **41 ms** (la precarga del modelo ya terminó) |
| casi 0 (Enter inmediato) | 440–480 ms (hay que esperar la carga) |

La mitigación por precarga funciona: el criterio de ~150 ms se cumple en el uso normal. Queda el peor caso, y la calidad absoluta sigue lejos del 85–90% de top-1 que justificaría descartar la fase 3 (criterio del CLAUDE.md).

