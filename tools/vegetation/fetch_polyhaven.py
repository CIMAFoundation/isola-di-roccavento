"""Download the CC0 Poly Haven plant models the vegetation is built from
(2026-11-04, docs/TODO.md: «Vegetazione realistica»).

    python3 tools/vegetation/fetch_polyhaven.py [--res 1k] [asset ...]

Writes vegetation_src/polyhaven/<asset>/ (glTF, .bin, textures), outside git:
the sources weigh gigabytes and are fetched again by this script. Poly Haven
assets are CC0 (https://polyhaven.com/license). Stdlib only.
"""
import argparse
import json
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'vegetation_src' / 'polyhaven'

# asset -> the species it stands in for (see docs/TODO.md)
ASSETS = {
    'island_tree_01': 'leccio',
    'island_tree_02': 'ulivo',
    'island_tree_03': 'carrubo',
    'tree_small_02': 'sughera',
    'pine_tree_01': "pino d'Aleppo",
    'searsia_lucida': 'lentisco',
    'searsia_burchellii': 'mirto',
    'didelta_spinosa': 'corbezzolo',
    'leipoldtia_schultzei': 'cisto',
    'wild_rooibos_bush': 'erica, ginestra',
    'fir_sapling_medium': 'cipresso (nei borghi, stretto)',
    'shrub_03': 'rosmarino',
    'shrub_04': 'rosmarino',
}

UA = {'User-Agent': 'isola-di-roccavento-vegetation-fetch'}


def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120) as r:
        return r.read()


def fetch(asset, res):
    files = json.loads(get(f'https://api.polyhaven.com/files/{asset}'))
    g = files['gltf'][res]['gltf']
    d = OUT / asset
    d.mkdir(parents=True, exist_ok=True)
    todo = [(g['url'], Path(g['url']).name, g.get('size'))]
    todo += [(v['url'], k, v.get('size')) for k, v in g.get('include', {}).items()]
    for url, rel, size in todo:
        p = d / rel
        if p.exists() and (size is None or p.stat().st_size == size):
            continue
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(get(url))
    # the leaves' cut-out: a map of its own, which the glTF does not use
    # (`alpha.py` wires it in)
    for key, v in files.items():
        if 'alpha' in key.lower() and res in v and 'png' in v[res]:
            p = d / 'textures' / f'{asset}_{key.lower()}_{res}.png'
            if not p.exists():
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_bytes(get(v[res]['png']['url']))
    info = json.loads(get(f'https://api.polyhaven.com/info/{asset}'))
    (d / 'info.json').write_text(json.dumps({'asset': asset, 'name': info.get('name'), 'authors': info.get('authors'),
                                             'license': 'CC0', 'source': f'https://polyhaven.com/a/{asset}',
                                             'polycount': info.get('polycount')}, indent=1))
    total = sum(f.stat().st_size for f in d.rglob('*') if f.is_file())
    print(f'{asset:22s} {ASSETS.get(asset, ""):18s} {total / 1e6:7.1f} MB', flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--res', default='1k')
    ap.add_argument('assets', nargs='*')
    a = ap.parse_args()
    for asset in a.assets or ASSETS:
        try:
            fetch(asset, a.res)
        except Exception as e:  # keep going: one missing asset should not stop the rest
            print(f'{asset}: {e}', file=sys.stderr)


if __name__ == '__main__':
    main()
