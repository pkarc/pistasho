# Entrenar el modelo de decisión local (Laya)

> Runbook de la fase 3 local (tareas 3.2–3.4 del plan). Va solo en español, como el resto de las notas
> de proyecto. Los scripts están en `training/` y se comentan en inglés.
>
> **Estado (2026-11-23): cerrado como «agotado».** Se ejecutó el plan en la RTX 5090 ([`training-5090.md`](training-5090.md))
> y ninguna variante pasa la puerta de decisión; resultados en [`training-experiments.md`](training-experiments.md) y
> [`eval.md`](eval.md). No se integra Laya ni el daemon; este runbook queda para reabrirlo.

**Objetivo (decisión del usuario, 2026-11-22):** que pistasho sea lo más eficiente posible **sin depender de
terceros**. Jev (y Anthropic) quedan como opción del usuario; el modelo local es el camino principal.
Presupuesto de recursos en el equipo del usuario: **hasta ~1 GB de RAM residente**, con un daemon que se apaga
solo. Los recursos grandes son para el proceso intermedio (ajustar), no para el uso: el entrenamiento se hace
una vez en una RTX 5090 y lo que se entrega es un ONNX pequeño que corre en CPU.

## Cómo se reparte el trabajo

| Dónde | Qué | Cómo |
|---|---|---|
| Máquina con pistasho y su base | Generar los datos (tardan ~40 min por las búsquedas) | `training/run.sh data` y `training/run.sh pack` |
| Máquina con la 5090 | Entrenar variantes, evaluarlas, exportar a ONNX | `training/run.sh setup`, `unpack`, `baseline`, `grid`, `export` |

La máquina de la GPU **no necesita pistasho ni su base**: recibe un paquete (`bundle.tar.gz`: los JSONL de
entrenamiento y validación y `eval-cands.json`, los candidatos congelados de las 388 preguntas de `eval`).
Necesita Python 3.10+, ~10 GB de disco y acceso a Hugging Face para el checkpoint de Laya (650 MB).

## En la 5090, paso a paso

```sh
git clone https://github.com/pkarc/pistasho && cd pistasho      # o copiar training/ y common
training/run.sh setup                       # venv con torch para CUDA 12.8 (Blackwell, sm_120) y laya
training/run.sh unpack /ruta/bundle.tar.gz
training/run.sh baseline                    # Laya sin tocar: la referencia (22–25% de top-1)
nohup training/run.sh grid > grid.log 2>&1 &    # entrena y evalúa cada variante; horas
python training/summary.py training/out     # la tabla comparada con el orden local y con Jev
training/run.sh export td20h512 td 20       # el mejor: ONNX con vocabulario podado + comprobación de paridad
```

Variables útiles: `EPOCHS` (3), `MICRO_BATCH` (16), `GRAD_ACCUM` (4), `VARIANTS="nombre:fmt:cands:head …"`.
`torch.cuda` debe mostrar la 5090 con `capability (12, 0)`; si dice que `sm_120` no está soportado, el torch
instalado es anterior a la compilación para CUDA 12.8.

**Qué esperar:** en la CPU de 16 hilos del desarrollo, 22 ejemplos entrenan en ~30 s y 1 250 ejemplos (una
época, `head_max_len` 512) en ~50 min. Los ~39 000 ejemplos × 3 épocas en una 5090 deberían ser decenas de
minutos por variante (estimación, no medida).

## Qué se entrena, y por qué así

- **Datos (`make_data.py`)**: cada ejemplo de tldr es un par (qué hace, el comando). La descripción en inglés
  es una pregunta que el comando responde, y la descripción en español (cuando la página está traducida,
  ~8 000 ejemplos) es una pregunta en español. Los candidatos equivocados son **lo que `pistasho ask` recupera
  de verdad** para ese texto, así el modelo aprende a distinguir entre los parecidos que va a ver; el correcto
  se mezcla en un lugar al azar y, en un 10 % de los casos, se quita toda la herramienta para que "none" sea
  aprendible. El objetivo es blando: 0.7 para el ejemplo exacto y 0.3 repartido entre los demás ejemplos de la
  misma herramienta (la métrica de `eval` acierta por herramienta). Un 25 % de las entradas añade una pregunta
  mezclada español-inglés (`crear directories and set their permisos`) hecha con la tabla de sinónimos, para
  atacar el hueco español → inglés. **Nada de esto usa un tercero**, ni tu historial, ni tus atajos, ni los
  bancos de `eval` (el 5 % de las herramientas, por hash, queda fuera para validar). Los textos son de tldr
  (CC BY 4.0: citarlos si se publica el modelo).
- **Variantes (`VARIANTS`)**: Laya lee todas las opciones dentro de **un presupuesto de tokens**
  (`head_max_len`, 256 en el checkpoint publicado: ~11 tokens por opción con 20 candidatos), así que ve poco de
  cada candidato. Se prueban el formato `td` (`herramienta: descripción`) y `tdc` (con el comando), cuántos
  candidatos (10 o 20) y el presupuesto (256, 512, 768). La longitud manda en la latencia en CPU.
- **Pérdida**: `soft-ce` (acepta los objetivos blandos), con `--shuffle-options` para que el orden de los
  candidatos no sea una pista.

## Lo que se probó aquí (CPU y GTX 1650 Ti, sin la 5090)

| Prueba | Resultado |
|---|---|
| `make_data.py` con 300 y con 1 200 entradas, y `laya-train --dry-run` con `head_max_len` 256 y 512 | Sin ejemplos descartados |
| Entrenamiento de humo en CPU (22 ejemplos, 31 s): guarda checkpoint, calibra, evalúa antes/después | Funciona; carga con `laya.load` |
| **Piloto en CPU: 1 252 ejemplos, 1 época, `head_max_len` 512** | En herramientas **nunca vistas**: acierto 21 % → **89 %** (pérdida 2.52 → 0.48). Sobre los bancos de `eval` con 20 candidatos: top-1 **40 %** (reales) y **31 %** (sintéticas), frente a 22 % / 21 % sin ajustar, y al orden local (53 % / 47 %) y Jev (83 % / 76 %) |
| `evaluate.py` sobre un checkpoint propio y sobre el base | Mismas cifras que los experimentos anteriores |
| Exportación a ONNX (`export_onnx.py`) del base y del checkpoint de humo | **Paridad exacta** con PyTorch (Δp = 0.000) |
| Poda de vocabulario (256 000 → 13 651 tokens con el corpus de prueba) sobre el checkpoint **publicado** | ONNX de **544 MB** en vez de 1 288 MB, con paridad exacta en las preguntas de `eval` |
| `run.sh` etapa por etapa en CPU con una variante mínima (`unpack`, `train`, `baseline`, `summary`, `export`) | Todas corren y encadenan; `data` y `pack` se corrieron completos (ver abajo) |
| Datos completos (`run.sh data` + `pack`) | **39 437** ejemplos de entrenamiento y **1 926** de validación por formato (4 104 con "none"), en `bundle.tar.gz` de 39 MB; 14 min con `ask --batch` (la primera versión, un proceso por pregunta, habría tardado ~4 h) |

### Abierto: la poda con un checkpoint ajustado **no** dio paridad

Con el checkpoint publicado la poda es exacta (Δp = 0.000). Con el checkpoint de la prueba mínima (16
ejemplos, `head_max_len` 256) la comprobación de `export_onnx.py` dio **Δp máximo 0.355** (media 0.104)
entre PyTorch y el ONNX podado, y al evaluar ese ONNX con `evaluate.py --onnx --vocab-map` el acierto cayó.
**No se investigó** (se cortó el trabajo ahí). Sospechas, por orden: (1) que el corpus de la poda con datos de
prueba tan pequeños deje fuera tokens de las preguntas (la prueba usó un corpus de 5 606 tokens; el real tendrá
decenas de miles); (2) que el `qtype` o las longitudes del checkpoint ajustado cambien lo que ve el grafo
exportado con formas de ejemplo; (3) un fallo real de la poda con pesos entrenados. **Antes de fiarse de una
exportación hay que repetir esa comprobación con el checkpoint grande y el corpus completo** (`run.sh export`
la imprime); si sigue sin dar paridad, exportar sin podar (`--prune-from` vacío) para aislar la causa.

### Lo que condiciona el diseño

1. **El piloto aprende, pero lo que aprende no se transfiere del todo.** En el reparto interno (herramientas
   nuevas, preguntas con el estilo de los datos) llega a 89 %, pero con las preguntas de `eval`, escritas como
   las escribe una persona, solo sube a 31–40 % con 1/30 de los datos y una época. Es la advertencia de
   siempre: la documentación de tldr no suena a lo que se teclea. El piloto dice "none" en el 45–51 % de las
   preguntas de `eval` (en el entrenamiento es 10 %): ante un estilo que no conoce, desconfía de todo. Con los
   39 000 ejemplos, tres épocas, el aumento mezclado español-inglés y un ajuste de `none_share`, debería
   mejorar; **no está probado**. La puerta de decisión está más abajo.
2. **La cuantización int8 dinámica de ONNX rompe el modelo.** Sobre el checkpoint publicado, el acuerdo del
   argmax con fp32 fue de 8–38 % según la variante (por tensor, por canal, con y sin la tabla de
   embeddings). Así que **el ONNX final es fp32 con el vocabulario podado**, no int8. Queda por probar la
   cuantización estática con calibración sobre entradas reales, solo si la RAM lo exige.
3. **RAM real del ONNX podado en onnxruntime:** ~0.9 GB al cargar y ~1.1 GB tras la primera inferencia (la
   sesión duplica los pesos al optimizar el grafo). Está en el límite del presupuesto de 1 GB; hay que medir de
   nuevo con el vocabulario podado de verdad (con el corpus completo serán ~30–40 mil tokens, no 13 651) y
   ajustar las opciones de la sesión (hilos, arena, grafo optimizado fuera de línea) al integrarlo en Rust.
4. **La latencia depende de la longitud de la secuencia:** con las 20 opciones en 256 tokens, ~170–260 ms en 16
   hilos; con una secuencia de 800 tokens, ~1 s. Mejor precisión con más tokens por opción cuesta latencia:
   por eso la longitud es parte de la rejilla, y por eso `td` y 10 candidatos están entre las variantes.
5. **El formato de entrada de Laya vive en código Python** (`build_sequence`, `build_head`): para usarlo en
   Rust hay que portarlo idéntico (o exportar el preprocesado). Se hace solo si el modelo pasa la puerta.

## Qué datos se pueden usar: los términos de Anthropic y de TypeSafe (2026-11-22)

Se propuso generar preguntas con un modelo de Anthropic y etiquetar con Jev (destilar). **Se leyeron los textos
vigentes y no se puede**, así que no se envió nada (solo un piloto de 96 ejemplos públicos de tldr para ver la
calidad, cuya salida no se guardó en ningún conjunto de datos):

| Proveedor | Texto vigente | Consecuencia |
|---|---|---|
| Anthropic, *Usage Policy* (12 nov 2026, "Do Not Abuse Our Platform") | "Use outputs to train an AI model (e.g., 'model scraping' or 'model distillation') **without prior authorization from Anthropic**" | Entrenar Laya con preguntas generadas por Claude **necesita autorización previa de Anthropic**, sea o no un modelo competidor. |
| Anthropic, *Commercial Terms* D.4 (17 jun 2025) | No acceder al servicio "to build a competing product or service, including to train competing AI models" | Además, el riesgo de que se lea como competidor. |
| TypeSafe, *Master Customer Agreement* (23 sep 2026) | "use the Services or any Output to perform model distillation, **train a model to imitate the output of the Services**, or develop … a similar or competing product or service" | Etiquetar con Jev para entrenar Laya **está prohibido** de forma explícita: es exactamente destilar. |

Lo que sí es independiente de ambos: usar a Jev o a Claude para **medir** (los experimentos de `eval`, `--decide`)
no entrena nada. También queda fuera cualquier pregunta de entrenamiento escrita por el asistente (Claude): son
salidas de Claude. **Los bancos `eval/questions-synth.toml` se escribieron como banco de prueba, no de
entrenamiento, y deben seguir así.** (La recomendación anterior de "destilar Jev" y de mezclar tldr, Jev y
Anthropic no se había comprobado contra estos términos y era incorrecta.)

Vías que quedan: (1) un modelo abierto con licencia que permita usar sus salidas (por ejemplo Apache-2.0, como
la familia Qwen3; se verifica la licencia del punto de control exacto antes de usarlo) que escriba las preguntas
**en la propia 5090**, un proceso intermedio y sin terceros; (2) pedir autorización por escrito a Anthropic;
(3) quedarse con tldr (en inglés y español, ya hecho) más conjuntos públicos con licencia permisiva, por
ejemplo NL2Bash (descripciones humanas en inglés de comandos de bash), y datos reales del programa opt-in de la
visión a largo plazo.

## Generar las preguntas con un modelo abierto en un servicio en línea (decisión del usuario, 2026-11-22)

La 5090 no se usa en esta fase, así que las preguntas con estilo humano las escribe un **modelo de pesos
abiertos en un servicio alojado**. Se comprobaron las dos capas de términos:

- **Modelos (fichas de Hugging Face):** Qwen3.5-27B y Qwen3.5-35B-A3B (Apache-2.0, 201 idiomas, el modo
  "thinking" viene activado y se apaga con `enable_thinking: false`), Gemma 4 31B-it / 12B-it / 26B-A4B-it
  (la ficha declara Apache-2.0 y 35+ idiomas explícitos con español; el texto de la licencia no se pudo
  verificar: el enlace redirige a un dominio que no es de Google y el repo no trae LICENSE), EuroLLM-22B
  (Apache-2.0, español nativo, pero no está en estos servicios) y gpt-oss-20b (Apache-2.0). Descartados: Llama,
  Gemma ≤ 3, los Mistral grandes o de licencia "other", Ministral-3-14B (licencia sin declarar).
- **Servicios (textos vigentes):** [DeepInfra](https://deepinfra.com/terms) (15 oct 2026), [Together AI](https://www.together.ai/terms-of-service)
  (19 may 2026) y [OpenRouter](https://openrouter.ai/terms) (31 ago 2026) **no restringen** entrenar con las
  salidas (DeepInfra y Together solo prohíben competir con su propio negocio de inferencia; OpenRouter remite
  a los términos de cada modelo, aquí Apache-2.0). Fireworks quedó fuera: un resumen indica que prohíbe usar el
  contenido, que incluye las salidas, para entrenar IA, y no se leyó entero. Los precios consultados (por millón
  de tokens, entrada / salida): Gemma 4 31B ≈ $0.20 / $0.40 y Qwen3.5-27B ≈ $0.26 / $2.60 en DeepInfra; todo el
  tldr cuesta unos $1.5 con Gemma y unos $6 con Qwen (Qwen sin "thinking").

Herramientas (`training/`): `gen_questions.py` (cualquier API compatible con OpenAI; la clave se lee de una
variable de entorno y nunca se imprime; reanudable; descarta preguntas que nombran la herramienta o sus
opciones; el modelo devuelve una lista vacía si la entrada es demasiado genérica), `quality_report.py`
(comparación entre modelos: vacías, idioma, repeticiones, si la pregunta recupera su ejemplo con
`pistasho ask`, y una hoja de lectura **a ciegas** con la clave aparte) y `make_data.py --generated`
(añade esas preguntas, con la etiqueta por construcción). Todo probado contra un servidor falso, **no contra
un servicio real**.

```sh
export DEEPINFRA_API_KEY=...      # la clave la pones tú en tu shell; nunca en el chat
python3 training/gen_questions.py --base-url https://api.deepinfra.com/v1/openai --key-env DEEPINFRA_API_KEY \
    --model google/gemma-4-31B-it --limit 600 --out training/out/gen/gemma.jsonl
python3 training/gen_questions.py ... --model Qwen/Qwen3.5-27B --out training/out/gen/qwen.jsonl \
    --extra-json '{"chat_template_kwargs": {"enable_thinking": false}}'
python3 training/quality_report.py training/out/gen/gemma.jsonl training/out/gen/qwen.jsonl --retrieve \
    --review training/out/gen/review.md
```

(Comprobar el identificador exacto del modelo y el campo para apagar el "thinking" en la página del servicio.)

## Cómo escribe la gente en una línea de comandos (corrección del usuario, 2026-11-22)

Al revisar a ciegas las preguntas generadas, el usuario confirmó que el **contenido** suena humano, pero el
**formato** no: nadie escribe así en una terminal. Una persona escribe **todo en minúsculas, sin tildes**
(la tilde pide una tecla muerta antes de la vocal; la ñ tiene tecla propia y se queda), **sin `¿` ni `¡`**,
sin signos de exclamación y **sin punto final**; las comas sí se usan y un `?` al final aparece a veces, como en
inglés. Lo mismo en inglés: sin mayúscula inicial. Es la escritura informal óptima para un usuario común.

Consecuencia: un modelo entrenado con la ortografía formal se prueba con preguntas que no se le parecen (los
embeddings y Laya ven tokens distintos con y sin tilde). `training/style.py` aplica esa regla de forma
determinista y se usa con probabilidad (`make_data.py --casual`, 0.8 por defecto: algunas personas sí
escriben bien, y los bancos de `eval` escritos a mano en su mayoría llevan tildes). También hay una versión
informal de los bancos (`make_eval_cache.py --casual`) para medir las dos formas. Se aplica a **todas** las
preguntas, las generadas y las derivadas de tldr, antes de recuperar los candidatos.

## Resultado en la 5090 (2026-11-23): agotado

Se corrieron 23 variantes (datos de 10k a 100k filas, épocas, composición, rango local en el texto, tasas de
aprendizaje, presupuesto de tokens, pérdida, codificador congelado, solo desacuerdos, `none_share`, aumentos).
El mejor top-1 es 54 / 44 contra el ≥ 70 % exigido; el modelo con el rango local solo iguala al orden local
(53 / 46) y la fusión queda en ±3 puntos. Cifras, diario y costos: `docs/training-experiments.md`.

Lo que conviene recordar si se reabre:
- **La validación de este diseño no predice la transferencia** (herramientas no vistas, pero de las mismas
  fuentes). Medir siempre en los bancos, tal cual y casual, y con varias semillas: el banco original (101
  preguntas) se mueve hasta ~5 puntos entre semillas.
- **Laya no ve la posición local** (se barajan las opciones). Darle el rango como texto (`tdr`) fue la única
  palanca que movió el top-1. Las filas con el ejemplo exacto inyectado no pueden llevar un rango inventado.
- Más datos (> 40k) y más épocas no ayudan; «none» no se arregla con `--none-share`.
- No se midió el ONNX (paridad, RAM, latencia) con un checkpoint grande: la poda con pesos ajustados sigue sin
  verificarse (arriba, «Abierto»).
- `laya-train` usa bf16 en CUDA según el checkpoint (`amp_dtype`) y no necesita parche en Blackwell.

Datos de entrenamiento: textos de tldr (CC BY 4.0), preguntas generadas con Gemma 4 31B (Apache-2.0) y
preguntas a nivel de herramienta de **NL2Bash** (Lin, Wang, Zettlemoyer y Ernst, *NL2Bash: A Corpus and Semantic
Parser for Natural Language Interface to the Linux Operating System*, LREC 2018; datos con licencia MIT): citarlos
si se publica algún modelo.

## Puerta de decisión

Con la tabla de `summary.py`, la variante elegida debe cumplir:

1. **Acierto:** top-1 ≥ ~70 % en los dos bancos (el orden local da 53 % / 47 %; Jev, 83 % / 76 %), y que "none"
   no se diga a la ligera (mucho menos de la mitad de las preguntas).
2. **Recursos:** ONNX fp32 podado dentro de ~1 GB de RAM en onnxruntime y ≤ ~300 ms por decisión en CPU.
3. Si no llega: probar `none_share` y las variantes de datos antes de dar el modelo por malo; la siguiente
   palanca son preguntas escritas como las escribe la gente (propias, públicas o de un maestro), que **piden
   el visto bueno del usuario** y revisar los términos de uso si las genera un servicio.

Si pasa: portar el preprocesado a Rust (`ort` y `tokenizers` ya son dependencias), el **daemon** (socket unix,
carga perezosa, se apaga solo tras ~10 min sin uso) y el orden de respaldo: modelo local → Jev (opcional) → LLM
(último recurso). Si no pasa: queda Jev opcional y el trabajo de aquí sigue siendo reutilizable.
