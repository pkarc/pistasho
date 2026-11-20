# ADR 0001 · El modo pregunta es la única entrada; el buscador pasa a ser la lista de alternativas

- **Estado:** Aceptada
- **Fecha:** 2026-11-19
- **Referencia visual:** `docs/pistasho-modo-pregunta.gif` (simulado, no es captura real)
- **Reemplaza:** el diseño de dos atajos (`Ctrl-G` buscador + `Ctrl-Space` pregunta) descrito en `CLAUDE.md` y `PLAN.md`

## Contexto

El diseño original tenía dos entradas:
- `Ctrl-G` abría un buscador difuso (fase 1, ya implementado en `tui.rs` / `search.rs`).
- `Ctrl-Space` activaba el modo pregunta en lenguaje natural (fase 2).

Problemas:
- Dos atajos que recordar para tareas casi iguales.
- Si el modo pregunta no acertaba, había que salir, abrir el buscador y volver a escribir.
- El usuario quiere que la herramienta sea, ante todo, **rápida**: preguntar y tener el comando.

## Decisión

**`Ctrl-Space` es la única entrada.** La interfaz del buscador se conserva, pero solo como **lista de candidatos** a la que se llega desde el modo pregunta. `Ctrl-G` deja de existir como función propia.

### Flujo

**1. Respuesta directa (caso normal).**
`Ctrl-Space` → el prompt cambia a ` ✦ pregunta ` (pista a la derecha: `⏎ preguntar · tab explorar · esc salir`) → escribes → Enter → spinner `resolviendo…` → el mejor comando queda en `BUFFER` con el prompt normal. Debajo, con `zle -M`:

```
  ⚡ local · tldr/fc · Ctrl-Space: ver 5 más
```

**2. Alternativas (segundo `Ctrl-Space`).**
Si el usuario presiona `Ctrl-Space` y el `BUFFER` sigue siendo exactamente el comando recién insertado, se abre la lista de candidatos **para la misma pregunta**, compacta y debajo del prompt (no en pantalla completa):
- Título: `otras opciones · «<pregunta>»`.
- Primera fila: campo de filtro `❯`. Escribir filtra la lista en vivo (búsqueda difusa actual).
- Filas: etiqueta (`tldr` / `mío` / `hist`) + comando con `{{marcadores}}` resaltados. El comando actual aparece en la lista.
- `↑↓` mover, `Enter` reemplaza el comando (pidiendo marcadores si los hay), `Esc` deja el actual sin cambios.
- Al reemplazar, debajo del prompt: `✓ reemplazado · <origen>`.

Si el usuario editó el `BUFFER`, o ya ejecutó el comando, `Ctrl-Space` vuelve a ser una pregunta nueva.

**3. Explorar sin pregunta.**
En el modo pregunta, `Tab` con el campo vacío abre la misma lista en modo `explorar · tus atajos, historial y tldr`: sin pregunta, ordenada como `empty_query_order` (mío → historial reciente → tldr), con filtro difuso al escribir. Es el antiguo buscador. `Enter` elige (con relleno de marcadores), `Tab` inserta la plantilla sin rellenar, `Esc` sale.

**4. tmux.**
El popup (`prefix + g`) abre el modo pregunta, con `Tab` para explorar igual que en zsh. El resultado se envía con `tmux send-keys -l`. Como no hay un `BUFFER` previo, el popup no tiene el paso 2: tras resolver muestra el resultado con `⏎ enviar · ↑↓ ver más`.

### Detalles técnicos sugeridos

- **Estado en el widget:** variables de shell `_PISTASHO_LAST_Q` y `_PISTASHO_LAST_CMD`. Al presionar `Ctrl-Space`: si `$BUFFER == $_PISTASHO_LAST_CMD` y no está vacío, se abren las alternativas; si no, pregunta nueva. Limpiar ambas en `zle-line-finish` (o con `add-zle-hook-widget`) para que no sobrevivan a un Enter.
- **CLI:**
  - `pistasho ask "<pregunta>"`: imprime el mejor comando.
  - `pistasho ask --alternatives --current "<cmd>" "<pregunta>"`: abre la lista y imprime el elegido (código 130 si se cancela).
  - `pistasho ask --explore`: abre la lista en modo explorar.
  - `pistasho search` se queda como subcomando para scripts y pruebas, pero sin atajo propio.
- **Metadatos para la pista de `zle -M`** (origen y cuántas alternativas hay): el comando puede ser multilínea (historial), así que conviene separar campos con NUL (`--format zsh`, y en zsh `parts=("${(@0)out}")`) o escribir los metadatos en otro descriptor. Que lo decida la implementación.
- **Interfaz compacta:** la lista va debajo del prompt (`Viewport::Inline` de ratatui), no en pantalla alternativa. Esto adelanta lo que era la tarea 5.1.
- **`tui.rs`** se refactoriza de "buscador" a un componente de **lista de candidatos** con dos modos (`Alternatives { question, current }`, `Explore`) más el modo de relleno de marcadores que ya existe.
- `search.rs`, `placeholders.rs`, `index.rs`, `eval` y el relleno de marcadores **no cambian de comportamiento**.

## Alternativas consideradas

1. **Mantener los dos atajos.** Descartada: doble entrada, y en el caso de fallo hay que volver a escribir.
2. **Quitar el buscador por completo.** Descartada: sin plan B cuando la primera respuesta falla, y se pierde la exploración del cheatsheet y el historial. El código ya existe.
3. **Mostrar siempre los candidatos antes de insertar.** Descartada: suma una tecla en el caso normal, que es el más frecuente, y va contra el objetivo de velocidad.

## Consecuencias

- Una sola tecla que recordar. El caso normal sigue costando lo mismo: pregunta + Enter.
- Corregir una respuesta cuesta una tecla y no obliga a reescribir nada.
- `ask` debe devolver los **N mejores candidatos**, no solo el primero (afecta a la tarea 2.2).
- La interfaz compacta pasa de la fase 5 a la fase 2.
- Se pierde abrir el buscador directo con texto en el prompt (`tar` + `Ctrl-G`). Se compensa porque preguntar solo `tar` también funciona: BM25 y la búsqueda difusa encuentran el nombre de la herramienta.
