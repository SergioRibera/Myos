# Myos — Guía de uso del logo

## 1. El logo
- **Idea:** la *y* de myos es una persona que se estira hacia arriba (brazos en V, cabeza lima). El esfuerzo de cada día, hecho letra.
- **Versiones** (`svg/` y `png/`):
  - **Wordmark** (principal): `myos-wordmark-*` — la *y* lleva la cabeza.
  - **Horizontal**: `myos-horizontal-*` — símbolo + nombre (la *y* del nombre va sin cabeza).
  - **Apilado**: `myos-stacked-*` — splash, perfiles, piezas cuadradas.
  - **Símbolo**: `myos-symbol-*` — ícono, avatar, sello.
  - **Símbolo chico**: `myos-symbol-small-*` — por debajo de 32 px (trazo más grueso, cabeza más separada).
- **Esquemas:** `color` (sobre claro) · `reversed` (sobre carbón/oscuro) · `carbon` (una tinta) · `lime` (una tinta, solo sobre oscuro) · `black` · `white`.

## 2. Regla del lima
**El lima nunca es fondo.** Solo colorea la marca (la cabeza, o el logo entero en la versión `lime`) y siempre sobre carbón u otro fondo oscuro. Fondos permitidos: carbón, tiza, blanco, fotos oscuras.

## 3. Espacio de respeto
Alrededor del logo, dejar libre **1 × el diámetro de la cabeza** en todos los lados. Escala con el logo.

## 4. Tamaño mínimo
| Versión | Pantalla | Impresión |
|---|---|---|
| Wordmark | 80 px de ancho | 20 mm |
| Horizontal | 120 px de ancho | 30 mm |
| Símbolo | 32 px | 8 mm |
| Símbolo chico | 16 px | 5 mm |

## 5. Color
| Nombre | HEX | RGB | CMYK (aprox.) | Pantone (aprox., verificar con guía física) |
|---|---|---|---|---|
| Carbón | `#15171C` | 21 23 28 | 25 18 0 89 | Black 6 C |
| Lima | `#C6F432` | 198 244 50 | 19 0 80 4 | 374 C / 2290 C |
| Lima profundo (cabeza sobre claro) | `#86B300` | 134 179 0 | 25 0 100 30 | 376 C |
| Tiza | `#F4F5F0` | 244 245 240 | 0 0 2 4 | — |

**Combinaciones aprobadas:** `color` sobre tiza o blanco · `reversed` o `lime` sobre carbón · `white` sobre fotos oscuras · `black`/`carbon` para una tinta.

**Contraste:** carbón sobre tiza ≈ 16:1 · lima sobre carbón ≈ 14:1. La cabeza lima profundo sobre claro (≈ 2.3:1) es decorativa: la *y* se lee igual sin ella. No usar lima para texto sobre claro.

## 6. Tipografía
- El wordmark es dibujado (no es una fuente): no reescribirlo tipeando.
- Para UI y textos se sugiere una sans redondeada con licencia abierta: **Nunito** (OFL) o **Figtree** (OFL). Fallback: `system-ui`.

## 7. No hacer
No estirar ni deformar · no rotar · no usar lima de fondo · no cambiar colores fuera de la paleta · no separar ni mover la cabeza · no agregar sombras, contornos ni degradados · no poner sobre fondos cargados sin contenedor · no usar el símbolo normal por debajo de 32 px.

## 8. Íconos
- `app-icon/icon-1024.png` — iOS (a sangre) + mipmaps legacy de Android (`[app.icon] image`).
- `app-icon/icon-tile-1024.png` — escritorio, ventana / `.exe` (`[app.icon] desktop`).
- `app-icon/android/{foreground,monochrome}.png` — adaptive icon 108 dp; fondo `#15171C` (`[app.icon.android]`).
- `web/` — favicon.ico/svg, apple-touch-icon, icon-192/512, maskable-512, `site.webmanifest`, `head-snippet.html`.

## 9. Regenerar
`tools/build-kit.sh` reconstruye todo desde `tools/kit.py` (masters con trazos → contornos con Inkscape → PNG):
`nix shell nixpkgs#python3 nixpkgs#inkscape nixpkgs#librsvg -c brand/tools/build-kit.sh brand`

## 10. Notas
Marca no verificada legalmente: hacer búsqueda profesional de marca registrada antes de lanzar.
