#!/usr/bin/env python3
"""Does a built site run with these address parameters? Headless Chromium, no
window, no sound (as scripts/perf_web.py, whose helpers this uses).

  .venv/bin/python scripts/web_check.py <site> <out dir> <query> [minutes] [phone]
      e.g. target/web out/webgl 'q=media&gpu=0&volumi=1'
      `phone`: a phone's viewport and touch (390x844 at 3x), as the page sees
      one: the light profile, WebGL2.

Starts a game, runs it on by `minutes` (60), takes four pictures (the play
camera, the fire from 2.5 km, the front from 900 and from 300 m) and prints:
whether the WebGPU build was loaded, the frames a second at each view, and
every error or warning the page logged (with the adapter's line).
"""
import argparse, re, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import perf_web as pw
from playwright.sync_api import sync_playwright

site, out, query = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
minutes = float(sys.argv[4]) if len(sys.argv) > 4 else 60
out.mkdir(parents=True, exist_ok=True)
args = argparse.Namespace(viewport='390x844', dpr=3) if 'phone' in sys.argv[5:] else argparse.Namespace(viewport='1440x900', dpr=1)
clean = lambda s: re.sub(r'%c|color: [^;]*;?|background: [^;]*;?|font-style: \w+', '', s).strip()
with sync_playwright() as p:
    server = pw.serve(site)
    browser, page, phone = pw.new_page(p, 'chromium', args)
    logs = []
    page.on('console', lambda m: logs.append((m.type, clean(m.text))))
    page.on('pageerror', lambda e: logs.append(('pageerror', str(e))))
    try:
        page.goto(f'http://127.0.0.1:{server.server_port}/?playtest=1&{query}')
        page.wait_for_function('window.roccaPlaytest?.state && !document.querySelector("#loading")', timeout=180000)
        page.wait_for_timeout(4000)
        print('webgpu build:', page.evaluate('window.roccaWebGpu'), ' profile:', page.evaluate('window.roccaProfile'), ' auto:', page.evaluate('JSON.stringify(window.roccaAuto || {})'), ' provoking vertex:', page.evaluate('window.roccaProvoking'), ' address:', page.evaluate('location.search'))
        page.evaluate('delete window.roccaPlaytest.camera')
        pw.start_game(page, phone)
        page.evaluate(f'window.roccaPlaytest.advance_s = {minutes * 60}')
        page.wait_for_function('window.roccaPlaytest.advance_s === 0', timeout=240000)
        pw.frames(page, 120)
        page.screenshot(path=str(out / 'gioco.png'))
        state = page.evaluate('window.roccaPlaytest.state')
        front = state.get('front') or [5200, 4600]
        for name, d, pitch in [('fuoco2500', 2500, -0.61), ('fronte900', 900, -0.35), ('fronte300', 300, -0.26)]:
            page.evaluate(f'window.roccaPlaytest.camera = {{x: {front[0]}, y: {front[1]}, distance: {d}, yaw: 0.6, pitch: {pitch}}}')
            pw.frames(page, 150)
            f0, t0 = page.evaluate('window.roccaPlaytest.frame'), time.time()
            page.wait_for_timeout(4000)
            f1, t1 = page.evaluate('window.roccaPlaytest.frame'), time.time()
            page.screenshot(path=str(out / f'{name}.png'))
            print(f'{name}: {(f1 - f0) / (t1 - t0):.0f} frames a second')
        state = page.evaluate('window.roccaPlaytest.state')
        print(f"phase {state['phase']}, t {state['time_s'] / 60:.0f} min")
        # another address than at the start: the page loaded again (a fallback)
        print('address at the end:', page.evaluate('location.search'))
    except Exception as e:
        print('FAILED:', type(e).__name__, str(e)[:300])
        try:
            page.screenshot(path=str(out / 'failed.png'))
        except Exception:
            pass
    browser.close(); server.shutdown()
seen = set()
for kind, text in logs:
    key = text[:160]
    if key in seen:
        continue
    if kind in ('error', 'pageerror', 'warning') or re.search(r'AdapterInfo|panicked|wgpu|ERROR|WARN|volum|fire_', text):
        seen.add(key)
        print(f'[{kind}] {text[:500]}')
