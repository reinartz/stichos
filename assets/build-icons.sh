#!/usr/bin/env bash
# stichos icon set. Requires: librsvg (via ImageMagick) or rsvg-convert, ImageMagick.
set -euo pipefail
OUT=${1:-dist/icons}; mkdir -p "$OUT"
# >=64px: full detail. 16-48px: simplified (no text bars, fat tick).
for s in 64 128 256 512 1024; do magick -background none icon-app.svg   -resize ${s}x${s} -flatten "$OUT/stichos-${s}.png"; done
for s in 16 24 32 48;     do magick -background none icon-tight.svg -resize ${s}x${s} -flatten "$OUT/stichos-${s}.png"; done
magick "$OUT/stichos-16.png" "$OUT/stichos-24.png" "$OUT/stichos-32.png" \
       "$OUT/stichos-48.png" "$OUT/stichos-64.png" "$OUT/stichos-128.png" \
       "$OUT/stichos-256.png" -colors 256 "$OUT/stichos.ico"
magick -background none icon-app.svg -resize 180x180 -flatten "$OUT/apple-touch-icon.png"
magick -background none icon-app.svg -resize 32x32  -flatten "$OUT/favicon-32.png"
magick -background none icon-tight.svg -resize 192x192 -flatten "$OUT/android-192.png"
magick -background none icon-tight.svg -resize 512x512 -flatten "$OUT/android-512.png"
# macOS: copy this folder to a Mac and run:  iconutil -c icns stichos.iconset
mkdir -p "$OUT/stichos.iconset"
for s in 16 32 64 128 256 512; do
  cp "$OUT/stichos-$s.png" "$OUT/stichos.iconset/icon_${s}x${s}.png"
  d=$((s*2)); [ -f "$OUT/stichos-$d.png" ] && cp "$OUT/stichos-$d.png" "$OUT/stichos.iconset/icon_${s}x${s}@2x.png"
done
cp icon-app.svg "$OUT/stichos.svg"; cp icon-glyph.svg "$OUT/stichos-glyph.svg"
echo "wrote $OUT"
