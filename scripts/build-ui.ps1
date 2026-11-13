# Compile l'accueil (crates/ui) en WebAssembly vers web/pkg (Windows).
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
# Chemins de compilation retirés du binaire (ils contiennent le nom de l'utilisateur).
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
$env:RUSTFLAGS = "--remap-path-prefix=$cargoHome=/cargo --remap-path-prefix=$((Get-Location).Path)=/src"
cargo build -p raspytv-ui --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir web/pkg target/wasm32-unknown-unknown/release/raspytv_ui.wasm
Write-Output 'Accueil compilé dans web/pkg'
