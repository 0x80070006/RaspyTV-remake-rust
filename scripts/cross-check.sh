#!/bin/sh
# Clippy + compilation aarch64 dans l'image Docker (packaging/cross).
set -eu
cd "$(dirname "$0")/.."
SRC="$PWD"
command -v cygpath >/dev/null 2>&1 && SRC="$(cygpath -w "$PWD")"
MSYS_NO_PATHCONV=1 docker run --rm -v "$SRC:/src" -v raspytv-target:/build -v raspytv-cargo:/opt/cargo/registry raspytv-cross
