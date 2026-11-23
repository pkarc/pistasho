# Plan de ejecución en la RTX 5090 (entrenar Laya)

> Guion para una sesión de Claude Code que corre **en la máquina de la GPU**, sin la memoria ni el historial de
> la sesión que preparó todo. Va solo en español. Complementa a [`docs/training.md`](training.md) (el runbook
> y las decisiones) y a `CLAUDE.md`. Léelos primero; aquí está lo que hace falta para ejecutar sin adivinar.

## 0. Contexto en cinco párrafos

**Qué es pistasho.** Herramienta en Rust: `Ctrl-Space` en zsh, el usuario escribe una pregunta ("comprimir una
carpeta") y el comando queda en el prompt, nunca se ejecuta. Busca en tldr-pages con BM25 + embeddings
(`multilingual-e5-small`) y entrega 20 candidatos. Con ellos, el orden local acierta el comando correcto en el
puesto 1 el **53 % / 47 %** de las veces (banco original / banco sintético). Un modelo de decisión que reordene
esos 20 candidatos lleva ese número a **83 % / 76 %** (medido con Jev, un servicio alojado de TypeSafe).

**Para qué se entrena.** El usuario quiere **no depender de terceros**: Jev y Anthropic quedan como opciones
suyas. La alternativa local es **Laya** (mmBERT-base, 322 M de parámetros, `convaiinnovations/laya`, subcarpeta
`multilingual`): un codificador que lee la pregunta y todas las opciones juntas y puntúa cada una (más una
opción "none"). Sin ajustar acierta 22 % / 21 %. Hay que **ajustarlo** (fine-tune) con datos de esta tarea.
La 5090 sirve para entrenar una vez; lo que se entrega es un ONNX que corre en CPU del usuario.

**Presupuesto de uso (lo que el modelo final debe cumplir).** ≤ ~1 GB de RAM residente en onnxruntime, ≤ ~300 ms
por decisión en CPU, en un daemon que se apaga solo tras ~10 min sin uso. Hoy el ONNX fp32 podado pesa ~544 MB
y ocupa ~0.9–1.1 GB de RAM: **está justo en el límite**.

**Dónde estamos.** Los pilotos en CPU (77 actualizaciones, ~1 250 ejemplos, una época) dieron como mejor
resultado **40 % / 34 %** de top-1, por debajo del orden local. Eso **no** es evidencia contra el método: eran
77 pasos. Lo que se quiere averiguar aquí es si con 10–100 veces más datos y con GPU el modelo llega a la
puerta de decisión (§6). El mayor problema detectado: con preguntas escritas "como las escribe la gente" el
modelo dice "none" demasiado poco o demasiado mal (la tasa de none cayó de ~50 % a 15–18 %).

**Reglas que no se rompen (el usuario las fijó).**
1. **Nunca entrenar con salidas de Anthropic/Claude ni de Jev (TypeSafe).** Sus términos lo prohíben. Los
   datos vienen de tldr, de NL2Bash (MIT) y de preguntas generadas con **Gemma 4 31B** (Apache-2.0) vía
   OpenRouter. Medir con Jev o Claude sí está permitido; entrenar no. No añadas datos de otra procedencia sin
   preguntar.
2. **Los bancos de `eval` son solo de prueba**: nunca entrenar con ellos (`eval/questions*.toml`).
3. **Las preguntas se escriben como las teclea la gente**: minúsculas, sin tildes, sin `¿¡`, sin punto final
   (`training/style.py`). Se mide en los dos bancos: tal cual y casual.
4. **No hacer commit ni push sin que el usuario lo pida.** Los resultados grandes no se versionan
   (`training/out/` está en `.gitignore`).
5. No dejar scripts sueltos en `/tmp` (se perdieron una vez); todo va en `training/`.

## 0.5 Cuándo se cierra este plan (decisión del usuario)

El objetivo de la sesión de la 5090 es **dejar todo lo de entrenamiento listo allá**. Este plan **no se cierra
al terminar la lista de pasos**: se cierra solo cuando ocurra una de dos cosas.

1. **Se logra un entrenamiento óptimo:** una variante pasa la puerta de decisión (§6), con la exportación a ONNX
   en paridad y medida en CPU (§7), y repetirla con otra semilla da un resultado parecido.
2. **Se agotan las opciones:** se probó el catálogo de palancas (§6.5) con criterio y ninguna llega; en ese caso
   se documenta con cifras por qué y qué queda (Jev opcional).

Mientras no ocurra ninguna, **se itera**: leer la tabla, formular una hipótesis, cambiar *una* cosa, entrenar,
medir, anotar. No pedir permiso para cada experimento dentro de este plan (sí para commits, para gastar dinero
en servicios y para añadir datos de procedencia nueva). Llevar un diario en `training/out/EXPERIMENTS.md`
(una fila por corrida: fecha, nombre, qué cambió respecto de la anterior, filas, épocas, hiperparámetros,
top-1 / top-3 en los cuatro bancos, none, tiempo, conclusión). Sin diario no hay forma de saber qué se probó.

Al cerrar, entregar el diario, la tabla final y la decisión (óptimo o agotado) al usuario.

## 1. Qué trae el paquete

`bundle.tar.gz` lo genera `training/run.sh pack` en la máquina de desarrollo y el usuario lo copia. Contiene:

| Archivo | Qué es |
|---|---|
| `data-td/train.jsonl`, `valid.jsonl` | ejemplos en formato `td` (opción = `herramienta: descripción`), 20 candidatos |
| `data-tdc/train.jsonl`, `valid.jsonl` | lo mismo con formato `tdc` (la opción lleva también el comando) |
| `eval-cands.json` | candidatos congelados de los dos bancos de `eval`, preguntas tal cual |
| `eval-cands-casual.json` | los mismos bancos escritos en estilo casual |

Cada fila de entrenamiento: pregunta + hasta 20 candidatos reales de `pistasho ask` (el correcto se inserta si
la búsqueda lo falló, todo barajado), 10 % de filas "none" (se quitó la herramienta correcta), objetivo blando
(0.7 al ejemplo exacto y 0.3 repartido entre los de la misma herramienta). La validación (5 % de las
herramientas, por hash md5) **nunca** se ve en el entrenamiento. Cada herramienta aporta como máximo 150 filas
(`--max-per-tool`) para que `tar`, `git` y `docker` no ahoguen a las demás.

De dónde salen las preguntas de entrenamiento: descripciones de tldr en inglés y español (con variantes: "cómo
hago…", palabras caídas, mezcla español-inglés), preguntas generadas con Gemma en dos estilos (pasadas A y B,
~61 000 entradas) y preguntas a nivel de herramienta de NL2Bash (Lin et al., LREC 2018; datos MIT; citar si se
publica el modelo; los textos de tldr son CC BY 4.0).

> Los conteos exactos de filas quedan en `training/out/data.log` de la máquina de desarrollo; al desempaquetar,
> `wc -l training/out/data-td/*.jsonl` los da.

### 1.1 El kit de regeneración (`kit.tar.gz`)

Para iterar sobre los **datos** (palancas 4, 5, 9 del §6.5) sin volver a la máquina de desarrollo, el usuario
copia también `kit.tar.gz` (`training/run.sh kit`): el binario `pistasho` de release (Linux x86_64), una base
limpia con los vectores, el modelo de embeddings, los archivos de preguntas generadas (`gemma*.jsonl`), NL2Bash
y la caché de candidatos. En la 5090:

```sh
tar xzf /ruta/kit.tar.gz -C training/out            # crea training/out/kit/
export PISTASHO_BIN=$PWD/training/out/kit/pistasho
export XDG_DATA_HOME=$PWD/training/out/kit/data XDG_CONFIG_HOME=$PWD/training/out/kit/config HISTFILE=/dev/null
export PISTASHO_EMBEDDINGS=1
training/out/venv/bin/python training/make_data.py --help     # luego con --out training/out/data-v2 --generated training/out/kit/sources/gemma*.jsonl --toollevel training/out/kit/sources/nl2bash.jsonl --max-per-tool 150 …
```

Usar siempre una carpeta de salida **nueva** por versión de datos (`data-v2`, …) y anotarla en el diario; las
bases y los candidatos del kit son de solo lectura en la práctica (la base está limpia: **sin** atajos ni
historial del usuario; no la mezcles con otra). El sqlite3 CLI no es necesario allá.

## 2. Preparar la máquina (una vez)

Requisitos: Python 3.10+, ~25 GB de disco libres (el checkpoint de Laya son 650 MB; los checkpoints de cada
corrida pesan ~1.3 GB cada uno; los ONNX, ~0.5–1.3 GB), acceso a Hugging Face, driver NVIDIA reciente.

```sh
git clone https://github.com/pkarc/pistasho && cd pistasho
nvidia-smi                                  # ¿ve la 5090?
training/run.sh setup                       # venv en training/out/venv, torch cu128 (Blackwell), laya==0.5.2, onnx
training/run.sh unpack /ruta/al/bundle.tar.gz
wc -l training/out/data-td/*.jsonl
```

`setup` imprime `torch … cuda True`, el nombre de la GPU y `capability (12, 0)` y la VRAM. **Si dice que
`sm_120` no está soportado**, el torch instalado es anterior a CUDA 12.8: reinstalar con
`pip install --upgrade torch --index-url https://download.pytorch.org/whl/cu128`. Si el `git clone` no trae lo
último, comprobar que `git log` muestra el commit "Per-tool cap, smoke and ladder stages…".

Variables de `run.sh`: `EPOCHS` (3), `MICRO_BATCH` (16), `GRAD_ACCUM` (4), `VARIANTS`, `SIZES`
(`10000 40000 100000`), `OUT`, `PY`. Lotes efectivos: 16×4 = 64.

## 3. Paso 1: la referencia y la prueba de humo

```sh
training/run.sh baseline        # Laya sin tocar. Esperado: top-1 ~22 % / ~21 %, mediana de latencia en GPU
training/run.sh smoke           # 200 filas, 1 época, head 512: filas/s, memoria, sin NaN
```

Qué mirar en `training/out/runs/smoke/train.log` y la línea final `trained 200 rows x 1 epochs in N s`:

- **Sin NaN ni `inf` en la pérdida.** La precisión mixta (fp16) de `laya-train` en Blackwell **no está probada**, y
  `laya-train` **no tiene un flag de precisión** (`--help` solo ofrece `--device`, `--encoder-lr`, `--head-lr`,
  `--loss`, `--label-smoothing`, `--target-error`, `--min-abstain-n`, `--freeze-encoder`, `--seed`). Si hay NaN,
  primero bajar `--encoder-lr`; si persiste, mirar el código instalado (`venv/lib/python*/site-packages/laya/`,
  buscar `autocast`/`GradScaler`) y proponer al usuario un parche local a bf16 o fp32.
- **Filas por segundo** (= 200 / N, descontando la carga ~20–30 s). Con eso se estima todo lo demás:
  `horas = filas × épocas / (filas_por_s × 3600)`. Apunta el número: decide el tamaño de la escalera.
- **Memoria**: `nvidia-smi` imprime `memory.used`. Si hay OOM con `MICRO_BATCH=16`, bajar a 8 y subir
  `GRAD_ACCUM` a 8 (mismo lote efectivo).
- La evaluación al final corre `evaluate.py` en los dos bancos (casual y tal cual); con 200 filas el resultado
  no significa nada, solo comprueba que todo el cableado funciona.

## 4. Paso 2: la escalera de datos (¿ayuda tener más?)

```sh
nohup training/run.sh ladder > ladder.log 2>&1 &     # 10k, 40k y 100k filas, 1 época cada una
python training/summary.py training/out               # tabla comparada
```

Cada escalón es una submuestra **balanceada por herramienta** (`subsample.py`) con la misma receta (`td`, 20
candidatos, head 512, `max_len` 768, 1 época). Si el conjunto tiene menos de 100k filas, el último escalón
usa todo. **Si la estimación del humo dice que un escalón pasa de ~3 h, quita ese tamaño de `SIZES`** o
sube el micro-batch.

La tabla (`summary.py`) muestra, por variante: top-1 / top-3 en los dos bancos tal cual y casual, la tasa de
`none`, la latencia mediana y la RAM pico, junto a las referencias "orden local" y "Jev".

**Cómo leerla:**

| Lo que ves | Qué significa | Siguiente paso |
|---|---|---|
| top-1 sube con cada escalón (≥ +5 puntos por salto) | los datos son el cuello de botella | ir a 100k+ con 3 épocas |
| sube de 10k a 40k y se aplana a 100k | los datos ya no mandan | probar variantes (§5), `head`, `tdc` |
| no supera al orden local (53/47) ni con 100k | algo falla en la receta o en la señal | revisar el §7 antes de gastar más horas |
| `none` casi nunca o casi siempre | desajuste del 10 % de filas "none" o de la calibración | revisar `--none-share` y `evaluate.py` |

Una pregunta del banco mueve ~1 punto en el original (101 preguntas) y ~0.35 en el sintético (287): **no
sacar conclusiones de diferencias menores a ~3 puntos**, sobre todo en el banco original.

## 5. Paso 3: las variantes (`grid`) con el tamaño que gane

Ajusta el número de filas con `subsample.py` si hace falta (o usa el conjunto entero), y elige las variantes.
Por defecto `VARIANTS="td20h256:td:20:256 td20h512:td:20:512 tdc20h768:tdc:20:768 td10h256:td:10:256"`, es
decir (nombre:formato:candidatos:presupuesto de tokens de las opciones):

- `td20h256`: el checkpoint publicado usa 256; ~11 tokens por opción, ve poco de cada candidato.
- `td20h512`: el candidato de trabajo (más texto por opción, ~250 ms de latencia en CPU).
- `tdc20h768`: incluye el comando en cada opción; mejor señal, ~1 s en CPU (**probablemente no cabe en ≤ 300 ms**,
  pero sirve para saber el techo).
- `td10h256`: 10 candidatos, la más barata.

```sh
EPOCHS=3 nohup training/run.sh grid > grid.log 2>&1 &
python training/summary.py training/out
```

Para usar solo parte de los datos en el `grid`, apunta la variable `OUT` a una carpeta con `data-td`/`data-tdc`
submuestreados (el `grid` lee `$OUT/data-$fmt`), o entrena una a una:
`training/run.sh train NOMBRE FMT CANDS HEAD [CARPETA_DE_DATOS]`.

## 6. Puerta de decisión (copiada de `docs/training.md`)

La variante elegida debe cumplir **todo**:

1. **Acierto:** top-1 ≥ ~70 % en los dos bancos (en ambas versiones, tal cual y casual; referencias: orden local
   53/47, Jev 83/76), y "none" no se dice a la ligera (mucho menos de la mitad de las preguntas, idealmente ≲ 20 %).
2. **Recursos:** ONNX fp32 podado dentro de ~1 GB de RAM en onnxruntime y ≤ ~300 ms por decisión en CPU.
3. Confianza calibrada (el umbral que decide cuándo llamar a Jev / al LLM debe poder ponerse con la salida del
   modelo).

Aplicación práctica:
- **Pasa 1 y 2** → exportar (§7), medir en la CPU **de la máquina del usuario**, y recién ahí se porta a Rust.
- **Pasa 1 pero no 2** (acierta, pero pesa o tarda) → `td10h256`/`head 256`, poda más agresiva, o bajar candidatos.
- **Palancas de entrenamiento que `run.sh` no expone** (pasan directo a `laya.train_cli`; añadir variables a
  `stage_train` si se usan): `--encoder-lr` / `--head-lr` (si la pérdida oscila o el top-1 baja tras la
  primera época, bajar el del codificador), `--label-smoothing`, `--target-error` y `--min-abstain-n` (la
  calibración del "none" / abstención), `--seed` (repetir la mejor variante con otra semilla para medir ruido).
- **No pasa 1** → no insistir con más épocas a ciegas: probar (a) `--none-share`, (b) la mezcla de estilos, (c)
  más datos por herramienta, y reportar. Que el modelo local no llegue es un resultado válido: queda Jev opcional.

## 6.5 Catálogo de palancas (para iterar hasta cerrar)

Ordenadas por coste esperado/beneficio. Cambiar una a la vez y medir en los cuatro bancos.

1. **Cantidad de datos y épocas:** escalera (§4), luego 2–4 épocas con el tamaño que gane.
2. **Tasas de aprendizaje:** `--encoder-lr` (probar 0.5× y 2× del valor por omisión), `--head-lr`.
3. **Presupuesto de tokens (`head`) y formato:** 256 / 512 / 768 / 1024; `td` vs `tdc`; 10 vs 20 candidatos.
4. **"None":** `--none-share` en `make_data.py` (0.05 / 0.10 / 0.20), `--target-error` y `--min-abstain-n`
   (calibración del abstenerse). Requiere regenerar solo los datos (necesita pistasho y su base: pedir al usuario
   o, en la 5090, reponer filas "none" con un script sobre los JSONL existentes quitando la herramienta correcta).
5. **Mezcla de datos** (`--mix`, `--casual`, `--max-per-tool`, proporción generadas / tldr / NL2Bash): ¿qué
   fuente ayuda o perjudica? Entrenar solo con cada fuente da la respuesta.
6. **Pérdida:** `--label-smoothing`; `--loss rlcd` (la alternativa de laya); objetivo blando 0.7/0.3 vs otros
   repartos.
7. **Descongelar/congelar:** `--freeze-encoder` como control (¿aprende algo la cabeza sola?).
8. **Currículo:** primero tldr puro y luego generadas, o al revés.
9. **Aumentos propios** sobre los JSONL: más variantes de estilo casual, errores de tecleo, mezcla es/en,
   preguntas cortas (2–3 palabras, lo que la gente escribe de verdad).
10. **Semillas y conjunto:** repetir lo mejor con 3 semillas; promediar pesos o elegir la mediana.
11. **Distilación a un modelo más pequeño** si el peso es lo que falla (por ejemplo `multilingual` podado de
    capas): solo con datos permitidos (§0), y como último recurso porque cambia el diseño.
12. **Cambiar el problema** si todo lo demás falla: reordenar con un cross-encoder más pequeño, o usar el modelo
    local solo para decidir "none" y dejar el orden al híbrido (probar con los puntajes ya medidos).

**Agotado** significa: las palancas 1–10 probadas con diario, la mejor variante repetida con tres semillas y aún
por debajo de la puerta, o por encima pero fuera del presupuesto de recursos sin arreglo en la exportación.

## 7. Paso 4: exportar y comprobar paridad (solo para el mejor)

```sh
training/run.sh export NOMBRE td 20     # export_onnx.py: poda del vocabulario (256k → ~13k), ONNX fp32, verificación
```

`export_onnx.py` compara las probabilidades del checkpoint de PyTorch con las del ONNX en 12 preguntas del
banco. **Hallazgo abierto que hay que resolver aquí:** con un checkpoint de prueba mínimo (afinado con 22
ejemplos), la poda dio un error de probabilidad **Δp = 0.355** (con el modelo base la paridad era exacta y el
modelo de 544 MB funcionaba). Hipótesis, en orden de coste:

1. Exportar **sin podar** (`--prune-from` omitido) con el checkpoint afinado: si la paridad vuelve, el fallo
   está en la poda (qué tokens se conservan, `--keep-first`, o pesos atados de la capa de salida).
2. Si sigue mal sin podar: es el exportador (operadores, longitud fija, máscara) con pesos entrenados; comparar
   capa a capa con hooks.
3. Si el ONNX sin podar sale bien, medir el tamaño real (~1.3 GB fp32): no cabe en el presupuesto; entonces la
   poda hay que arreglarla, no evitarla.

Los ONNX **int8 dinámicos rompen** el modelo (descartado; por eso `--no-int8`). Alternativas a explorar solo si
el peso importa: fp16 con conversión cuidadosa, o cuantización estática calibrada.

Después de exportar, medir en CPU **con límite de memoria** (el entorno del usuario es la CPU, no la 5090):

```sh
/usr/bin/time -v training/out/venv/bin/python training/evaluate.py --model training/out/runs/NOMBRE/ckpt \
  --onnx training/out/runs/NOMBRE/onnx/model.onnx --vocab-map training/out/runs/NOMBRE/onnx/vocab_map.npy \
  --cache training/out/eval-cands.json --fmt td --cands 20 --device cpu
```

(Confirmar los nombres reales de los archivos con `ls training/out/runs/NOMBRE/onnx`.) El número que importa es
el RSS máximo y la latencia mediana por pregunta.

## 8. Qué devolver al usuario (y a la otra sesión)

No copiar los checkpoints. Lo que vale:

- La tabla de `python training/summary.py training/out` completa.
- `training/out/runs/*/eval.json`, `eval-casual.json`, `train.log` (las pérdidas por época) y la salida de
  `baseline`.
- Para cada variante: filas × épocas, tiempo total y filas por segundo; memoria de GPU usada.
- Resultado de la exportación: tamaño del ONNX, paridad (Δp), RSS y latencia en CPU.
- El ONNX elegido (`runs/NOMBRE/onnx/`) **solo si pasó la puerta**.
- Cualquier cambio a `training/*.py` o `run.sh` que haga falta (por ejemplo el flag de precisión). El usuario
  decide el commit.

Con esos números se actualizan `docs/eval.md`, `docs/training.md` (sección de resultados) y la sección 3.2–3.4
de `PLAN.md`, y se decide si se porta a Rust (preprocesado idéntico: plantilla de opciones, `head_max_len`
repartido entre las opciones, tokenizador `tokenizers`, `ort`, poda con `vocab_map`) y el daemon (socket unix,
carga perezosa, se apaga solo tras ~10 min).

## 9. Trampas conocidas

- **Disco:** una cuantización llenó el disco al 100 % y corrompió ONNX. Mantén ≥ 10 GB libres y borra
  `runs/*/ckpt` de las variantes descartadas.
- **zsh y globs:** un `rm` con un glob sin coincidencias aborta todo el comando en zsh; usa `rm -f` por archivo
  o `setopt nonomatch`.
- **Reiniciar el equipo mata el entrenamiento** (no hay checkpoint intermedio): usa `nohup`/`tmux` y evita el
  suspendido. Un `grid` largo puede dividirse con `VARIANTS` para no perderlo todo.
- **`set -e` / `pipefail`**: `run.sh` los usa; una tubería con `grep` sin coincidencias (`grep -v warn`) puede
  matar la etapa en silencio si no se tiene cuidado. Si una etapa termina "muy rápido", mira el `.log`.
- **El tamaño del banco manda en el ruido**: ver §4. No elegir variante por 1–2 puntos.
- **`evaluate.py` con `--cands`** debe coincidir con los candidatos con los que se entrenó (20 o 10).
- Las preguntas del banco `eval` en español llevan tildes; el estilo casual no. Un modelo que acierta solo en
  una de las dos formas **no pasa** la puerta.

## 10. Lista de verificación

- [ ] `nvidia-smi` y `torch.cuda` ven la 5090 (`capability (12, 0)`)
- [ ] `unpack` hecho; conteos de filas anotados
- [ ] `baseline` ≈ 22 % / 21 %
- [ ] `smoke` sin NaN; filas/s y memoria anotadas; precisión de entrenamiento decidida
- [ ] `ladder` con la tabla; conclusión sobre si más datos ayuda
- [ ] `grid` (o variantes elegidas) con la tabla; mejor variante nombrada
- [ ] Puerta de decisión evaluada (§6) con los números reales
- [ ] Exportación: paridad resuelta, tamaño, RSS y latencia en CPU
- [ ] Diario `EXPERIMENTS.md` al día; plan cerrado solo por **óptimo** o **agotado** (§0.5), con la conclusión escrita
- [ ] Resultados devueltos al usuario (§8); sin commits sin su permiso
