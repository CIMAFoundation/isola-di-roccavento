#!/usr/bin/env python3
"""Touch regression: mobile DPR, portrait/landscape, tutorial, orders and restart.
Build with scripts/build_web.sh first. Uses an isolated browser profile.
"""
import argparse
import functools
import http.server
import json
from pathlib import Path
import sys
import threading
import time
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]

# No sound from a headless run (user, 2026-11-02): Playwright mutes headless
# Chromium (`--mute-audio`) but not WebKit, where the game's WebAudio reached
# the Mac's speakers. Everything the page sends to its speakers goes through a
# gain of 0; the audio graph keeps running, so the measures do not change.
MUTE = r"""
(() => {
  if (typeof AudioNode === 'undefined') return;
  const connect = AudioNode.prototype.connect;
  const silent = new WeakMap();
  AudioNode.prototype.connect = function (target, ...rest) {
    if (target instanceof AudioDestinationNode) {
      let gain = silent.get(target);
      if (!gain) {
        gain = target.context.createGain();
        gain.gain.value = 0;
        connect.call(gain, target);
        silent.set(target, gain);
      }
      return connect.call(this, gain, ...rest);
    }
    return connect.call(this, target, ...rest);
  };
  const play = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function () { this.muted = true; return play.call(this); };
})();
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser-executable')
    parser.add_argument('--browser', choices=['chromium', 'webkit'], default='chromium')
    parser.add_argument('--reloads', type=int, default=0, help='Reload after the title, checking each fresh startup')
    parser.add_argument('--smoke', action='store_true', help='Test native taps through title, name and tutorial, in both orientations')
    parser.add_argument('--edge-pan', action='store_true', help='Test dragging at each map edge, stopping and cancellation')
    parser.add_argument('--output', type=Path, default=ROOT / 'out/playtest/mobile')
    args = parser.parse_args()
    if args.browser == 'webkit' and not args.smoke:
        parser.error('WebKit supports --smoke; full gestures use Chromium CDP')
    args.output.mkdir(parents=True, exist_ok=True)
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_): pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Quiet, directory=str(ROOT / 'target/web')))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    errors, samples, actions = [], [], []
    started = time.monotonic()
    passed = False
    try:
        with sync_playwright() as pw:
            gpu = ['--enable-gpu'] + (['--use-angle=metal'] if sys.platform == 'darwin' else [])
            browser = getattr(pw, args.browser).launch(headless=True, executable_path=args.browser_executable, args=gpu if args.browser == 'chromium' else [])
            page = browser.new_page(viewport={'width': 390, 'height': 844}, device_scale_factor=3, is_mobile=True, has_touch=True, locale='it-IT')
            # canvas pixels per CSS pixel: the density, at most 2 (`web_input::backing_scale`)
            BACKING = 2
            page.add_init_script(MUTE)
            page.set_default_timeout(120000)
            page.on('pageerror', lambda e: errors.append(str(e)) if 'Using exceptions for control flow' not in str(e) else None)
            page.on('console', lambda m: errors.append(m.text) if m.type == 'error' else None)
            cdp = page.context.new_cdp_session(page) if args.browser == 'chromium' else None
            def state(): return page.evaluate('window.roccaPlaytest.state')
            def frames(n=4):
                frame = page.evaluate('window.roccaPlaytest.frame')
                page.wait_for_function('f => window.roccaPlaytest.frame >= f', arg=frame + n)
            def control(label):
                frames()
                matches = [c for c in state()['controls'] if c['label'] == label and c['enabled']]
                assert matches, f'Missing control: {label}'
                c = matches[0]
                w, h = page.viewport_size.values()
                assert 0 <= c['x'] <= w and 0 <= c['y'] <= h, (label, c)
                return c
            def touch(kind, points):
                cdp.send('Input.dispatchTouchEvent', {'type': kind, 'touchPoints': [dict(x=x, y=y, id=i, radiusX=5, radiusY=5, force=1) for i, x, y in points]})
                frames(2)
            def tap(label):
                c = control(label)
                if args.smoke:
                    # Native taps include browser event ordering and no artificial
                    # frame delay between press/release (especially on Safari).
                    page.touchscreen.tap(c['x'], c['y']); frames()
                else:
                    touch('touchStart', [(0, c['x'], c['y'])]); touch('touchEnd', [])
                actions.append(label)
            def drag(start, end, screenshot=None):
                touch('touchStart', [(0, *start)])
                for i in range(1, 13):
                    p = [start[j] + (end[j] - start[j]) * i / 12 for j in range(2)]
                    touch('touchMove', [(0, *p)])
                if screenshot: snap(screenshot)
                touch('touchEnd', [])
            def snap(name):
                frames()
                s = state()
                data_bytes = page.evaluate('Object.values(window.roccaData).reduce((n, b) => n + b.byteLength, 0)')
                samples.append({'name': name, 'memory_bytes': s['wasm_memory_bytes'], 'phase': s['phase'], 'js_data_bytes': data_bytes})
                page.screenshot(path=str(args.output / f'{name}.png'), scale='css')
                (args.output / f'{name}.json').write_text(json.dumps(s, ensure_ascii=False, indent=2))
                print(name, s['phase'], s['expect'], round(s['wasm_memory_bytes'] / 1048576, 1), 'MiB', flush=True)
            def rotate(w, h):
                page.set_viewport_size({'width': w, 'height': h}); frames(12)
                # The tutorial reserves part of the camera viewport for its
                # sidebar; check the entire canvas backing buffer instead.
                size = page.locator('#rocca').evaluate('c => [c.width, c.height]')
                assert size == [w * BACKING, h * BACKING], size
            def advance(seconds):
                page.evaluate('s => window.roccaPlaytest.advance_s = s', seconds)
                page.wait_for_function('window.roccaPlaytest.advance_s === 0'); frames()
            try:
                page.goto(f'http://127.0.0.1:{server.server_port}/?playtest=1')
                page.wait_for_function('window.roccaPlaytest?.state && !document.querySelector("#loading")')
                frames(); assert state()['camera']['pixels'] == [390 * BACKING, 844 * BACKING]
                assert page.evaluate('window.roccaDataGzip === true'), 'Phone data should stay compressed'
                assert page.evaluate('Object.values(window.roccaData).every(b => b[0] === 31 && b[1] === 139)'), 'Phone store must contain gzip bytes'
                snap('01-title-portrait')
                for reload in range(args.reloads):
                    page.reload()
                    page.wait_for_function('window.roccaPlaytest?.state && !document.querySelector("#loading")')
                    frames(30)
                    assert state()['phase'] == 'Intro' and state()['error'] is None
                    snap(f'01-reload-{reload + 1}')
                tap('Menu'); snap('01b-menu-portrait')
                rotate(844, 390); snap('01c-menu-landscape'); tap('Menu'); rotate(390, 844)
                tap('Gioca'); tap('Nome')
                page.locator('#player-name').fill('Prova telefono')
                page.get_by_role('button', name='Continua', exact=True).click(); frames()
                assert state()['player'] == 'Prova telefono'
                snap('02-intro-portrait')
                tap('TUTORIAL'); frames(90); snap('03-tutorial-portrait')
                if args.smoke:
                    tap('Avanti'); assert state()['step'] == 1
                    rotate(844, 390); snap('04-tutorial-landscape')
                    tap('Avanti'); assert state()['step'] == 2
                    assert not errors, errors
                    passed = True
                    return
                # The script's explanatory steps come first: «Avanti» up to the first drag.
                for _ in range(20):
                    if state()['tutorial_target']: break
                    tap('Avanti')
                frames(120)
                target = state()['tutorial_target']; assert target
                s0 = state()['step']
                team = state()['teams'][target['team']]['name']
                if args.edge_pan:
                    c = control(team)
                    touch('touchStart', [(0, c['x'], c['y'])])
                    touch('touchMove', [(0, c['x'] + 14, c['y'] - 14)])
                    center = (195, 350)
                    for x, y, axis, sign in [(382, 350, 0, 1), (8, 350, 0, -1), (195, 124, 1, -1), (195, 590, 1, 1)]:
                        touch('touchMove', [(0, *center)]); frames(8)
                        before = state()['camera']['focus']
                        touch('touchMove', [(0, x, y)])
                        page.wait_for_timeout(250); frames(4)
                        after = state()['camera']['focus']
                        assert (after[axis] - before[axis]) * sign > 10, (before, after, x, y)
                    snap('edge-pan-held')
                    # A stationary finger in the center must stop the motion.
                    touch('touchMove', [(0, *center)]); frames(8)
                    before = state()['camera']['focus']; frames(20)
                    assert sum(abs(a-b) for a,b in zip(before, state()['camera']['focus'])) < 1
                    # Panels do not pan, and cancel must leave no queued motion/order.
                    touch('touchMove', [(0, 100, 40)]); frames(8)
                    before = state()['camera']['focus']; frames(20)
                    assert sum(abs(a-b) for a,b in zip(before, state()['camera']['focus'])) < 1
                    touch('touchCancel', []); frames(20)
                    assert sum(abs(a-b) for a,b in zip(before, state()['camera']['focus'])) < 1
                    assert state()['step'] == s0 and state()['teams'][target['team']]['hex'] is None
                    snap('edge-pan-canceled')
                    c = control(team)
                    touch('touchStart', [(0, c['x'], c['y'])])
                    touch('touchMove', [(0, c['x'] + 14, c['y'] - 14)])
                    touch('touchMove', [(0, 382, 350)])
                    page.wait_for_timeout(150); frames(4)
                    # Release on the HUD cancels the order and stops panning.
                    touch('touchMove', [(0, 100, 40)]); touch('touchEnd', [])
                    before = state()['camera']['focus']; frames(20)
                    assert sum(abs(a-b) for a,b in zip(before, state()['camera']['focus'])) < 1
                    assert state()['step'] == s0 and state()['teams'][target['team']]['hex'] is None
                    assert not errors, errors
                    passed = True
                    return
                # Tap-select then tap the ground: this never depended on a mouse.
                tap(team)
                target = state()['tutorial_target']
                touch('touchStart', [(0, target['x'], target['y'])]); touch('touchEnd', [])
                assert state()['step'] == s0 + 1, state()
                rotate(844, 390); frames(120); snap('04-tutorial-landscape')
                target = state()['tutorial_target']; team = state()['teams'][target['team']]['name']
                c = control(team)
                # Releasing over the HUD cancels; the tutorial must stay put.
                drag((c['x'], c['y']), (100, 40))
                assert state()['step'] == s0 + 1 and state()['teams'][target['team']]['hex'] is None
                snap('04b-canceled-drop')
                c = control(team)
                touch('touchStart', [(0, c['x'], c['y'])])
                touch('touchMove', [(0, c['x'] + 14, c['y'] - 14)])
                touch('touchCancel', [])
                assert state()['step'] == s0 + 1 and state()['teams'][target['team']]['hex'] is None
                c = control(team)
                target = state()['tutorial_target']
                before = state()['camera']['focus']
                drag((c['x'], c['y']), (target['x'], target['y']), '05-drag-preview')
                assert state()['step'] == s0 + 2, state()
                assert state()['teams'][target['team']]['hex'] is not None
                # A fresh camera flight can start after the valid drop, so
                # ownership is checked in the saved in-drag observation.
                in_drag = json.loads((args.output / '05-drag-preview.json').read_text())
                assert sum(abs(a-b) for a,b in zip(before,in_drag['camera']['focus'])) < 10
                def read_until(expect):
                    for _ in range(20):
                        if state()['expect'].startswith(expect): return
                        tap('Avanti')
                    assert False, (expect, state()['expect'])
                read_until('Resume'); tap('Riprendi'); advance(1800); frames(120)
                read_until('Evacuate')
                snap('06-evacuation-instruction')
                tap('Le Ghiande'); snap('07-evacuation-card'); before_step = state()['step']; tap('Evacua')
                assert state()['step'] > before_step, state()
                read_until('Resume'); tap('Riprendi')
                # Keep each batch short enough to stop at the tutorial boundary.
                for _ in range(40):
                    if state()['phase'] != 'Tutorial': break
                    advance(120)
                assert state()['phase'] == 'Scenario', state()
                snap('08b-scenario'); tap('Parti'); frames(40)
                rotate(390, 844); snap('09-game-portrait')
                # Zoom, two-finger pan/rotation, and cancellation must not issue orders.
                teams = state()['teams']
                dist = state()['camera']['distance']; tap('+')
                assert state()['camera']['distance'] < dist
                pinch_before = state()['camera']['distance']
                touch('touchStart', [(0, 80, 420), (1, 260, 420)])
                touch('touchMove', [(0, 60, 430), (1, 290, 400)])
                touch('touchEnd', [])
                assert state()['camera']['distance'] < pinch_before * 0.9
                assert [t['hex'] for t in state()['teams']] == [t['hex'] for t in teams]
                tap('Isola'); frames(10)
                snap('10-game-home')
                tap('Riprendi')
                for _ in range(30):
                    if state()['phase'] == 'Fine': break
                    advance(600); m = state()['wasm_memory_bytes'] / 1048576
                    print('  run', state()['phase'], round(m, 1), 'MiB', flush=True)
                assert state()['phase'] == 'Fine', state()
                page.wait_for_function('window.roccaPlaytest.state.baseline_ready')
                snap('11-end-portrait')
                rotate(844, 390); snap('12-end-landscape')
                tap('Nuova partita'); snap('13-restart')
                assert state()['phase'] == 'Intro'
                rotate(320, 568); snap('14-small-phone'); tap('Gioca'); snap('15-small-intro')
                tap('TUTORIAL'); frames(90)
                rotate(568, 320); frames(90); snap('16-small-landscape')
                tap('Avanti'); assert state()['step'] == 1
                # Also smoke-test mouse input on a desktop Retina viewport.
                page.set_viewport_size({'width': 1280, 'height': 800}); frames(12)
                c = control('Avanti'); page.mouse.click(c['x'], c['y']); frames()
                assert state()['step'] == 2
                snap('17-desktop-mouse')
                assert not errors, errors
                assert max(s['memory_bytes'] for s in samples) <= 450 * 1048576, samples
                passed = True
            finally:
                if not passed:
                    try: snap('failure')
                    except Exception: pass
                browser.close()
    finally:
        server.shutdown()
        report = {'browser': args.browser, 'smoke': args.smoke, 'passed': passed, 'seconds': time.monotonic()-started, 'errors': errors, 'samples': samples, 'actions': actions}
        (args.output / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
        write_report(args.output, report)
        print('Report:', args.output / 'report.html')

def write_report(out, report):
    """One self-contained report.html: the run's summary and every screenshot, in order."""
    import base64, html
    esc = html.escape
    cards = []
    for n in report['samples']:
        png, js = out / f"{n['name']}.png", out / f"{n['name']}.json"
        if not png.exists(): continue
        img = base64.b64encode(png.read_bytes()).decode()
        st = json.loads(js.read_text()) if js.exists() else {}
        controls = ', '.join(f"{esc(c['label'])}{'' if c['enabled'] else ' (off)'}" for c in st.get('controls', []))
        cam = st.get('camera', {})
        rows = [('fase', st.get('phase')), ('passo', st.get('step')), ('atteso', st.get('expect')),
                ('memoria', f"{n['memory_bytes'] / 1048576:.0f} MiB"), ('schermo', cam.get('pixels')),
                ('distanza camera', cam.get('distance'))]
        facts = ''.join(f"<tr><th>{k}</th><td>{esc(str(v))}</td></tr>" for k, v in rows if v is not None)
        bad = ' bad' if n['name'] == 'failure' else ''
        cards.append(f"<figure class='card{bad}' id='{esc(n['name'])}'><a href='#{esc(n['name'])}'><img src='data:image/png;base64,{img}' alt='{esc(n['name'])}'></a>"
                     f"<figcaption><b>{esc(n['name'])}</b><table>{facts}</table><details><summary>controlli</summary>{controls}</details></figcaption></figure>")
    peak = max((n['memory_bytes'] for n in report['samples']), default=0) / 1048576
    status = 'SUPERATO' if report['passed'] else 'FALLITO'
    errors = ''.join(f"<li>{esc(e)}</li>" for e in report['errors']) or '<li>nessuno</li>'
    toc = ' · '.join(f"<a href='#{esc(n['name'])}'>{esc(n['name'])}</a>" for n in report['samples'] if (out / f"{n['name']}.png").exists())
    doc = f"""<!doctype html><html lang='it'><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'>
<title>Playtest mobile</title><style>
body{{font:14px system-ui;margin:0;padding:16px;background:#14171c;color:#e8e8e8}}
h1{{margin:0 0 8px}} .ok{{color:#6fdc8c}} .ko{{color:#ff7a7a}} a{{color:#8ab4ff}}
.grid{{display:flex;flex-wrap:wrap;gap:16px;align-items:flex-start;margin-top:16px}}
.card{{margin:0;background:#1e232b;border:1px solid #333;border-radius:10px;padding:10px;max-width:min(100%,860px)}}
.card.bad{{border-color:#ff7a7a}} img{{max-width:100%;height:auto;border-radius:6px;display:block}}
th{{text-align:left;color:#9aa;padding-right:10px;font-weight:normal}} table{{margin:6px 0}} summary{{cursor:pointer;color:#9aa}}
</style><h1>Playtest mobile <span class='{'ok' if report['passed'] else 'ko'}'>{status}</span></h1>
<p>Browser: {esc(report['browser'])}{' (smoke)' if report['smoke'] else ''} · {report['seconds']:.0f} s · picco memoria Wasm {peak:.0f} MiB · {len(report['samples'])} immagini</p>
<p>Errori del browser:</p><ul>{errors}</ul><p>{toc}</p><div class='grid'>{''.join(cards)}</div></html>"""
    (out / 'report.html').write_text(doc)


if __name__ == '__main__': main()
