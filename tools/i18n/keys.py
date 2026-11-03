#!/usr/bin/env python3
"""Estrae i testi da tradurre (le chiavi di `tr!("...")`) dai sorgenti Rust.

    python3 tools/i18n/keys.py            # scrive data/i18n/keys.json
    python3 tools/i18n/keys.py --check    # verifica ogni data/i18n/<lingua>.json

Le chiavi sono i testi italiani. La verifica controlla che ogni catalogo li
abbia tutti, senza avanzi, e che ogni traduzione conservi i segnaposto `{0}`,
il grassetto `**`, il sottolineato `__` e gli a capo.
"""
import json, re, sys, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
I18N = ROOT / "data" / "i18n"
LIT = re.compile(r'\btr!\(\s*"((?:[^"\\]|\\.)*)"', re.S)


def unescape(s):
    return re.sub(r'\\(n|"|\'|\\|t)', lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), s)


def keys():
    found = {}
    for p in sorted((ROOT / "crates").rglob("*.rs")):
        text = p.read_text()
        # comments are not code: drop `//` lines
        text = "\n".join(l for l in text.split("\n") if not l.lstrip().startswith("//"))
        for m in LIT.finditer(text):
            found.setdefault(unescape(m.group(1)), set()).add(p.relative_to(ROOT).as_posix())
    # texts from the data, read with `i18n::data`: the scenarios' names and
    # descriptions (`game.json` → `levels`)
    for p in sorted((ROOT / "data" / "scenarios").glob("*/game.json")):
        for level in json.loads(p.read_text()).get("levels", []):
            for field in ("name", "about"):
                found.setdefault(level[field], set()).add(p.relative_to(ROOT).as_posix())
    return found


def shape(s):
    return (sorted(re.findall(r"\{\d+\}", s)), s.count("**"), s.count("__"), s.count("\n"))


def main():
    ks = keys()
    if "--check" not in sys.argv:
        (I18N / "keys.json").write_text(json.dumps(sorted(ks), ensure_ascii=False, indent=0) + "\n")
        print(f"{len(ks)} testi in data/i18n/keys.json")
        return
    bad = 0
    for p in sorted(I18N.glob("*.json")):
        if p.stem == "keys":
            continue
        cat = json.loads(p.read_text())
        missing = [k for k in ks if k not in cat]
        extra = [k for k in cat if k not in ks]
        wrong = [k for k in cat if k in ks and shape(k) != shape(cat[k])]
        empty = [k for k in cat if not cat[k].strip()]
        if missing or extra or wrong or empty:
            bad += 1
            print(f"{p.stem}: mancano {len(missing)}, avanzano {len(extra)}, forma diversa {len(wrong)}, vuote {len(empty)}")
            for k in (missing + extra + wrong + empty)[:3]:
                print("   ", repr(k)[:90])
        else:
            print(f"{p.stem}: ok ({len(cat)})")
    sys.exit(1 if bad else 0)


main()
