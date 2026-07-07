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

echo "==> compress baked layers (brotli, OSZ1 magic)"
# GitHub Pages / Fastly does NOT gzip or brotli `application/octet-stream`, so the big
# postcard layers would otherwise download at full size. Ship them pre-compressed and
# decompress in the loader (crates/app-interactive/src/loading.rs); a 4-byte `OSZ1`
# magic prefix marks a compressed file so the loader tells it from a raw asset.
# Decompressed bytes are byte-identical → no layer/statistic changes. Only the native
# `crates/app-interactive/assets/` copies stay raw (native dev reads those directly).
# Skipped (with a warning) if the `brotli` CLI is absent; the loader still reads raw.
if command -v brotli >/dev/null 2>&1; then
  n=0; saved=0
  for f in "$OUT"/assets/processed/*; do
    [ -f "$f" ] || continue
    raw=$(wc -c < "$f")
    [ "$raw" -gt 65536 ] || continue   # tiny layers: compression overhead not worth it
    tmp="$f.osz"
    { printf 'OSZ1'; brotli -q 11 -c "$f"; } > "$tmp"
    new=$(wc -c < "$tmp")
    if [ "$new" -lt "$raw" ]; then
      mv "$tmp" "$f"; n=$((n + 1)); saved=$((saved + raw - new))
    else
      rm -f "$tmp"   # incompressible (already tight) → keep the raw file
    fi
  done
  echo "   ✓ compressed $n layers, saved $((saved / 1024 / 1024)) MB of transfer"
else
  echo "   ⚠ brotli CLI not found (brew install brotli) — shipping layers UNCOMPRESSED"
fi

echo "==> further-reading cards (web/content/reading/*.md -> reading.json)"
if command -v python3 >/dev/null 2>&1; then
  python3 tools/build_reading.py
else
  echo "   (python3 not found — skipping reading.json; the panel hides gracefully)"
fi

# Phase-0 bundle-size budget gate (docs/SCALING.md): fail the build if a tracked
# asset grows past its budget, so weight regressions are caught before they ship +
# bloat git/Pages. Budgets are sized just above current; bump them *deliberately*
# (with a note in the commit) when an asset is meant to grow.
echo "==> bundle-size budget gate"
# Budgets are the on-disk (post-brotli, hence over-the-wire) sizes. If the brotli step
# above is skipped, the raw layers blow these — that's intentional: don't ship a deploy
# that would download the uncompressed 46 MB taxi layer. Install brotli, or raise these.
budgets=(
  "app-interactive_bg.wasm:28"                          # WASM bundle
  "assets/processed/taxi_day_nyc.ostaxiday:15"          # citywide taxis (opt-in ?city=nyc), brotli ~11 MB
  "assets/processed/taxi_day_20260421.ostaxiday:3"      # Manhattan taxis (default build), brotli ~2 MB
  "assets/processed/graph_nyc.osgraph:8"                # citywide street graph, brotli ~5.5 MB
)
TOTAL_BUDGET_MB=175
gate_fail=0
for entry in "${budgets[@]}"; do
  f="${entry%%:*}"; budget="${entry##*:}"
  [ -f "$OUT/$f" ] || continue
  sz=$(du -m "$OUT/$f" | cut -f1)
  if [ "$sz" -gt "$budget" ]; then
    echo "   ✗ $f: ${sz} MB > ${budget} MB budget"; gate_fail=1
  else
    echo "   ✓ $f: ${sz}/${budget} MB"
  fi
done
total=$(du -sm "$OUT" | cut -f1)
if [ "$total" -gt "$TOTAL_BUDGET_MB" ]; then
  echo "   ✗ web/dist total: ${total} MB > ${TOTAL_BUDGET_MB} MB budget"; gate_fail=1
else
  echo "   ✓ web/dist total: ${total}/${TOTAL_BUDGET_MB} MB"
fi
if [ "$gate_fail" -ne 0 ]; then
  echo "✗ bundle over budget — shrink the asset, or raise the budget in web/build.sh (and say why in the commit)."
  exit 1
fi

echo "==> done: $(du -sh "$OUT" | cut -f1) in $OUT"
ls -lah "$OUT"/*.wasm
echo "Serve:  python3 -m http.server -d $OUT 8080   (then open http://localhost:8080)"
