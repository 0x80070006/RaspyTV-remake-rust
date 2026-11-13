#!/bin/sh
# Prépare dist/raspytv-rs-aarch64.tar.gz : binaires du Pi, web/ (avec l'accueil WASM) et installation.
set -eu
cd "$(dirname "$0")/.."
sh scripts/build-ui.sh
docker build -t raspytv-cross packaging/cross
sh scripts/cross-check.sh
OUT=dist/raspytv-rs
rm -rf "$OUT" && mkdir -p "$OUT"
cp dist/aarch64/raspytv dist/aarch64/raspytv-kiosk "$OUT/"
cp -r web "$OUT/web"
rm -rf "$OUT/web/assets/art"/* "$OUT/web/assets/backgrounds"/*
cp scripts/install.sh site.example.json packaging/raspytv-rs.service packaging/labwc-autostart "$OUT/"
tar -C dist -czf dist/raspytv-rs-aarch64.tar.gz raspytv-rs
echo "dist/raspytv-rs-aarch64.tar.gz prêt"
