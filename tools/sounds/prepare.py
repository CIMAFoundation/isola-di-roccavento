"""Builds data/sounds/ from the recordings the game uses (2026-10-30).

    python3 tools/sounds/prepare.py --kenney <kenney_interface-sounds.zip> --jingle <a3_archi_leggeri.mp3>
    python3 tools/sounds/prepare.py --parti ambiente      # only the fire and wind beds

Downloads the Wikimedia Commons recordings (public domain / CC0), cuts the
gull calls, makes the cicadas seamless loops, takes the interface sounds from
Kenney's «Interface Sounds» (CC0, kenney.nl/assets/interface-sounds, the zip
needs a browser download) and the opening jingle (Lyria 3 via OpenRouter, two
cuts of it). The fire and the wind (user, 2026-11-02) are Freesound recordings
(CC0, the public previews), cut into beds the game loops (`crates/sfx`).
Everything is MP3 (LAME). Needs ffmpeg and NumPy. The sea and the cues of the
game are made in code (crates/sfx).
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
# The fire and the wind (user, 2026-11-02: recordings instead of the noise made
# in code). Freesound, all CC0, from the public previews (128 kbit/s; the
# originals need an account). A bed is cut long enough to loop: the game drops
# BED_MARGIN_S at each end (whatever padding the MP3 decoder adds) and fades the
# last BED_FADE_S into the start, so a file holds loop + fade + 2 margins.
FREESOUND = "https://freesound.org/s/"
BEDS = {
    # name: (sources, start s, loop s, high-pass Hz, LAME quality). Several
    # sources are summed at the given loudness offsets (LU), each tiled to the
    # length. The wind has little above 8 kHz: a lower quality is enough.
    "fuoco": ([("https://cdn.freesound.org/previews/659/659951_13767047-hq.mp3", 0.0)], 3.5, 30.0, 40, 5),
    "fuoco_forte": ([("https://cdn.freesound.org/previews/755/755366_1185437-hq.mp3", 0.0),
                     ("https://cdn.freesound.org/previews/387/387924_4082837-hq.mp3", -3.0)], 1.0, 30.0, 35, 5),
    # a breeze, not a gale (user, 2026-11-02: «molto forti come campioni»):
    # the leaves after the birds, the grass after the gusts
    "vento_alberi": ([("https://cdn.freesound.org/previews/150/150174_229898-hq.mp3", 0.0)], 34.0, 40.0, 80, 7),
    "vento_aperto": ([("https://cdn.freesound.org/previews/376/376965_1661766-hq.mp3", 0.0)], 46.0, 42.0, 80, 7),
}
BED_CREDITS = [
    "fuoco.mp3: Trees and bush burning in a wildfire, Legand569, CC0 (https://freesound.org/s/659951/)",
    "fuoco_forte.mp3: Wild fire. Large, goochiano, CC0 (https://freesound.org/s/755366/); "
    "Fire - massive no cracking, Aegersum, CC0 (https://freesound.org/s/387924/)",
    "vento_alberi.mp3: wind in the trees, utente Freesound cancellato (229898), CC0 (https://freesound.org/s/150174/)",
    "vento_aperto.mp3: Wind is blowing into the grass on the top of the hill (Venezuela, Gran Sabana), "
    "felix.blume, CC0 (https://freesound.org/s/376965/)",
]
BED_RATE = 32000
# must match sfx::BED_MARGIN_S and sfx::BED_FADE_S
BED_MARGIN_S = 0.25
BED_FADE_S = 4.0
BED_LUFS = -23.0
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


def loudness(x, rate):
    """Integrated loudness (LUFS) of float32 mono, by ffmpeg's EBU R128 meter."""
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as f:
        tmp = f.name
    with wave.open(tmp, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype(np.int16).tobytes())
    log = subprocess.run(["ffmpeg", "-hide_banner", "-nostats", "-i", tmp, "-af", "ebur128", "-f", "null", "-"],
                         capture_output=True, text=True).stderr
    os.unlink(tmp)
    return float(log.rsplit("I:", 1)[1].split("LUFS")[0])


def high_pass(x, rate, hz):
    """Second-order Butterworth high-pass, forwards and backwards (no phase shift)."""
    try:
        from scipy.signal import butter, sosfiltfilt
        return sosfiltfilt(butter(2, hz, "highpass", fs=rate, output="sos"), x).astype(np.float32)
    except ImportError:
        pass
    import math
    w = math.tan(math.pi * hz / rate)
    k = 1 / (1 + math.sqrt(2) * w + w * w)
    b = np.array([k, -2 * k, k])
    a = np.array([1, 2 * (w * w - 1) * k, (1 - math.sqrt(2) * w + w * w) * k])

    def run(v):
        y = np.zeros_like(v)
        x1 = x2 = y1 = y2 = 0.0
        for i, s in enumerate(v):
            o = b[0] * s + b[1] * x1 + b[2] * x2 - a[1] * y1 - a[2] * y2
            x2, x1, y2, y1 = x1, s, y1, o
            y[i] = o
        return y
    return run(run(x)[::-1])[::-1].astype(np.float32)


def bed(sources, start, loop_s, hp, dl):
    """A bed for the game: loop + fade + a margin at each end, at BED_LUFS."""
    n = int((loop_s + BED_FADE_S + 2 * BED_MARGIN_S) * BED_RATE)
    mix = np.zeros(n, np.float32)
    for k, (url, lu) in enumerate(sources):
        x = pcm(fetch(url, dl), BED_RATE)
        x = x[int(start * BED_RATE):] if k == 0 else x[int(0.5 * BED_RATE):-int(0.5 * BED_RATE)]
        if len(x) < n:  # a shorter layer is tiled, its seams crossfaded
            f = int(1.0 * BED_RATE)
            ramp = np.linspace(0, 1, f, dtype=np.float32)
            out = x.copy()
            while len(out) < n:
                out = np.concatenate([out[:-f], out[-f:] * (1 - ramp) + x[:f] * ramp, x[f:]])
            x = out
        x = high_pass(x[:n], BED_RATE, hp)
        mix += x * 10 ** ((BED_LUFS + lu - loudness(x, BED_RATE)) / 20)
    return mix * 10 ** ((BED_LUFS - loudness(mix, BED_RATE)) / 20)


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
    ap.add_argument("--kenney")
    ap.add_argument("--jingle")
    ap.add_argument("--parti", default="animali,ambiente,interfaccia,jingle",
                    help="which parts to make again (comma-separated); the credits are always written whole")
    ap.add_argument("--cache", default=os.path.join(tempfile.gettempdir(), "rocca-sounds"))
    a = ap.parse_args()
    parts = set(a.parti.split(","))
    if "interfaccia" in parts and not a.kenney or "jingle" in parts and not a.jingle:
        ap.error("--kenney and --jingle are needed for the parts interfaccia and jingle")
    os.makedirs(OUT, exist_ok=True)
    credits = ["Suoni registrati del gioco (tools/sounds/prepare.py). Il mare e i segnali sono sintetizzati in crates/sfx.", ""]
    os.makedirs(a.cache, exist_ok=True)
    dl = a.cache
    rate = 32000
    if "ambiente" in parts:
        for name, (sources, start, loop_s, hp, quality) in BEDS.items():
            save(bed(sources, start, loop_s, hp, dl), BED_RATE, name, quality)
    credits += BED_CREDITS
    if "animali" in parts:
        make_animals(dl, rate)
    credits.append(f"gabbiano_*.mp3: {GULL[1]} ({GULL[0]})")
    credits += [f"cicale_{k + 1}.mp3: {who} ({url})" for k, (url, who) in enumerate(CICADAS)]
    if "interfaccia" in parts:
        make_interface(a.kenney)
    credits.append("ui_*.mp3: Kenney, «Interface Sounds» 1.0, CC0 (https://kenney.nl/assets/interface-sounds)")
    for name, (start, length, fade_in, fade_out) in JINGLES.items():
        if "jingle" in parts:
            make_jingle(a.jingle, name, start, length, fade_in, fade_out)
        credits.append(f"{name}.mp3: generato con Google Lyria 3 (lyria-3-clip-preview, via OpenRouter) il 2026-10-30, {length} s da {start} s")
    open(os.path.join(OUT, "CREDITS.txt"), "w").write("\n".join(credits) + "\n")


def make_animals(dl, rate):
    x = pcm(fetch(GULL[0], dl), rate)
    for k, (a0, a1) in enumerate(GULL_CALLS):
        call = x[int(a0 * rate):int(a1 * rate)].copy()
        e = int(0.05 * rate)
        call[:e] *= np.linspace(0, 1, e)
        call[-e:] *= np.linspace(1, 0, e)
        save(rms_to(call, -20), rate, f"gabbiano_{k + 1}")
    for k, (url, who) in enumerate(CICADAS):
        loop = seamless(pcm(fetch(url, dl), rate), rate, CICADA_S, LOOP_FADE_S)
        save(rms_to(loop, -22), rate, f"cicale_{k + 1}", 7)


def make_interface(kenney):
    with zipfile.ZipFile(kenney) as z:
        for name, src in KENNEY.items():
            data = z.read(f"Audio/{src}.ogg")
            with tempfile.NamedTemporaryFile(suffix=".ogg", delete=False) as f:
                f.write(data)
            save(pcm(f.name, 44100), 44100, name, 4)
            os.unlink(f.name)


def make_jingle(jingle, name, start, length, fade_in, fade_out):
    out = os.path.join(OUT, name + ".mp3")
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-ss", str(start), "-t", str(length), "-i", jingle,
                    "-af", f"afade=t=in:d={fade_in},afade=t=out:st={length - fade_out}:d={fade_out},loudnorm=I=-14:TP=-1",
                    "-ar", "44100", "-c:a", "libmp3lame", "-q:a", "3", out], check=True)
    print(out, os.path.getsize(out) // 1000, "kB")


if __name__ == "__main__":
    main()
