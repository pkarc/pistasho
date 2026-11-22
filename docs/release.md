# Cómo se publica una versión

> Pasos para el mantenedor. Nada de esto se ejecuta solo: etiquetar y publicar son decisiones de
> una persona. (Este documento va solo en español, como el resto de las notas de proyecto.)

Lo que ya está preparado, **sin probar de punta a punta** porque necesita una etiqueta o una cuenta:

| Pieza | Archivo | Falta probar |
|---|---|---|
| Binarios de GitHub (x86_64 y aarch64) | `.github/workflows/release.yml` | Que corra con una etiqueta `v*`; el runner `ubuntu-24.04-arm` y la descarga estática de onnxruntime para aarch64 |
| Paquete AUR | `packaging/aur/PKGBUILD` | `updpkgsums` con el tarball de la etiqueta y `makepkg` en un contenedor de Arch limpio |
| `cargo install pistasho` | `Cargo.toml` (`exclude`, metadatos) | `cargo publish` real; `cargo publish --dry-run` sí se probó |

Se escribió el flujo de releases a mano en vez de `cargo-dist` (no estaba instalado); si se prefiere
`cargo-dist`, `dist init` lo reemplaza sin tocar el código.

## Antes de etiquetar

1. `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
2. `cargo build --release && PISTASHO_BIN=$PWD/target/release/pistasho python3 scripts/e2e.py`
3. Subir la versión en `Cargo.toml`, `packaging/aur/PKGBUILD` (`pkgver`) y `CHANGELOG.md` / `CHANGELOG.es.md`.
4. `cargo publish --dry-run`

## Publicar

1. `git tag vX.Y.Z && git push origin vX.Y.Z` (el workflow `release` crea el release con los binarios).
2. `cargo publish`.
3. AUR: en un clon de `ssh://aur@aur.archlinux.org/pistasho.git`, copiar `PKGBUILD`, `updpkgsums`, `makepkg --printsrcinfo > .SRCINFO`, probar en un contenedor de Arch y subir.

## Notas

- El binario pesa ~32 MB por llevar onnxruntime estático. El modelo de embeddings (135 MB) no va dentro:
  `pistasho update` lo baja con su suma de verificación.
- `cargo install` necesita red al compilar: el crate `ort` descarga onnxruntime.
