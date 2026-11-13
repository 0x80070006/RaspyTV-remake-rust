#!/bin/sh
# Compile l'accueil (crates/ui) en WebAssembly vers web/pkg.
# Prérequis : rustup target add wasm32-unknown-unknown
#             cargo install wasm-bindgen-cli --version 0.2.130
set -eu
cd "$(dirname "$0")/.."
# Chemins de compilation retirés du binaire (ils contiennent le nom de l'utilisateur).
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="--remap-path-prefix=$CARGO_HOME_DIR=/cargo --remap-path-prefix=$PWD=/src ${RUSTFLAGS:-}"
cargo build -p raspytv-ui --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/raspytv_ui.wasm
echo "Accueil compilé dans web/pkg"
