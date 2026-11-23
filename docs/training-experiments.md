# Experimentos de entrenamiento en la RTX 5090 (Laya)

> Diario de la sesión del 2026-11-23 que ejecutó [`docs/training-5090.md`](training-5090.md) y tabla final con el costo de cada corrida.
> Conclusión: **agotado** (§0.5, punto 2), ninguna variante pasa la puerta de decisión; ver "CIERRE" más abajo. Solo en español, como el resto de las notas de proyecto.
> Los datos, los checkpoints y las claves no se versionan (`training/out/` está en el `.gitignore`). Las corridas se reproducen con `training/run.sh` y las colas `training/queue-*.sh`.

## Diario

Formato: fecha | nombre | cambio | filas x épocas | hiperparámetros | top-1/top-3 (orig, synth, orig-casual, synth-casual) | none | tiempo | conclusión

Datos: data-td train 172 324 filas, valid 8 388 (bundle). GPU: RTX 5090 32 GB, torch cu128.

| fecha | corrida | qué cambió | filas × épocas | hiperparámetros | top-1 / top-3 en los cuatro bancos | none | tiempo | conclusión |
|---|---|---|---|---|---|---|---|---|
| 2026-11-23 | baseline | Laya sin tocar | 0 | - | orig 27/50, synth 22/44 | 25-28 % | 20 s | ≈ esperado (22-27 %). Local: 53/76, 46/66 |
| 2026-11-23 | smoke | 200 filas, 1 época, head 512 | 200x1 | mb16 ga4, lr por omisión | as-written orig 28/53, synth 27/50; casual orig 23/53, synth 25/51 | 8-9 % | 85 s total (incluye eval de 8 388 filas valid) | cableado OK, sin NaN (pérdida 2.70). laya usa bf16 en CUDA por checkpoint (`amp_dtype`), no hace falta parche. Filas/s real: medir en ladder-10000 |
| 2026-11-23 | ladder-10000 | 10k filas balanceadas por herramienta, td, 20 cands, head 512, max_len 768 | 10000x1 | mb16 ga4 (150 updates), soft-ce | as-written orig 39/67, synth 34/62; casual orig 39/67, synth 36/62 | 11-18 % | 159 s (~63 filas/s); acc valid 0.16→0.60; pérdida media 1.415 (oscila 1.1–1.6 entre pasos 300-500) | mejor que baseline (27/22) pero < orden local (53/46). Throughput: 172k filas ≈ 46 min/época → la escalera completa cabe sin recortar. Siguiente: 40k y 100k |
| 2026-11-23 | ladder-40000 | 40k filas (mismo recipe) | 40000x1 | idem | as-written orig 44/72, synth 37/65; casual orig 43/73, synth 38/66 | 2-4 % | 387 s (~103 filas/s); acc valid 0.73; pérdida media 0.997 | +3-5 puntos sobre 10k; sigue < orden local (53/46). "none" casi desaparece (2-4 %): pendiente none-share / calibración |

## 2026-11-23 · Diagnóstico de validación (sin entrenar nada nuevo)
Cambios de código (sin commit): `make_data.py` escribe `train.meta.jsonl` / `valid.meta.jsonl` (fuente, orden local); regenerado en `data-meta/` con la caché del kit y **verificado idéntico byte a byte** a `data-td`. Nuevos: `valid_breakdown.py`, `subset_by_source.py`, `queue-1.sh`. Nota: `subsample.py` es muestreo uniforme al azar, NO balanceado por herramienta (el plan decía lo contrario).

Composición del entrenamiento (172 324 filas): gemma-a 36 %, gemma-b 39 %, tldr-en 17 %, tldr-es 3 %, tldr-mix 2 %, nl2bash 2 %. En 71 179 filas (39 %) el ejemplo exacto no salía entre los 20 candidatos y se inyectó.

Validación (8 388 filas, herramientas no vistas). local = el primer candidato local es una herramienta correcta; model = acierto a nivel de herramienta:
| fuente | filas | local | 10k | 40k |
|---|---|---|---|---|
| gen-gemma | 2985 | 36 % | 65 % | 77 % |
| gen-gemma-b | 3155 | 23 % | 54 % | 71 % |
| nl2bash | 334 | 13 % | 11 % | 19 % |
| tldr-en | 1432 | 74 % | 81 % | 90 % |
| tldr-es | 316 | 83 % | 77 % | 86 % |
| tldr-mix | 166 | 72 % | 87 % | 94 % |
| ALL sin none | 7524 | 39 % | 66 % | 76 % |
| ALL (exacto = acc. de laya-train) | 8388 | - | 60 % | 73 % |

Conclusión provisional: en validación el modelo supera al orden local por ~37 puntos (39 → 76), pero en los bancos está ~9 por debajo (44 vs 53). La validación está llena de preguntas donde el orden local falla (gemma, 23-36 %), mientras que los bancos son preguntas donde el orden local ya acierta ~50 %. NL2Bash (preguntas humanas a nivel de herramienta) es la fuente que peor transfiere (19 %) y solo pesa el 2 %.

| fecha | corrida | qué cambió | filas × épocas | hiperparámetros | top-1 / top-3 en los cuatro bancos | none | tiempo | conclusión |
|---|---|---|---|---|---|---|---|---|
| 2026-11-23 | ladder-100000 | 100k filas (mismo recipe) | 100000x1 | idem | as-written orig 42/71, synth 33/64; casual orig 41/71, synth 34/65 | 9 % | 921 s (~109 filas/s); pérdida media 0.750 | NO mejora sobre 40k (dentro del ruido ±3). Escalera plana desde 40k: con 1 época los datos no son el cuello de botella. Pérdida de train sigue bajando (1.42→1.00→0.75) sin transferir a los bancos. Siguiente: e3-mix40k y gemma40k (queue-1.sh) |

## 2026-11-23 · Nota de método: la validación NO mide transferencia
`valid.jsonl` son herramientas no vistas pero de las MISMAS fuentes que el entrenamiento (tldr, Gemma, NL2Bash); con 40k da 73-75 % mientras los bancos dan ~40 %, y el orden local saca 39 % ahí contra ~50 % en los bancos. **Manda el resultado en los bancos (original y sintético, tal cual y casual).** La validación solo sirve para detectar que el entrenamiento aprende algo y, desglosada por fuente (`valid_breakdown.py`), para ver qué fuente es fácil por coincidencia de texto (las derivadas de tldr).

## 2026-11-23 · Fusión modelo + orden local (`evaluate.py --dump` + `fuse.py`)
La fusión es lo que haría pistasho en uso real: Laya no ve la posición local (el barajado la quita). RRF (k=10) con peso w al rango del modelo; `lin` = p_modelo + lam · puntaje local. Top-1 / top-3, banco original | sintético, tal cual.

| punto | orig | synth |
|---|---|---|
| orden local | 53 / 76 | 46 / 66 |
| ladder-40000 solo | 44 / 72 | 37 / 65 |
| ladder-40000 rrf mejor w (0.5 en orig, 1 en synth) | 60 / 77 · 55 / 78 | 44 / 70 · 45 / 74 |
| ladder-100000 rrf w=0.75 | 59 / 81 | 45 / 70 |
| ladder-40000 lin lam=4 | 55 / 75 | 47 / 72 |

- **Top-3 sube de forma consistente** (+2 a +8 puntos en los dos bancos y los dos checkpoints).
- **Top-1: el mejor punto del original (57-60 %) no se reproduce en el sintético** (44-47 % frente a 46 %). Con el peso elegido en un banco y aplicado al otro: 44-47 % (synth) y 55-56 % (orig), es decir ±3 del orden local = ruido. No hay mejora robusta de top-1.
- **Complementa, pero poco** (modelo solo vs orden local, top-1, 40k): orig 30 ambos bien, 24 rotas, 14 arregladas, 33 ambos mal; synth 72 / 60 / 34 / 121. Oráculo (acierta el modelo o el local) 67 % / 58 %: ni el techo de combinarlos llega a ≥70 % en el sintético.
- Ojo: con n = 101 el banco original mueve ~1 punto por pregunta; los pesos se barrieron sobre los mismos bancos que se informan.

## Cola
- queue-1.sh (en curso): e3-mix40k (3 épocas, mezcla), gemma40k (Gemma+NL2Bash, 1 época); desglose por fuente de ladder-100000, e3 y gemma40k.
- queue-2.sh (detrás): had40k = 40k solo con filas donde el ejemplo exacto estaba entre los candidatos (sin inyección), todas las fuentes. En el entrenamiento el ejemplo exacto salía recuperado solo en 55 % (gemma-a), 38 % (gemma-b), 53 % (nl2bash), 89-92 % (tldr); el resto se inyectó.
- Cambios de código sin commit: make_data.py (meta con fuente, orden local y `had`), evaluate.py (--dump), nuevos valid_breakdown.py, fuse.py, subset_by_source.py, queue-1/2.sh; docs/training-5090.md (subsample: uniforme, no balanceado).

## 2026-11-23 · Fusión con mitades fijas (peso elegido en una mitad del banco, medido en la otra; media de las dos)
`fuse.py` ahora parte cada banco en preguntas pares / impares. Top-1, fusión vs orden local vs modelo solo:
| checkpoint | banco | rrf fusión | rrf local | lin fusión | lin local | modelo solo |
|---|---|---|---|---|---|---|
| ladder-40000 | orig | 60 % (56, 65) | 53 % | 51 % | 52 % | 44 % |
| ladder-40000 | synth | 42 % | 46 % | 45 % | 46 % | 37 % |
| ladder-40000 casual | orig / synth | 55 % / 46 % | 52 % / 44 % | 51 % / 47 % | 52 % / 45 % | 43 % / 38 % |
| ladder-100000 | orig / synth | 55 % / 45 % | 53 % / 46 % | 51 % / 46 % | 52 % / 46 % | 42 % / 33 % |
| ladder-100000 casual | orig / synth | 59 % / 45 % | 52 % / 44 % | 55 % / 43 % | 51 % / 45 % | 41 % / 34 % |
Las dos mitades del original difieren hasta 17 puntos (48-65 %): el ruido del banco de 101 preguntas es de varios puntos. Con el peso fuera de muestra la fusión queda en ±4 del orden local en todas las filas (synth, 287 preguntas, 42-47 % vs 44-46 %): **no hay mejora de top-1 demostrada**; el top-3 sí sube.

## Palanca nueva: rango local como texto (fmt `tdr`)
`common.py` (formato `tdr`: `#3 herramienta: descripción`, rango real de la búsqueda; las listas ya en orden local, como los bancos, lo calculan solas), `make_data.py` (cada candidato lleva `rank`; el formato `td` sigue idéntico byte a byte, comprobado). Datos en `data-rank/`; `data-rank40k/` = las mismas 40k filas de `had40k` (solo ejemplo exacto recuperado: la inyección daría un rango inventado). Cola: queue-3.sh (detrás de queue-1/2) → rank40k, con `evaluate.py --dump --fmt tdr`. Comparar contra had40k (mismas filas, sin rango). Aviso: en `valid` de `data-rank` las filas inyectadas llevan como rango su posición barajada (basura): no usar su validación para `tdr`, solo los bancos.

| fecha | corrida | qué cambió | filas × épocas | hiperparámetros | top-1 / top-3 en los cuatro bancos | none | tiempo | conclusión |
|---|---|---|---|---|---|---|---|---|
| 2026-11-23 | e3-mix40k | los mismos 40k de ladder-40000, 3 épocas (única diferencia: EPOCHS=3) | 40000x3 | idem | as-written orig 38/70, synth 34/63; casual orig 35/72, synth 34/66 | n/d | 960 s (~125 filas/s); pérdida media por época 0.991 / 0.443 / 0.245 | PEOR que 1 época (44/37 y 43/38): más épocas bajan la pérdida de train y no transfieren (sobreajuste a la distribución de entrenamiento). Palanca 1 (épocas) descartada con esta mezcla |
| 2026-11-23 | gemma40k | 40k filas solo de gen-gemma, gen-gemma-b y nl2bash (`subset_by_source.py`), 1 época | 40000x1 | idem | as-written orig 42/73, synth 36/64; casual orig 38/76, synth 37/65 | n/d | 388 s; pérdida media 1.159 | Igual que la mezcla completa a 1 época (44/37, 43/38) dentro del ruido: quitar tldr NO mejora los bancos. La hipótesis "las preguntas de Gemma son las que transfieren" no se sostiene en este experimento |

### Desglose de validación por fuente (acierto a nivel de herramienta; local = orden local)
| fuente | local | 10k | 40k | 100k | e3-mix40k | gemma40k |
|---|---|---|---|---|---|---|
| gen-gemma | 36 % | 65 | 77 | 80 | 77 | 73 |
| gen-gemma-b | 23 % | 54 | 71 | 76 | 73 | 69 |
| nl2bash | 13 % | 11 | 19 | 29 | 33 | 18 |
| tldr-en | 74 % | 81 | 90 | 92 | 91 | 81 |
| tldr-es | 83 % | 77 | 86 | 87 | 86 | 80 |
| tldr-mix | 72 % | 87 | 94 | 93 | 92 | 86 |
| ALL | 39 % | 62 | 75 | 79 | 77 | 71 |
La validación sigue subiendo con más datos (100k) y con más épocas (e3), y los bancos no se mueven (ver filas anteriores): confirma que no predice la transferencia. NL2Bash (la fuente humana a nivel de herramienta) sube de 11 a 33 % en validación con más datos/épocas y aun así gemma40k, que lo incluye, no mejora los bancos. Con 40 % de "none" dicho en nl2bash, gemma40k se abstiene demasiado en esa fuente.

| fecha | corrida | qué cambió | filas × épocas | hiperparámetros | top-1 / top-3 en los cuatro bancos | none | tiempo | conclusión |
|---|---|---|---|---|---|---|---|---|
| 2026-11-23 | had40k | 40k filas SOLO con el ejemplo exacto ya recuperado (sin inyección), todas las fuentes, 1 época | 40000x1 | idem | as-written orig 45/69, synth 37/64; casual orig 45/70, synth 38/64 | n/d | 382 s; pérdida media 0.551 | Igual que la mezcla (44/37, 43/38) en los bancos: la inyección NO es lo que limita la transferencia. Validación 71 % (peor que 75 % de la mezcla; gen-gemma-b 62 %: es la fuente donde más se inyectaba, 38 % recuperada). Fusión con mitades fijas: orig 53/55 %, synth 44/46 % vs local 53/46 %: ≈ nada. Complemento: orig 35 ambos, 19 rotas, 10 arregladas; synth 74 / 58 / 32; oráculo 63 % / 57 % |
| 2026-11-23 | rank40k | mismas 40k filas que had40k pero con el rango local en el texto de cada opción (fmt `tdr`: `#3 tar: …`) | 40000x1 | idem | as-written orig 52/71, synth 44/67; casual orig 46/70, synth 43/67 | n/d | 410 s; pérdida media 0.530 | **Sube +7/+7 sobre had40k (45/37) y alcanza al orden local (53/46), pero no lo supera.** El original cae 6 puntos de tal cual a casual (52→46): ese banco tiene mucho ruido. Fusión con mitades fijas: orig rrf 57/50, lin 51/46; synth rrf 43/44, lin 47/44 vs local 53/46 y 52/44 → ±3, nada. **Ya no rompe lo que acierta el orden local**: orig 41 ambos, 13 rotas, 12 arregladas (antes 24/14); synth 94 / 38 / 32 (antes 60/34). Oráculo 65 % / 57 %: el techo de combinar sigue por debajo del 70 % en el sintético. Lectura: el modelo aprende a copiar el rango y poco más |
| 2026-11-23 | rank40k-seed1 | rank40k con `--seed 1` (única diferencia) | 40000x1 | idem | as-written orig 47/71, synth 43/66; casual orig 46/70, synth 43/66 | n/d | 412 s; pérdida media 0.629 (semilla 0: 0.530) | **Ruido entre semillas**: orig 52→47 (tal cual) y 46→46 (casual); synth 44→43 y 43→43. Con la misma receta el original se mueve hasta ~5 puntos y el sintético ~1: diferencias menores de ~5 (orig) y ~2 (synth) no son señal. Referencia para leer las demás palancas |
| 2026-11-23 | rank-elr-half | rank40k con `--encoder-lr 1.25e-5` (la mitad del 2.5e-5 por omisión) | 40000x1 | idem | as-written orig 53/71, synth 43/67; casual orig 50/70, synth 42/67 | n/d | 405 s; pérdida media 0.633 | Dentro del ruido de semillas (orig 47-52 / 46; synth 43-44 / 43): bajar el encoder-lr no aporta nada demostrable |
| 2026-11-23 | rank-elr-x2 | rank40k con `--encoder-lr 5e-5` (el doble) | 40000x1 | idem | as-written orig 51/73, synth 41/69; casual orig 50/71, synth 43/70 | n/d | 401 s; pérdida media 0.493 (la más baja con rango) | Top-1 dentro del ruido (orig 46-53, synth 41-44). **Top-3 el más alto hasta ahora (73/69 y 71/70)**, 2-4 puntos sobre las otras corridas con rango (70-71 / 66-67): posible señal, pero una sola corrida; falta confirmar con otra semilla |
| 2026-11-23 | rank-h768 | rank40k con presupuesto de tokens de las opciones 768 (max_len 1024) en vez de 512 | 40000x1 | idem | as-written orig 50/69, synth 43/68; casual orig 48/66, synth 43/67 | n/d | 423 s; pérdida media 0.527 | Idéntico a rank40k dentro del ruido; más tokens no ayudan (con `td` las opciones ya caben: ~11 tokens cada una) y cuestan latencia. Descartado |
| 2026-11-23 | rank-rlcd | rank40k con `--loss rlcd` (en vez de soft-ce) | 40000x1 | idem | as-written orig 51/72, synth 42/68; casual orig 50/73, synth 44/69 | n/d | 405 s; pérdida 0.682 (otra función, no comparable) | Top-1 dentro del ruido; top-3 72/68 y 73/69: de los más altos junto a elr-x2 |
| 2026-11-23 | rank-all | formato `tdr` sobre TODAS las filas con el ejemplo recuperado (96 911), 1 época, receta por omisión | 96911x1 | idem | as-written orig 46/77, synth 39/69; casual orig 47/75, synth 39/68 | n/d | 855 s (~113 filas/s); pérdida media 0.435 | **Más datos con rango NO mejoran el top-1**: peor que rank40k (52/44, 46/43) y por encima del ruido en el sintético (−4/−5). Top-3 sube (77/69 y 75/68). Dos veces y media los datos no ayudan: coherente con la escalera `td` |

## 2026-11-23 · Mantenimiento de disco
El disco llegó a 99 % (3,2 GB libres; los 14 checkpoints pesaban 1,3 GB cada uno). Borrados los `ckpt/` de 11 corridas (smoke, ladder-10000/40000/100000, e3-mix40k, gemma40k, had40k, rank40k-seed1, rank-all, rank-elr-half, rank-h768); sus resultados (`eval*.json`, `eval.txt`, `train.log`, `dump*.json`, `valid-breakdown*`) se conservan. Quedan checkpoints de rank40k, rank-elr-x2 y rank-rlcd. Reproducir una corrida borrada: `training/run.sh train` con su receta y carpeta de datos (data-ladder-*, data-gemma40k, data-meta2→data-had40k, data-rank40k, data-rank-all). 18 GB libres.

## 2026-11-23 · Tanda final (queue-6): desacuerdos, semillas, controles. Cierra el plan (límite fijado por el usuario)
Todo con fmt `tdr`, head 512, 1 época, receta por omisión salvo lo indicado. La columna de minutos de `final_table.py` sale del `train.log` e incluye la evaluación de validación de Laya (~1-2 min), no solo el entrenamiento.

| fecha | corrida | qué cambió | filas × épocas | hiperparámetros | top-1 / top-3 en los cuatro bancos | none | tiempo | conclusión |
|---|---|---|---|---|---|---|---|---|
| 2026-11-23 | dis-s0 | solo filas de DESACUERDO: ejemplo recuperado, el orden local falla en el puesto 1 y la respuesta no es "none" (32 609 filas, todas) | 32609x1 | `--seed 0` | as-written orig 30/51, synth 30/51; casual orig 29/51, synth 32/54 | n/d | 441 s (compartió CPU con la generación de datos) | Esperado: nunca vio filas donde el local acierta y contradice casi siempre. **Fusión con mitades fijas: el barrido elige w=0 (= orden local) en todos los casos**: orig 53/52 %, synth 46/46 % = local. Complemento: orig 16 ambos, 38 rotas, 14 arregladas; synth 36 / 96 / 49 (arregla 49 contra 32 de rank40k, pero rompe 96 contra 38). Oráculo 67 % / 63 % (el más alto hasta ahora en el sintético; rank40k 57 %) |
| 2026-11-23 | bal-s0 | control equilibrado: 20 000 filas de desacuerdo + 20 000 de acuerdo (local acierta, no "none"), todas con ejemplo recuperado | 40000x1 | `--seed 0` | as-written orig 50/73, synth 42/68; casual orig 50/71, synth 42/68 | n/d | 491 s | Igual que rank40k dentro del ruido (orig 47-53, synth 43-44): equilibrar no aporta. Fusión con mitades fijas: orig rrf 53/57, lin 51/48; synth rrf 45/44, lin 45/47 vs local 53/52 y 46/44 → ±3 = nada. Complemento: orig 37 ambos, 17 rotas, 14 arregladas; synth 82 / 50 / 38; oráculo 67 % / 59 % |
| 2026-11-23 | dis-s1 / dis-s2 | igual que dis-s0 con `--seed 1` y `--seed 2` | 32609x1 | idem | s1: orig 35/50, synth 30/51; casual 34/49, 31/52. s2: orig 29/53, synth 29/52; casual 28/52, 31/53 | n/d | 417 s / 409 s | **Tres semillas consistentes**: el modelo solo queda en 28-35 % de top-1 en los cuatro bancos (local 44-53 %). Fusión con mitades fijas: el barrido elige w=0 (orden local) en las tres: orig 53/52 %, synth 46/44 % = local, sin variación entre semillas. Arregla 14-18 (orig) y 48-49 (synth) y rompe 37-39 (orig) y 95-97 (synth); oráculo 67-71 % / 63 %. Línea de desacuerdos: no supera al orden local en ninguna semilla; cerrada |
| 2026-11-23 | rank40k-seed2 | rank40k con `--seed 2` | 40000x1 | idem | as-written orig 49/74, synth 41/69; casual orig 46/72, synth 42/68 | n/d | 408 s | Tercera semilla de rank40k. Tres semillas (orig / synth, tal cual): 52, 47, 49 / 44, 43, 41; casual: 46, 46, 46 / 43, 43, 42. Contra el local (53 / 46 y 52 / 44): −1…−6 y −2…−5 tal cual; −6 y −1…−2 casual. Ninguna semilla lo supera en tal cual; en el sintético casual queda a −2…−1 |
| 2026-11-23 | rank-freeze | rank40k con `--freeze-encoder` (solo se entrena la cabeza) | 40000x1 | idem | as-written orig 41/61, synth 32/57; casual orig 41/60, synth 33/57 | n/d | 224 s; pérdida media 2.290 | Control: la cabeza sola queda 10-12 puntos por debajo del modelo completo (rank40k: 47-52 / 41-44), así que el codificador SÍ aporta; pero la cabeza sola ya es casi el modelo cero-shot (27/22) más algo de uso del rango. Fusión: orig 54, synth 45 = local |
| 2026-11-23 | rank-n20 | rank40k con `--none-share 0.20` (20 % de filas "none" en vez de 10 %), 40k filas con ejemplo recuperado | 40000x1 | idem | as-written orig 54/72, synth 44/67; casual orig 53/69, synth 46/67 | **0 %** | 408 s | El mejor top-1 sin fusión de toda la sesión (orig 54/53, synth 44/46: = al orden local 53/52 y 46/44), pero es UNA semilla y cae dentro del ruido de rank40k (orig 47-52, synth 41-44). **A pesar de 20 % de "none" en el entrenamiento, dice "none" el 0 % de las veces en los cuatro bancos**: el problema de "none" no se resuelve con none-share. Fusión: orig 57, synth 45 vs local 53 y 46 |
| 2026-11-23 | rank-aug | rank40k con 40 % de las preguntas acortadas (2-3 palabras clave) o con una errata (`--augment 0.4`), recuperación nueva de los 60 914 textos, 40k filas con ejemplo recuperado | 40000x1 | idem | as-written orig 52/69, synth 41/66; casual orig 49/68, synth 42/67 | 0 % | 405 s | Igual que rank40k dentro del ruido (orig 47-52, synth 41-44): preguntas cortas y erratas no mueven los bancos. Fusión: orig 56, synth 46 vs local 53 y 46 |

## 2026-11-23 · CIERRE: «agotado» (§0.5 punto 2), con el límite de tanda fijado por el usuario
Tabla final con costo (filas, épocas, minutos; incluye la evaluación de validación de Laya) en `training/out/FINAL_TABLE.md`, generada por `training/final_table.py`. Copia de los resultados de todas las corridas (eval*.json, eval.txt, train.log, dump*.json, valid-breakdown*) en `training/out/final-results/`.

**Resultado.** Ninguna variante pasa la puerta de decisión (§6): el mejor top-1 es 54/53 % (orig) y 44/46 % (synth) en una sola semilla (`rank-n20`), frente a ≥ 70 % exigido; Jev da 83 / 76. El modelo con el rango local en el texto (`tdr`) solo IGUALA al orden local (53/46 y 52/44), no lo supera: tres semillas de rank40k quedan entre −7 y −1 puntos del orden local en los cuatro bancos. Con pesos elegidos en una mitad del banco y medidos en la otra, la fusión modelo + orden local queda en ±3 puntos del orden local en TODAS las corridas; el top-3 del modelo alcanza al local pero no lo supera de forma consistente. Ninguna variante cumple el criterio de éxito (superar al orden local de forma consistente en las tres semillas).

**Qué se probó (todo con diario):** cantidad de datos (10k a 100k, con y sin rango), épocas (3: peor), composición (solo Gemma+NL2Bash, solo ejemplo recuperado, equilibrado desacuerdo/acuerdo), rango local como texto (única palanca que movió el top-1: +5..+8), tasa de aprendizaje (×0.5, ×2), presupuesto de tokens (768), pérdida rlcd, `--freeze-encoder` (control: el codificador sí aporta, −10..−12 puntos sin él), solo desacuerdos (3 semillas: 28-35 % solo, la fusión elige el orden local), none-share 0.20, preguntas cortas y erratas, semillas (ruido: ~5 puntos en orig, ~2 en synth). Sin probar por el límite de tanda: currículo y calibración de "none".

**Hallazgos que quedan:**
1. La validación (herramientas no vistas, mismas fuentes) sube de 60 a 79 % con más datos y épocas mientras los bancos no se mueven: no mide transferencia; mandan los bancos.
2. Más datos no ayudan a partir de ~40k (la escalera es plana; con rango, 97k es peor en top-1).
3. El modelo solo contradice con provecho al orden local en preguntas que este falla, pero rompe más de lo que arregla (techo de combinar: 57-71 %, ninguna fusión lo alcanza).
4. "none" no se resuelve con none-share: con 20 % de "none" en el entrenamiento dice "none" el 0 % de las veces en los bancos.
5. Pista no confirmada: `rank-n20` iguala al orden local con una semilla (54/44/53/46 contra 53/46/52/44); con tres semillas haría falta para afirmar algo. No se corrió por el límite de tanda.

**Recomendación (la del usuario, anotada):** no integrar el modelo local (no justifica ~1 GB de RAM más un daemon), dejar Jev como opción para quien quiera la mejora (83 / 76) y conservar la tubería de datos, el kit, el diario y la medida de que más datos no ayudan.

## Tabla final con costo

La columna de minutos sale de `train.log` e incluye la evaluación de validación de Laya (~1-2 min por corrida), no solo el entrenamiento.
Generada con `python3 training/final_table.py training/out`.

| run | filas | épocas | min | orig | synth | orig casual | synth casual | none | fusión rrf orig/synth | fusión lin orig/synth |
|---|---|---|---|---|---|---|---|---|---|---|
| bal-s0 | 40000 | 1 | 8.2 | 50 / 73 | 42 / 68 | 50 / 71 | 42 / 68 | 0 % | 53 / 45 | 51 / 45 |
| dis-s0 | 32609 | 1 | 7.3 | 30 / 51 | 30 / 51 | 29 / 51 | 32 / 54 | 0 % | 53 / 46 | 52 / 46 |
| dis-s1 | 32609 | 1 | 7.0 | 35 / 50 | 30 / 51 | 34 / 49 | 31 / 52 | 0 % | 53 / 46 | 52 / 46 |
| dis-s2 | 32609 | 1 | 6.8 | 29 / 53 | 29 / 52 | 28 / 52 | 31 / 53 | 0 % | 53 / 46 | 52 / 42 |
| e3-mix40k | 40000 | 3 | 16.0 | 38 / 70 | 34 / 63 | 35 / 72 | 34 / 66 | 18 % | - | - |
| gemma40k | 40000 | 1 | 6.5 | 42 / 73 | 36 / 64 | 38 / 76 | 37 / 65 | 10 % | - | - |
| had40k | 40000 | 1 | 6.4 | 45 / 69 | 37 / 64 | 45 / 70 | 38 / 64 | 0 % | 53 / 44 | 55 / 46 |
| ladder-10000 | 10000 | 1 | 2.6 | 39 / 67 | 34 / 62 | 39 / 67 | 36 / 62 | 15 % | 52 / 42 | 50 / 47 |
| ladder-100000 | 100000 | 1 | 15.3 | 42 / 71 | 33 / 64 | 41 / 71 | 34 / 65 | 9 % | 55 / 45 | 51 / 46 |
| ladder-40000 | 40000 | 1 | 6.5 | 44 / 72 | 37 / 65 | 43 / 73 | 38 / 66 | 3 % | 60 / 42 | 51 / 45 |
| rank-all | 96911 | 1 | 14.2 | 46 / 77 | 39 / 69 | 47 / 75 | 39 / 68 | 0 % | 60 / 46 | 58 / 44 |
| rank-aug | 40000 | 1 | 6.8 | 52 / 69 | 41 / 66 | 49 / 68 | 42 / 67 | 0 % | 56 / 46 | 53 / 48 |
| rank-elr-half | 40000 | 1 | 6.8 | 53 / 71 | 43 / 67 | 50 / 70 | 42 / 67 | 0 % | 58 / 47 | 57 / 48 |
| rank-elr-x2 | 40000 | 1 | 6.7 | 51 / 73 | 41 / 69 | 50 / 71 | 43 / 70 | 0 % | 54 / 44 | 53 / 44 |
| rank-freeze | 40000 | 1 | 3.7 | 41 / 61 | 32 / 57 | 41 / 60 | 33 / 57 | 31 % | 54 / 45 | 57 / 46 |
| rank-h768 | 40000 | 1 | 7.0 | 50 / 69 | 43 / 68 | 48 / 66 | 43 / 67 | 0 % | 55 / 44 | 52 / 48 |
| rank-n20 | 40000 | 1 | 6.8 | 54 / 72 | 44 / 67 | 53 / 69 | 46 / 67 | 0 % | 57 / 45 | 58 / 47 |
| rank-rlcd | 40000 | 1 | 6.8 | 51 / 72 | 42 / 68 | 50 / 73 | 44 / 69 | 5 % | 58 / 46 | 57 / 45 |
| rank40k | 40000 | 1 | 6.8 | 52 / 71 | 44 / 67 | 46 / 70 | 43 / 67 | 0 % | 57 / 43 | 51 / 47 |
| rank40k-seed1 | 40000 | 1 | 6.9 | 47 / 71 | 43 / 66 | 46 / 70 | 43 / 66 | 12 % | 60 / 45 | 55 / 47 |
| rank40k-seed2 | 40000 | 1 | 6.8 | 49 / 74 | 41 / 69 | 46 / 72 | 42 / 68 | 0 % | 57 / 45 | 55 / 46 |
| smoke | 200 | 1 | 1.4 | 28 / 53 | 27 / 50 | 23 / 53 | 25 / 51 | 9 % | - | - |
| **orden local** | - | - | - | 53 / 76 | 46 / 66 | 52 / 77 | 44 / 67 | - | - | - |
| **Jev (referencia)** | - | - | - | 83 / 92 | 76 / 88 | - | - | - | - | - |

### Grupos de semillas: top-1 del modelo menos top-1 del orden local, en puntos (mín … máx entre semillas)

| grupo | orig | synth | orig casual | synth casual |
|---|---|---|---|---|
| rank40k (3 seeds): rank40k, rank40k-seed1, rank40k-seed2 | -7 … -1 | -5 … -2 | -7 … -7 | -2 … -1 |
| disagree (3 seeds): dis-s0, dis-s1, dis-s2 | -25 … -19 | -17 … -16 | -25 … -19 | -14 … -13 |
