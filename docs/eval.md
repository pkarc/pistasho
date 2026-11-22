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

## Tarea 3.0 · Rerankers y techo de la recuperación (2026-11-21)

Antes de construir la fase 3 se midió si una capa de decisión tendría de dónde ganar. Dos bancos: las 101 preguntas originales y **287 sintéticas** (`eval/questions-synth.toml`, generadas, con las herramientas aceptadas listadas a mano). Una herramienta acierta también con sus subcomandos (`ip-address` vale por `ip`, `git-diff` por `git`): con la regla estricta de antes varios fallos eran falsos.

### Rerankers sin ajustar (`scripts/rerank_experiment.py`, CPU, sobre los 20 mejores candidatos)

| | originales (top-1 / top-3) | sintéticas | Latencia |
|---|---|---|---|
| orden del híbrido (de entonces) | 46 / 61 | 33 / 48 | n/a |
| mmarco-mMiniLMv2 (118M) | 42 / 55 | 25 / 46 | 190 ms |
| horizon-small (141M) | 44 / 61 | 32 / 50 | 365 ms |
| bge-reranker-v2-m3 (568M) | 48 / 60 | 31 / 51 | 2.2 s |

Ninguno mejora de forma útil y todos cuestan mucho más que el presupuesto (~150 ms). Con la regla estricta el recall@10 del híbrido era 69% / 60%: el techo estaba en la recuperación, no en el orden. Laya sin ajustar no se probó: su ficha reporta ≈0.35 de acierto frente a 0.32 del azar y dice que es "una base para especializar".

### Subir el techo (top-1 / top-3 / recall@10, regla de subcomandos, híbrido)

| Paso | originales | sintéticas |
|---|---|---|
| RRF 1:1, k=60 (punto de partida) | 50 / 67 / 74 | 45 / 61 / 72 |
| + pesos 1:2, k=10 | 50 / 67 / 82 | 48 / 62 / 77 |
| + un ejemplo de tldr por herramienta (`per_tool = 1`) | 50 / 73 / 84 | 48 / 64 / 81 |
| + sinónimos (`data/synonyms.toml`) | **51 / 74 / 85** | **47 / 66 / 84** |

- Rejilla de fusión (1:0, 1:1, 1:2, 1:3, 1:5, 0:1 y k = 5, 10, 20, 60): la diferencia entre las buenas es de 1–2 puntos; se eligió 1:2 con k=10 por el mejor recall en los dos bancos. Solo meaning (0:1) queda en 47/62/80, así que BM25 aporta.
- `per_tool`: 1 gana 6 puntos de top-3; 2 y 3 casi no cambian nada.
- Sinónimos: lo que más sube a **BM25 solo** (originales 41/60/74 → 48/69/83; sintéticas 35/54/69 → 37/61/76), es decir, a quien no baje el modelo. En el híbrido suma 1–3 puntos.
- Un hallazgo: `fold` no quitaba la `ñ`, y los sinónimos de `contraseña`, `tamaño` o `dueño` nunca coincidían (lo vio un test).
- **Riesgo de sobreajuste, pequeño pero real:** los pesos, `per_tool` y la tabla se eligieron mirando ambos bancos, y el banco sintético lo escribí yo conociendo los fallos de la primera pasada. Son pocos parámetros y la tabla es vocabulario general, pero la cifra honesta de generalización llegará con preguntas reales del usuario (tarea 1.4).

### Lo que sigue fallando

Comandos cotidianos con descripciones muy genéricas (`rm`, `mkdir`, `pwd`, `ping`, `ssh-keygen`, `hostname`) pierden contra herramientas raras que casualmente comparten una palabra (`gdown`, `b2`, `nxc-ssh`). Es un problema de modelo de significado (e5-small es pequeño) o de prioridad de herramientas comunes, no de orden entre los 10 primeros.

## Tarea 4.3 · Cuándo preguntar al LLM (2026-11-22)

`pistasho eval --confidence` imprime, por umbral de puntaje fusionado del mejor candidato, qué parte de las preguntas se mandaría al LLM y cuánto acierta el candidato local en cada grupo (híbrido, regla de subcomandos):

| Umbral | Se pregunta (originales / sintéticas) | Local acierta, lo que se queda | Local acierta, lo que se pregunta |
|---|---|---|---|
| 0.18 | 0% / 0% | 51% / 47% | n/a |
| **0.20** | **9% / 15%** | 55% / 50% | **11% / 26%** |
| 0.22 | 21% / 29% | 61% / 54% | 14% / 28% |
| 0.25 | 48% / 57% | 62% / 57% | 40% / 39% |

El puntaje fusionado separa mal: aun lo "seguro" acierta solo 55–62%, porque muchos "fallos" son herramientas válidas que el banco no acepta (`fclones` por `fdupes`, `halt` por `shutdown`). Pero el extremo bajo sí es fiable: debajo de 0.20 el candidato local casi nunca es el correcto (11–26%). Se eligió **0.20** (se pregunta en 1 de cada 7–11 preguntas) y es configurable (`llm.threshold`). Con 0.22 se preguntaría a casi un cuarto de las preguntas, con más latencia y coste.

**Lo que no se midió:** cuánto acierta el LLM en ese grupo (cada medición gasta una clave de pago; las pruebas usan un servidor falso). Queda para ver en uso real, con `pistasho cache list` para revisar lo que respondió.

## Tarea 3.6 · Decisión sobre la fase 3 (2026-11-22)

La capa de decisión (Laya / Jev) no se construye. Resumen de lo medido y averiguado:

| Qué | Dato |
|---|---|
| Rerankers generales sobre los 20 mejores (3.0) | −3 a +3 puntos de top-1, 190 ms – 2.2 s en CPU: no compensan |
| Techo de la recuperación (recall@10) | 85% / 84% (originales / sintéticas): hay de dónde reordenar, pero no con modelos sin ajustar |
| Cuándo llamar al LLM (4.3) | El puntaje fusionado aísla un grupo (9–15% de las preguntas) donde el candidato local acierta solo 11–26%: ya cubre "¿hace falta el LLM?" |
| Laya sin ajustar | ≈0.35 de acierto en su propia ficha (azar 0.32); ajustarlo pide datos que no hay y un daemon de 0.5–1.7 GB |
| Jev alojado | Acceso anticipado con lista de espera; documentación pública de terceros, incompleta y con hosts distintos; sin clave no se puede medir |

Y la decisión del usuario de no depender de modelos locales pesados descarta Laya local, el daemon y el servidor en el Proxmox. Queda abierta una puerta barata: si hay clave de Jev, medir `choice` sobre los 10 mejores con `eval`.

## Fase 3 reabierta · Jev (2026-11-22)

Con una clave de Jev (`TYPESAFE_API_KEY`) se midió antes de construir (`scripts/jev_experiment.py`; se envían solo preguntas de `eval` y candidatos de tldr, sin historial ni atajos). El formato real de la API (`POST /v1/systemone`, `Authorization: Bearer`, `state` + `questions` con `criteria`; respuesta con `choice`, `confidence`, `probabilities` o `noul`) se verificó con una llamada mínima: la documentación de terceros era incompleta.

### `choice` sobre los mejores candidatos locales + "none"

| | originales (101) top-1 / top-3 | sintéticas (287) top-1 / top-3 |
|---|---|---|
| orden local (híbrido) | 52 / 74 | 48 / 67 |
| Jev sobre 10 candidatos (experimento) | 79 / 85 | 73 / 83 |
| Jev sobre 20 candidatos (experimento) | 82 / 92 | 76 / 89 |
| Jev sobre 30 candidatos (experimento) | 82 / 92 | 75 / 90 |
| **implementación real, 20 candidatos (`eval --decide`)** | **83 / 92** | **76 / 88** |

Con 10 candidatos el tope era el recall de la búsqueda local (85%: lo que no trae, Jev no lo puede elegir); con 20 sube a 93% / 91% y con 30 casi no cambia, así que se envían 20 (`decide::MAX_OPTIONS`) a la misma latencia. En modo `--decide` la columna `recall@10` de `eval` mide el top-10 después del reordenamiento, por eso sube.

- Latencia de la llamada: mediana ≈ 200 ms, p95 ≈ 270 ms.
- **Calibración:** cuando Jev elige un candidato con confianza ≥ 0.95 acierta el 97% (de 34 respuestas en las originales y de 71 en las sintéticas); entre 0.8 y 0.95, 77–81%; bajo 0.8, 70–71%.
- **"none"** (13–16% de las preguntas): el candidato local acierta solo el 20–23% ahí, frente a 53–57% cuando Jev elige uno. Es la señal para llamar al LLM: en el grupo de puntaje local dudoso (< 0.20), Jev lo marca "none" en 42–56% de los casos, y cuando elige uno acierta 58–67% frente a 22–29% del orden local.
- Una política "confiar en Jev solo con confianza ≥ t" rinde menos que usar siempre su orden (t = 0: 75% / 71%; t = 0.95: 61% / 56%), así que se usa siempre su orden.
- Con Jev, la parte que queda es el recall (85%): lo que la búsqueda local no trae, el modelo no lo puede elegir.

### `noul` "¿es destructivo?" contra las reglas fijas (63 comandos etiquetados a mano, 29 destructivos)

| | destructivos que caza | falsas alarmas |
|---|---|---|
| reglas fijas | 17 / 29 | 0 |
| Jev solo, noul ≥ 0.5 | 18 / 29 | 0 |
| reglas + Jev (cortes 0.65 rojo, 0.3 aviso si las reglas dicen ok) | **21 / 29** | **0** |

Los que Jev suma con el corte final: `find / -name '*.log' -delete`, `truncate -s 0 …/postgresql/…`, `rm -rf /var/lib/docker`, `tar czf … / --remove-files` (`docker system prune -af --volumes`, 0.53, queda en aviso). Ninguno de los dos caza `iptables -F && iptables -P INPUT DROP` (noul 0.08) ni la bomba fork (0.12): las reglas fijas siguen siendo la red principal.

**Revisión con plantillas reales.** Lo que se envía en producción es la plantilla de tldr con sus `{{marcadores}}`, no un comando relleno, y los puntajes cambian: sobre 20 plantillas, las inofensivas (`ls -la {{path}}`, `tar czf …`, `ssh {{user}}@{{host}}`, `mkdir -p …`) sacaron ≤ 0.20; `dd … of={{/dev/sdX}}` 0.93, `mkfs.ext4 {{/dev/sdXN}}` 0.91, `DROP TABLE {{table}}` 0.70, `shred` 0.67; y un simple `rm {{path/to/file}}` 0.55, `rm -rf {{path/to/directory}}` 0.59, `docker system prune` 0.51, `git reset --hard {{commit}}` 0.41, `git push --force` 0.31 y `chmod -R 777 {{dir}}` 0.04 (este último lo cubren las reglas sobre `/`). Con el corte inicial de 0.5 cada `rm` habría abierto la caja roja: por eso el corte final es 0.65.

**Margen fino:** muestras pequeñas (63 comandos, 20 plantillas) y etiquetas mías. El modelo solo puede subir el nivel, nunca bajarlo, y una caja roja de más cuesta un Enter.

### Costes

~200 ms por llamada con 10, 20 o 30 candidatos (una para elegir y otra para el riesgo, en serie) y $0.042 por millón de tokens de entrada (≈ 350 tokens con 2 opciones, ~1000 con 20): evaluar los 388 casos costó céntimos. Sin caché de veredictos.

## Fase 3 · Laya local sin ajustar (2026-11-22)

El usuario aclaró (2026-11-22) que Proxmox y Ollama siguen descartados, pero que Laya local con un daemon tiene sentido **si los recursos son una fracción manejable**; fijó el presupuesto en **hasta ~1 GB de RAM residente, con apagado solo tras un rato sin uso**, y pidió medir primero (`scripts/laya_experiment.py`: el mismo experimento `choice` que con Jev, 20 candidatos + "none", pero con `laya-multilingual` en esta máquina, 16 hilos, PyTorch CPU fp32).

| | originales (101) top-1 / top-3 | sintéticas (287) top-1 / top-3 |
|---|---|---|
| orden local (híbrido) | 53 / 76 | 47 / 65 |
| **Laya multilingüe sin ajustar**, reordenando | **22 / 51** | **21 / 46** |
| Jev, 20 candidatos (referencia) | 83 / 92 | 76 / 88 |

- **No sirve sin ajustar:** peor que el orden local y casi al azar entre 20 candidatos; dice "none" en el 24–26% de las preguntas sin que eso signifique nada (el candidato local acierta 33–49% ahí). Su propia ficha ya lo decía ("una base para especializar, no un motor de decisión listo para usar").
- **Recursos medidos (PyTorch fp32):** 2279 MB de RAM máxima (incluye la carga del checkpoint de 647 MB), 5.6 s de carga con el caché de Hugging Face caliente (24.7 s la primera vez), ~190 ms de mediana por llamada con 20 candidatos (p95 ~240 ms). **Supera el presupuesto de 1 GB**; la variante ONNX int8 (`laya[onnx]`, ~0.5 GB según la ficha) no se midió.
- **Lo que falta para que sea una opción:** ajustarlo con datos propios (3.2 y 3.3), exportarlo a ONNX int8 y medir que cabe en el presupuesto y en ~200 ms, y escribir el daemon. El acierto ajustado es **desconocido**; Jev ya da 76–83% hoy.

