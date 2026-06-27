#!/usr/bin/env bash
# Build the WebGPU/WASM bundle for our-space into web/dist/.
#
# Prereqs (one-time):
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.125   # match the wasm-bindgen crate
#   brew install binaryen                              # provides wasm-opt
#
# Then:  ./web/build.sh   &&   python3 -m http.server -d web/dist 8080
# Open http://localhost:8080 in a WebGPU browser (localhost is a secure context).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=web/dist
WASM=target/wasm32-unknown-unknown/wasm-release/app-interactive.wasm

echo "==> cargo build (wasm-release, WebGPU)"
cargo build -p app-interactive --profile wasm-release --target wasm32-unknown-unknown

echo "==> wasm-bindgen"
mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" --out-name app-interactive "$WASM"

echo "==> wasm-opt -Oz"
# Enable the wasm features the Rust toolchain emits (bulk-memory, etc.).
wasm-opt -Oz \
  --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals --enable-reference-types --enable-multivalue \
  -o "$OUT/app-interactive_bg.opt.wasm" "$OUT/app-interactive_bg.wasm"
mv "$OUT/app-interactive_bg.opt.wasm" "$OUT/app-interactive_bg.wasm"

echo "==> copy page + assets"
cp web/index.html "$OUT/index.html"
rm -rf "$OUT/assets"
cp -r crates/app-interactive/assets "$OUT/assets"
# Serve the bundle verbatim on GitHub Pages (no Jekyll processing).
touch "$OUT/.nojekyll"

echo "==> further-reading cards (web/content/reading/*.md -> reading.json)"
if command -v python3 >/dev/null 2>&1; then
  python3 tools/build_reading.py
else
  echo "   (python3 not found — skipping reading.json; the panel hides gracefully)"
fi

echo "==> done: $(du -sh "$OUT" | cut -f1) in $OUT"
ls -lah "$OUT"/*.wasm
echo "Serve:  python3 -m http.server -d $OUT 8080   (then open http://localhost:8080)"
