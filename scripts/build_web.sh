#!/usr/bin/env bash
set -euo pipefail

# The browser build: a static site in target/web/. The game's data is not in the
# wasm: it is gzipped next to it (target/web/data/, listed in manifest.json with
# a content hash in each URL), so a new build of the code does not make anyone
# download the data again (see crates/datafs and web/index.html).
# Serve it with any static server, or GitHub Pages.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
tgt="${CARGO_TARGET_DIR:-$root/target}"
out="$tgt/web"

command -v wasm-bindgen >/dev/null || {
  echo "wasm-bindgen-cli is required (same version as the wasm-bindgen crate in Cargo.lock)" >&2
  exit 1
}

# libsm64 (Mario) is C: it needs an LLVM clang that targets wasm (brew install llvm);
# crates/libsm64-native/build.rs finds it, or set CC_wasm32_unknown_unknown.
cargo build --package game --target wasm32-unknown-unknown --profile wasm-release
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" --out-name game \
  "$tgt/wasm32-unknown-unknown/wasm-release/game.wasm"
# A version in the URLs, so a browser that has the previous build cached
# fetches the new one (python's server and GitHub Pages both let it cache).
v="$(shasum "$out"/game_bg.wasm | cut -c1-12)"
li="$(shasum web/i18n.js | cut -c1-12)"
sed -e "s#'./game.js'#'./game.js?v=$v'#" -e "s#'./game_bg.wasm'#'./game_bg.wasm?v=$v'#" \
  -e "s#\"./i18n.js\"#\"./i18n.js?v=$li\"#" web/index.html > "$out"/index.html
cp -R web/img "$out"/img
# The landing page's texts (tools/i18n/landing.py)
cp web/i18n.js "$out"/i18n.js
# Apache hosts (games.cimafoundation.org) serve .wasm without its type unless told
# and compressed on the way when the server can (mod_deflate): the wasm is
# about a quarter of its size gzipped
cat > "$out"/.htaccess <<'HT'
AddType application/wasm .wasm
# the page and the data list must never be kept by a proxy (the rest is
# versioned in its URL)
<IfModule mod_headers.c>
  <FilesMatch "\.(html|json)$">
    Header set Cache-Control "no-cache"
  </FilesMatch>
</IfModule>
<IfModule mod_deflate.c>
  AddOutputFilterByType DEFLATE application/wasm application/javascript text/javascript text/html text/css application/json
</IfModule>
HT
# What the game loads: the one scenario, the behaviours, the fuels, the sounds,
# the models and pictures.
python3 -I - "$root" "$out" <<'PY'
import gzip, hashlib, json, os, sys

root, out = sys.argv[1], sys.argv[2]
files = []
# assets/: the models and pictures the desktop binary has compiled in (2026-10-30)
for sub in ("data/scenarios/rocca_ventosa", "data/behaviours", "data/i18n", "data/sounds", "assets/models", "assets/characters", "assets/scenarios", "assets/brand"):
    for d, _, names in os.walk(os.path.join(root, sub)):
        for n in names:
            rel = os.path.relpath(os.path.join(d, n), root).replace(os.sep, "/")
            # from assets/ only what the game reads: the bakes and the pictures
            if rel.startswith("assets/") and (not rel.endswith((".json", ".png")) or "preview" in rel):
                continue
            files.append(rel)
files.append("data/fuels_eu12.json")
manifest = []
for path in sorted(f for f in files if not f.endswith(("params.json", "i18n/keys.json", "CREDITS.txt"))):
    raw = open(os.path.join(root, path), "rb").read()
    gz = gzip.compress(raw, 9, mtime=0)
    dest = os.path.join(out, path + ".gz")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    open(dest, "wb").write(gz)
    h = hashlib.sha256(gz).hexdigest()[:12]
    manifest.append({"path": path, "url": f"./{path}.gz?v={h}", "bytes": len(gz)})
json.dump(manifest, open(os.path.join(out, "data", "manifest.json"), "w"), indent=0)
print("data: %d files, %.1f MB gzipped (of %.1f MB)" % (
    len(manifest), sum(m["bytes"] for m in manifest) / 1e6,
    sum(os.path.getsize(os.path.join(root, m["path"])) for m in manifest) / 1e6))
PY
if command -v wasm-opt >/dev/null; then
  # Rust's default wasm features; a failure keeps the unoptimised file
  if wasm-opt -Oz --strip-debug --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
      --enable-mutable-globals --enable-reference-types --enable-multivalue \
      -o "$out"/game.opt.wasm "$out"/game_bg.wasm; then
    mv "$out"/game.opt.wasm "$out"/game_bg.wasm
  else
    echo "wasm-opt failed: the wasm is not optimised" >&2
    rm -f "$out"/game.opt.wasm
  fi
fi
# the data list's URL carries its hash, as the wasm's does
m="$(shasum "$out"/data/manifest.json | cut -c1-12)"
sed -i.bak "s#'./data/manifest.json'#'./data/manifest.json?v=$m'#" "$out"/index.html && rm "$out"/index.html.bak
# a gzipped copy, inflated by the page: some hosts do not compress
gzip -9 -c "$out"/game_bg.wasm > "$out"/game_bg.wasm.gz
echo "Built $out ($(du -sh "$out"/game_bg.wasm | cut -f1) wasm)"
