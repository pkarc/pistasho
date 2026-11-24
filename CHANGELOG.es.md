# Registro de cambios

[English](CHANGELOG.md) · [Español](CHANGELOG.es.md)

Todavía no hay ninguna versión publicada. Esto es lo que traerá la primera, `0.1.0`.

## [0.1.0] — sin publicar

### Agregado
- **Modo pregunta.** `Ctrl-Space` en zsh (o `prefix + g` en un popup de tmux, también por ssh) convierte una pregunta en inglés o español en un comando en tu prompt. Nunca se ejecuta solo.
- **Alternativas y explorar.** Un segundo `Ctrl-Space` sobre la respuesta sin editar lista otras opciones para la misma pregunta; `Tab` con la pregunta vacía explora tus atajos, historial y tldr.
- **Búsqueda.** Páginas de tldr (inglés + español), tu `cheats.toml` y tu historial de zsh, ordenados con BM25 sobre un índice SQLite FTS5, una tabla de sinónimos en español, un ejemplo de tldr por herramienta antes de repetir, y un modelo de embeddings multilingüe opcional (`multilingual-e5-small`) fusionado por rangos.
- **Comprobaciones de cada comando:** programas no instalados (y una oferta de instalarlos con una `y` explícita y el comando exacto a la vista), una tabla curada comando → paquete para apt, pacman, dnf y zypper, alternativas hechas con herramientas base, reglas fijas de riesgo con caja roja de confirmación, y un selector que solo ofrece dispositivos de bloque extraíbles.
- **LLM de respaldo (opcional, apagado por defecto).** Anthropic, cuando nada local coincide o la mejor coincidencia es dudosa; solo envía la pregunta, el nombre del SO, el gestor de paquetes y el shell; las respuestas se guardan en caché (`pistasho cache`).
- **Modelo de decisión (opcional, apagado por defecto).** Jev (TypeSafe, alojado) reordena los mejores candidatos de tldr y avisa cuando ninguno sirve (top-1 52% → 83% en 101 preguntas, 48% → 76% en 287 generadas), y suma una segunda opinión de riesgo junto a las reglas fijas (21 de 29 comandos destructivos cazados en vez de 17, sin falsas alarmas en la muestra). Solo se envían tu pregunta y texto público de tldr, nunca tus comandos propios, historial ni valores rellenados.
- `pistasho risk <comando>` y `pistasho eval --decide`.
- `pistasho eval` con top-1, top-3, recall@10 y una tabla de confianza; un banco de 101 preguntas reales y 287 generadas.
- `pistasho doctor`, `pistasho config` (asistente), interfaz en inglés y español.

### Medido
- Sobre 101 preguntas, el top-1 / top-3 pasó de 28% / 41% (búsqueda difusa de la fase 1) a 51% / 74% con la búsqueda híbrida. Ver `docs/eval.md` para cada medición detrás de cada decisión.

### Decidido no construir
- Un modelo local de decisión (Laya) y un LLM local (Ollama): exigen un servicio residente y cientos de MB de RAM para una sola herramienta, y los rerankers generales probados no ayudaron. Se ajustó Laya con 23 variantes en una RTX 5090 y ninguna superó al orden local (mejor top-1 54% / 44% contra 53% / 46%; ver `docs/eval.md`). Solo se admiten proveedores alojados.
