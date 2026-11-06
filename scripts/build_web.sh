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
# Two builds of the same code: WebGL2 (game.*, phones and browsers without
# WebGPU) and WebGPU (game_gpu.*, computers: drawn as on the desktop). Bevy
# chooses the backend when compiling, so the page chooses the build. The
# WebGPU one has a target directory of its own, or each would rebuild the other.
# `ROCCA_WEB_GPU=0` skips it (the page then always loads WebGL2).
gpu="${ROCCA_WEB_GPU:-1}"
cargo build --package game --target wasm32-unknown-unknown --profile wasm-release
if [ "$gpu" = 1 ]; then
  cargo build --package game --target wasm32-unknown-unknown --profile wasm-release --features webgpu --target-dir "$tgt/webgpu"
fi
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" --out-name game \
  "$tgt/wasm32-unknown-unknown/wasm-release/game.wasm"
if [ "$gpu" = 1 ]; then
  wasm-bindgen --target web --no-typescript --out-dir "$out" --out-name game_gpu \
    "$tgt/webgpu/wasm32-unknown-unknown/wasm-release/game.wasm"
fi
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
            # the logo: the game halves the 1200 px one (kiosk/characters.rs)
            # the scanned plants (`models::scans`) are not sent as they are: 20 MB
            # of meshes and 22 MB of cards. A lighter set is built below.
            if rel.startswith("assets/") and (not rel.endswith((".json", ".png")) or "preview" in rel or "_scans" in rel or "_real" in rel or rel == "assets/brand/cima_logo_white.png"):
                continue
            files.append(rel)
files.append("data/fuels_eu12.json")
manifest = []

def scans_for_the_web():
    """The scanned plants for the browser (`?scans=1`, WebGPU): without the
    nearest level (lod0, three quarters of the bytes) and with the cards at
    half size. Downloaded only when asked for (`only` in the manifest, see
    web/index.html). Needs Pillow for the cards: without it, no scans."""
    src = os.path.join(root, "assets/models")
    try:
        from PIL import Image
        index = json.load(open(os.path.join(src, "vegetation_scans.json")))
        data = open(os.path.join(src, "vegetation_scans.bin"), "rb").read()
        Image.MAX_IMAGE_PIXELS = None
        cards = Image.open(os.path.join(src, "foliage_scans.png")).convert("RGBA")
    except Exception as e:
        print("scanned plants: not in the web data (%s)" % e)
        return []
    import io
    vb, kept = index["vertex_bytes"], bytearray()
    for levels in index["plants"].values():
        levels.pop("lod0", None)
        for level in levels.values():
            n = level["vertices"] * vb + level["indices"] * 2
            chunk = data[level["offset"]:level["offset"] + n]
            level["offset"] = len(kept)
            kept += chunk
    half = cards.resize((cards.width // 2, cards.height // 2), Image.LANCZOS)
    index["atlas"] = {"width": half.width, "height": half.height}
    png = io.BytesIO()
    half.save(png, "PNG", optimize=True)
    found = [("assets/models/vegetation_scans.json", json.dumps(index).encode()),
             ("assets/models/vegetation_scans.bin", bytes(kept)),
             ("assets/models/foliage_scans.png", png.getvalue())]
    # the realistic rocca (scripts/build_rocca_real.py, landmarks::rocca_scans)
    # and vehicles (tools/units/build_real_vehicles.py, units.rs): as they are
    for n in ("rocca_scans.json", "vehicles_real.json"):
        if os.path.exists(os.path.join(src, n)):
            found.append(("assets/models/" + n, open(os.path.join(src, n), "rb").read()))
    return found

def photographs_for_the_web():
    """What else the realistic style reads (`?scans=1`, WebGPU): the ground's
    and the buildings' photographs (`assets/ground`, `assets/buildings`) at
    half their side, and their indexes as they are. Needs Pillow."""
    try:
        from PIL import Image
    except Exception as e:
        print("realistic ground and buildings: not in the web data (%s)" % e)
        return []
    import io
    found = []
    for sub in ("assets/ground", "assets/buildings"):
        for n in sorted(os.listdir(os.path.join(root, sub))) if os.path.isdir(os.path.join(root, sub)) else []:
            rel, full = sub + "/" + n, os.path.join(root, sub, n)
            if n.endswith(".json"):
                found.append((rel, open(full, "rb").read()))
            elif n.endswith(".png"):
                picture = Image.open(full)
                half = picture.resize((max(picture.width // 2, 1), max(picture.height // 2, 1)), Image.LANCZOS)
                png = io.BytesIO()
                half.save(png, "PNG", optimize=True)
                found.append((rel, png.getvalue()))
    return found

for path, raw in scans_for_the_web() + photographs_for_the_web():
    gz = gzip.compress(raw, 6, mtime=0)
    dest = os.path.join(out, path + ".gz")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    open(dest, "wb").write(gz)
    h = hashlib.sha256(gz).hexdigest()[:12]
    manifest.append({"path": path, "url": f"./{path}.gz?v={h}", "bytes": len(gz), "only": "scans"})
    print("for `?scans=1`: %s, %.1f MB gzipped (of %.1f MB)" % (path, len(gz) / 1e6, len(raw) / 1e6))
for path in sorted(f for f in files if not f.endswith(("params.json", "i18n/keys.json", "CREDITS.txt"))):
    raw = open(os.path.join(root, path), "rb").read()
    gz = gzip.compress(raw, 9, mtime=0)
    dest = os.path.join(out, path + ".gz")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    open(dest, "wb").write(gz)
    h = hashlib.sha256(gz).hexdigest()[:12]
    manifest.append({"path": path, "url": f"./{path}.gz?v={h}", "bytes": len(gz)})
json.dump(manifest, open(os.path.join(out, "data", "manifest.json"), "w"), indent=0)
always = [m for m in manifest if "only" not in m]
print("data: %d files, %.1f MB gzipped (of %.1f MB)" % (
    len(always), sum(m["bytes"] for m in always) / 1e6,
    sum(os.path.getsize(os.path.join(root, m["path"])) for m in always) / 1e6))
PY
builds=(game)
[ "$gpu" = 1 ] && builds+=(game_gpu)
for b in "${builds[@]}"; do
  if command -v wasm-opt >/dev/null; then
    # Rust's default wasm features; a failure keeps the unoptimised file
    if wasm-opt -Oz --strip-debug --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
        --enable-mutable-globals --enable-reference-types --enable-multivalue \
        -o "$out"/$b.opt.wasm "$out"/${b}_bg.wasm; then
      mv "$out"/$b.opt.wasm "$out"/${b}_bg.wasm
    else
      echo "wasm-opt failed: $b's wasm is not optimised" >&2
      rm -f "$out"/$b.opt.wasm
    fi
  fi
  # a gzipped copy, inflated by the page: some hosts do not compress
  gzip -9 -c "$out"/${b}_bg.wasm > "$out"/${b}_bg.wasm.gz
done
# A version in the URLs, so a browser that has the previous build cached
# fetches the new one (python's server and GitHub Pages both let it cache).
v="$(shasum "$out"/game_bg.wasm | cut -c1-12)"
vg="$v"
[ "$gpu" = 1 ] && vg="$(shasum "$out"/game_gpu_bg.wasm | cut -c1-12)"
li="$(shasum web/i18n.js | cut -c1-12)"
# the data list's URL carries its hash, as the wasm's does
m="$(shasum "$out"/data/manifest.json | cut -c1-12)"
sed -e "s#'./game.js'#'./game.js?v=$v'#" -e "s#'./game_bg.wasm'#'./game_bg.wasm?v=$v'#" \
  -e "s#'./game_gpu.js'#'./game_gpu.js?v=$vg'#" -e "s#'./game_gpu_bg.wasm'#'./game_gpu_bg.wasm?v=$vg'#" \
  -e "s#const webgpuBuilt = true#const webgpuBuilt = $([ "$gpu" = 1 ] && echo true || echo false)#" \
  -e "s#\"./i18n.js\"#\"./i18n.js?v=$li\"#" -e "s#'./data/manifest.json'#'./data/manifest.json?v=$m'#" \
  web/index.html > "$out"/index.html
for b in "${builds[@]}"; do
  echo "Built $out ($b: $(du -sh "$out"/${b}_bg.wasm | cut -f1) wasm)"
done
