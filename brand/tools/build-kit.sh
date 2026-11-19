#!/usr/bin/env bash
# Rebuild the Myos brand kit: stroke masters -> outlined SVG -> PNG / icons.
# Needs python3, inkscape, rsvg-convert, e.g.:
#   nix shell nixpkgs#python3 nixpkgs#inkscape nixpkgs#librsvg -c ./build-kit.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-$here/../}
CARBON="#15171C"; CHALK="#F4F5F0"
python3 "$here/kit.py" "$out"
cd "$out"
mkdir -p svg png web app-icon/android
while read -r name mode; do
  area=(); [ "$mode" = drawing ] && area=(--export-area-drawing)
  inkscape "src/$name.svg" --actions="select-all:all;object-stroke-to-path" "${area[@]}" \
    --export-plain-svg --export-type=svg --export-filename="svg/$name.svg" 2>/dev/null
done < src/modes.txt
python3 "$here/clean.py" svg/*.svg
for f in svg/myos-*.svg; do n=$(basename "$f" .svg)
  case $n in *symbol*) sizes="512 1024";; *) sizes="600 1200";; esac
  for w in $sizes; do rsvg-convert -w "$w" "$f" -o "png/$n-$w.png"; done
done
# App icons (consumed by istmo.toml [app.icon])
rsvg-convert -w 1024 svg/app-icon-ios-1024.svg -o app-icon/icon-1024.png
rsvg-convert -w 1024 svg/app-icon-tile.svg     -o app-icon/icon-tile-1024.png
for l in foreground monochrome; do
  rsvg-convert -w 432 "svg/android-adaptive-$l.svg" -o "app-icon/android/$l.png"
done
# Web
cp svg/favicon.svg web/favicon.svg
for s in 16 32 48; do rsvg-convert -w $s svg/favicon.svg -o "web/favicon-$s.png"; done
rsvg-convert -w 180 svg/app-icon-ios-1024.svg -o web/apple-touch-icon.png
for s in 192 512; do rsvg-convert -w $s svg/app-icon-tile.svg -o "web/icon-$s.png"; done
rsvg-convert -w 512 svg/maskable-512.svg -o web/maskable-512.png
python3 "$here/ico.py" web/favicon.ico web/favicon-16.png web/favicon-32.png web/favicon-48.png
cat > web/site.webmanifest <<X
{
  "name": "Myos",
  "short_name": "Myos",
  "icons": [
    { "src": "/icon-192.png", "sizes": "192x192", "type": "image/png" },
    { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png" },
    { "src": "/maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ],
  "theme_color": "$CARBON",
  "background_color": "$CHALK",
  "display": "standalone"
}
X
cat > web/head-snippet.html <<X
<link rel="icon" href="/favicon.ico" sizes="48x48">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="manifest" href="/site.webmanifest">
<meta name="theme-color" content="$CARBON">
X
echo "kit built in $out"
