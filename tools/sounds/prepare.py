"""Builds data/sounds/ from the recordings the game uses (2026-10-30).

    python3 tools/sounds/prepare.py --kenney <kenney_interface-sounds.zip> --jingle <a3_archi_leggeri.mp3>

Downloads the Wikimedia Commons recordings (public domain / CC0), cuts the
gull calls, makes the cicadas seamless loops, takes the interface sounds from
Kenney's «Interface Sounds» (CC0, kenney.nl/assets/interface-sounds, the zip
needs a browser download) and the opening jingle (Lyria 3 via OpenRouter, two
cuts of it). Everything is MP3 (LAME). Needs ffmpeg and
NumPy. The ambience and the cues of the game are made in code (crates/sfx).
"""
import argparse
import io
import os
import subprocess
import tempfile
import time
import urllib.request
import wave
import zipfile

import numpy as np

OUT = "data/sounds"
UA = {"User-Agent": "rocca-sfx/0.1 (isola-di-roccavento; CIMA Foundation)"}
COMMONS = "https://upload.wikimedia.org/wikipedia/commons/"

GULL = (COMMONS + "5/5c/XC707075_-_European_Herring_Gull_-_Larus_argentatus.mp3",
        "XC707075 - European Herring Gull - Larus argentatus, Sonothèque ADVL, CC0")
# seconds of the calls in the recording
GULL_CALLS = [(1.1, 4.3), (4.5, 6.8), (6.9, 10.6), (17.4, 19.5)]
CICADAS = [
    (COMMONS + "a/a8/Cicada_orni.ogg", "Cicada orni, DavidDelon, public domain"),
    (COMMONS + "f/f4/Cicadas_in_Greece.ogg", "Cicadas in Greece, Channel R, public domain"),
    (COMMONS + "3/3d/Cicada_20200619.wav", "Cicada 20200619, Shyamal, CC0"),
]
CICADA_S = 24.0
LOOP_FADE_S = 3.0
# The opening jingle (user, 2026-10-30), two cuts chosen on the clip's envelope:
# its climax, in E minor, as the CIMA logo fades in, then its closing cadence
# on G as the logo fades out on the island. name -> (start, length, fade in,
# fade out) in seconds; each ends in the clip's own decay.
JINGLES = {
    "jingle_logo": (7.4, 2.6, 0.25, 0.5),
    "jingle_isola": (27.0, 2.7, 0.3, 0.5),
}
# game name -> Kenney file
KENNEY = {
    "ui_clic": "click_002",
    "ui_prendi": "select_002",
    "ui_ordine": "drop_002",
    "ui_errore": "error_004",
    "ui_passo": "confirmation_001",
}


def pcm(src, rate):
    """Any audio file -> mono float32 at `rate`."""
    raw = subprocess.run(["ffmpeg", "-loglevel", "error", "-i", src, "-ac", "1", "-ar", str(rate), "-f", "s16le", "-"],
                         check=True, capture_output=True).stdout
    return np.frombuffer(raw, np.int16).astype(np.float32) / 32768


def save(x, rate, name, quality=6):
    """float32 mono -> data/sounds/<name>.mp3, LAME VBR (`quality` 0 best, 9
    smallest). Not Ogg: ffmpeg's own Vorbis encoder ignores the bitrate."""
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as f:
        tmp = f.name
    with wave.open(tmp, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype(np.int16).tobytes())
    out = os.path.join(OUT, name + ".mp3")
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", tmp, "-c:a", "libmp3lame", "-q:a", str(quality), out], check=True)
    os.unlink(tmp)
    print(out, os.path.getsize(out) // 1000, "kB")


def rms_to(x, db):
    return x * (10 ** (db / 20) / (np.sqrt((x ** 2).mean()) + 1e-9))


def fetch(url, dir):
    """Downloads once into `dir` (Commons answers 429 to quick repeats)."""
    path = os.path.join(dir, os.path.basename(url))
    if os.path.exists(path):
        return path
    time.sleep(2)
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120) as r:
        open(path, "wb").write(r.read())
    return path


def seamless(x, rate, length_s, fade_s):
    """`length_s` of `x` whose end fades into its start: it loops with no seam."""
    n, f = int(length_s * rate), int(fade_s * rate)
    start = max(0, (len(x) - n - f) // 2)
    seg = x[start:start + n + f].copy()
    ramp = np.linspace(0, 1, f, dtype=np.float32)
    seg[:f] = seg[:f] * ramp + seg[n:n + f] * (1 - ramp)
    return seg[:n]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kenney", required=True)
    ap.add_argument("--jingle", required=True)
    ap.add_argument("--cache", default=os.path.join(tempfile.gettempdir(), "rocca-sounds"))
    a = ap.parse_args()
    os.makedirs(OUT, exist_ok=True)
    credits = ["Suoni registrati del gioco (tools/sounds/prepare.py). Tutto il resto è sintetizzato in crates/sfx.", ""]
    os.makedirs(a.cache, exist_ok=True)
    dl = a.cache
    rate = 32000
    x = pcm(fetch(GULL[0], dl), rate)
    for k, (a0, a1) in enumerate(GULL_CALLS):
        call = x[int(a0 * rate):int(a1 * rate)].copy()
        e = int(0.05 * rate)
        call[:e] *= np.linspace(0, 1, e)
        call[-e:] *= np.linspace(1, 0, e)
        save(rms_to(call, -20), rate, f"gabbiano_{k + 1}")
    credits.append(f"gabbiano_*.mp3: {GULL[1]} ({GULL[0]})")
    for k, (url, who) in enumerate(CICADAS):
        loop = seamless(pcm(fetch(url, dl), rate), rate, CICADA_S, LOOP_FADE_S)
        save(rms_to(loop, -22), rate, f"cicale_{k + 1}", 7)
        credits.append(f"cicale_{k + 1}.mp3: {who} ({url})")
    with zipfile.ZipFile(a.kenney) as z:
        for name, src in KENNEY.items():
            data = z.read(f"Audio/{src}.ogg")
            with tempfile.NamedTemporaryFile(suffix=".ogg", delete=False) as f:
                f.write(data)
            save(pcm(f.name, 44100), 44100, name, 4)
            os.unlink(f.name)
    credits.append("ui_*.mp3: Kenney, «Interface Sounds» 1.0, CC0 (https://kenney.nl/assets/interface-sounds)")
    for name, (start, length, fade_in, fade_out) in JINGLES.items():
        out = os.path.join(OUT, name + ".mp3")
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-ss", str(start), "-t", str(length), "-i", a.jingle,
                        "-af", f"afade=t=in:d={fade_in},afade=t=out:st={length - fade_out}:d={fade_out},loudnorm=I=-14:TP=-1",
                        "-ar", "44100", "-c:a", "libmp3lame", "-q:a", "3", out], check=True)
        print(out, os.path.getsize(out) // 1000, "kB")
        credits.append(f"{name}.mp3: generato con Google Lyria 3 (lyria-3-clip-preview, via OpenRouter) il 2026-10-30, {length} s da {start} s")
    open(os.path.join(OUT, "CREDITS.txt"), "w").write("\n".join(credits) + "\n")


if __name__ == "__main__":
    main()
