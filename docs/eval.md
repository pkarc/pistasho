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
