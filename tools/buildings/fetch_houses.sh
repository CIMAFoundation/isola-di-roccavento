#!/bin/sh
# Downloads the house sources into buildings_src/ (out of git): the Poly Haven
# CC0 facade kit at 1K, glTF and its colour maps only (the bake reads no
# normal or roughness maps). Then:
#   /Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup --python tools/buildings/build_real_houses.py
set -e
cd "$(dirname "$0")/../.."
A=modular_urban_apartments_facade
D=buildings_src/polyhaven/$A
mkdir -p "$D/textures"
curl -fsS "https://api.polyhaven.com/files/$A" -o "$D/files.json"
python3 -I - "$D" <<'PY' | while read -r url path; do curl -fsSL -C - --retry 5 -o "$D/$path" "$url"; done
import json, sys
f = json.load(open(sys.argv[1] + "/files.json"))["gltf"]["1k"]["gltf"]
print(f["url"], "modular_urban_apartments_facade_1k.gltf")
for k, v in f["include"].items():
    if k.endswith(".bin") or "_diff_" in k:
        print(v["url"], k)
PY
md5 -q "$D/$A.bin" 2>/dev/null || md5sum "$D/$A.bin"
echo "expected e54b1de59041da0f44db64efb8bda8f6"
