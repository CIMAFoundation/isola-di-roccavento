#!/usr/bin/env python3
"""Main-thread cost of the web build, headless (no window), phone viewport.

  .venv/bin/python scripts/perf_web.py frames <site> [<site>...] [--mode title|game]
      WebKit (Safari's engine): CPU spent in each frame callback, mean and percentiles.
  .venv/bin/python scripts/perf_web.py cpu <site> --out <dir> [--mode title|game]
      Chromium CPU profile (DevTools protocol) and its time by Bevy system; with a
      wasm built with function names (docs/web.md, «CPU per frame»), else wasm-function[N].
  .venv/bin/python scripts/perf_web.py gl <site> [--mode title|game]
      WebGL2 calls per frame, and draws per shader program (Chromium).
  .venv/bin/python scripts/perf_web.py shots <site> --out <dir> --view <spec> [--view ...]
      Screenshots at fixed framings (Chromium), to compare two builds: each <spec> is
      `name:title|game@<min>:<x>:<y>:<distance>:<yaw>:<pitch>` (focus in scenario
      metres, radians; `game@60` an hour into the game). Needs a build with
      `playtest::camera` (2026-11-02).

<site> is a built site (target/web, or $CARGO_TARGET_DIR/web). Profile `q=bassa`
(the phone's) unless --query says otherwise. «title» waits --settle seconds on the
title (the demo fire burns there); «game» starts a game and lets it run, the clock
as for a player (the playtest bridge only presses the buttons, then it is removed).
The viewport is a phone's (390x844 at 3x, touch) unless --viewport says otherwise:
1000 px wide or more is a computer (mouse), e.g. `--viewport 1440x900 --dpr 1
--query q=media`.
"""
import argparse, collections, functools, http.server, json, re, statistics, threading
from pathlib import Path
from playwright.sync_api import sync_playwright

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

FRAME_TIMER = r"""
(() => {
  const raf = window.requestAnimationFrame.bind(window);
  window.__cb = [];
  window.requestAnimationFrame = (cb) => raf((t) => {
    const s = performance.now();
    try { cb(t); } finally { window.__cb.push(performance.now() - s); }
  });
})();
"""

GL_COUNTER = r"""
(() => {
  const counts = {}, draws = new Map(), names = new Map();
  let frames = 0, current = null;
  window.__gl = { counts, draws, get frames() { return frames; }, reset() { for (const k in counts) delete counts[k]; draws.clear(); frames = 0; } };
  const proto = WebGL2RenderingContext.prototype;
  for (const name of Object.getOwnPropertyNames(proto)) {
    const d = Object.getOwnPropertyDescriptor(proto, name);
    if (!d || typeof d.value !== 'function' || name === 'constructor') continue;
    const orig = d.value;
    proto[name] = function (...a) { counts[name] = (counts[name] || 0) + 1; return orig.apply(this, a); };
  }
  const useProgram = proto.useProgram;
  proto.useProgram = function (p) { current = p; return useProgram.call(this, p); };
  const label = (gl, p) => {
    if (!names.has(p)) {
      const blocks = [];
      for (let i = 0; i < gl.getProgramParameter(p, gl.ACTIVE_UNIFORM_BLOCKS); i++) blocks.push(gl.getActiveUniformBlockName(p, i));
      names.set(p, `#${names.size} attrs=${gl.getProgramParameter(p, gl.ACTIVE_ATTRIBUTES)} ${blocks.join(',').slice(0, 160)}`);
    }
    return names.get(p);
  };
  const drawEI = proto.drawElementsInstanced, drawAI = proto.drawArraysInstanced;
  const note = (gl, n) => { if (!current) return; const k = label(gl, current); const e = draws.get(k) || [0, 0]; e[0]++; e[1] += n; draws.set(k, e); };
  proto.drawElementsInstanced = function (m, c, t, o, i) { note(this, c * i); return drawEI.call(this, m, c, t, o, i); };
  proto.drawArraysInstanced = function (m, f, c, i) { note(this, c * i); return drawAI.call(this, m, f, c, i); };
  const raf = window.requestAnimationFrame.bind(window);
  const tick = () => { frames++; raf(tick); };
  raf(tick);
})();
"""


def serve(site):
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_):
            pass
    Quiet.extensions_map = {**Quiet.extensions_map, '.wasm': 'application/wasm', '.js': 'application/javascript'}
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Quiet, directory=str(site)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def new_page(pw, browser_name, args, init_script=None):
    """A browser and a page with the viewport asked (a phone's by default)."""
    launch = {'args': ['--enable-gpu', '--use-angle=metal']} if browser_name == 'chromium' else {}
    browser = getattr(pw, browser_name).launch(headless=True, **launch)
    w, h = map(int, args.viewport.split('x'))
    phone = w < 1000
    dpr = args.dpr if args.dpr else (3 if phone else 1)
    page = browser.new_page(viewport={'width': w, 'height': h}, device_scale_factor=dpr, is_mobile=phone, has_touch=phone, locale='it-IT')
    page.set_default_timeout(240000)
    page.add_init_script(MUTE)
    if init_script:
        page.add_init_script(init_script)
    return browser, page, phone


def frames(page, n=4):
    """Wait for `n` frames of the game (the playtest bridge counts them)."""
    f = page.evaluate('window.roccaPlaytest.frame')
    page.wait_for_function('f => window.roccaPlaytest.frame >= f', arg=f + n)


def start_game(page, phone):
    """From the title, through name and tutorial choice, into a running game."""
    def tap(*labels):
        # the title's «Gioca» shows only after the opening sequence
        page.wait_for_function('ls => (window.roccaPlaytest.state?.controls || []).some(c => ls.includes(c.label) && c.enabled)', arg=list(labels), timeout=60000)
        frames(page)
        cs = [c for c in page.evaluate('window.roccaPlaytest.state')['controls'] if c['label'] in labels and c['enabled']]
        assert cs, labels
        if phone:
            page.touchscreen.tap(cs[0]['x'], cs[0]['y'])
        else:
            page.mouse.click(cs[0]['x'], cs[0]['y'])
        frames(page, 8)
    tap('Gioca'); tap('Nome')
    if phone:
        # the phone types in an HTML field (its keyboard), the computer in the game's
        page.locator('#player-name').fill('Profilo')
        page.get_by_role('button', name='Continua', exact=True).click()
    else:
        page.keyboard.type('Profilo')
    frames(page, 10)
    tap('Inizia'); frames(page, 20)
    tap('Parti'); frames(page, 60)
    tap('Riprendi'); frames(page, 10)


def open_game(pw, browser_name, site, args, init_script=None):
    """A page at the state asked (`--mode`), its clock running. Returns (browser, page, server)."""
    server = serve(site)
    browser, page, phone = new_page(pw, browser_name, args, init_script)
    base = f'http://127.0.0.1:{server.server_port}/'
    if args.mode == 'title':
        page.goto(f'{base}?{args.query}')
        page.wait_for_selector('#play')
        page.click('#play')
        page.wait_for_function('window.roccaWasm && !document.querySelector("#loading")')
    else:
        page.goto(f'{base}?playtest=1&{args.query}')
        page.wait_for_function('window.roccaPlaytest?.state && !document.querySelector("#loading")')
        start_game(page, phone)
        page.evaluate('delete window.roccaPlaytest')
    page.wait_for_timeout(int(args.settle * 1000))
    return browser, page, server


def cmd_frames(args):
    with sync_playwright() as pw:
        for site in args.sites:
            browser, page, server = open_game(pw, 'webkit', site, args, FRAME_TIMER)
            page.evaluate('window.__cb = []')
            t0 = page.evaluate('performance.now()')
            page.wait_for_timeout(int(args.seconds * 1000))
            cb, t1 = page.evaluate('[window.__cb, performance.now()]')
            browser.close(); server.shutdown()
            cb = sorted(x for x in cb if x > 0.2)  # the page's own tiny callbacks
            print(f"{site.name:12s} {args.mode:5s} frame CPU mean {statistics.mean(cb):5.2f} ms  p50 {cb[len(cb) // 2]:5.2f}"
                  f"  p95 {cb[int(len(cb) * 0.95)]:5.2f}  max {cb[-1]:6.1f}  ({len(cb) / ((t1 - t0) / 1000):.0f} fps)", flush=True)


def cmd_gl(args):
    with sync_playwright() as pw:
        browser, page, server = open_game(pw, 'chromium', args.sites[0], args, GL_COUNTER)
        page.evaluate('window.__gl.reset()')
        page.wait_for_function('window.__gl.frames >= 120')
        res = page.evaluate('({counts: window.__gl.counts, frames: window.__gl.frames, draws: [...window.__gl.draws.entries()]})')
        browser.close(); server.shutdown()
    n = res['frames']
    calls = sorted(res['counts'].items(), key=lambda kv: -kv[1])
    print(f"{n} frames, {sum(v for _, v in calls) / n:.0f} WebGL calls per frame")
    for k, v in calls[:20]:
        print(f'{v / n:9.1f}  {k}')
    print('draws per frame by program (vertices drawn per frame):')
    for k, (d, v) in sorted(res['draws'], key=lambda kv: -kv[1][0]):
        print(f'{d / n:7.1f}  {v / n / 1000:8.1f} k  {k}')


def cmd_shots(args):
    args.out.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as pw:
        server = serve(args.sites[0])
        browser, page, phone = new_page(pw, 'chromium', args)
        logs = []
        page.on('console', lambda m: logs.append(m.text))
        page.goto(f'http://127.0.0.1:{server.server_port}/?playtest=1&{args.query}')
        page.wait_for_function('window.roccaPlaytest?.state && !document.querySelector("#loading")')
        page.wait_for_timeout(int(args.settle * 1000))
        in_game, minutes = False, 0.0
        for spec in args.view:
            name, phase, x, y, d, yaw, pitch = spec.split(':')
            if phase.startswith('game'):
                if not in_game:
                    page.evaluate('delete window.roccaPlaytest.camera')
                    start_game(page, phone)
                    in_game = True
                want = float(phase.partition('@')[2] or 0)
                if want > minutes:
                    page.evaluate(f'window.roccaPlaytest.advance_s = {(want - minutes) * 60}')
                    page.wait_for_function('window.roccaPlaytest.advance_s === 0')
                    minutes = want
            elif in_game:
                raise SystemExit(f'{name}: the title views come before the game ones')
            page.evaluate(f'window.roccaPlaytest.camera = {{x: {x}, y: {y}, distance: {d}, yaw: {yaw}, pitch: {pitch}}}')
            frames(page, args.hold)
            page.screenshot(path=str(args.out / f'{name}.png'))
            state = page.evaluate('window.roccaPlaytest.state')
            print(f"{name}: phase {state['phase']}, t {state['time_s'] / 60:.0f} min, camera {state['camera']}", flush=True)
        browser.close(); server.shutdown()
    for line in logs:
        if 'vegetation' in line:
            print(re.sub(r'%c|color: [^;]*;?|background: [^;]*;?|font-style: \w+', '', line).strip())


def system_of(name):
    """The Bevy system a `FunctionSystem<...>` frame runs, else None."""
    if 'FunctionSystem<' not in name:
        return None
    m = re.search(r'\(\), \(\), ([^ ]+?)> as bevy_ecs::system::system::System>', name)
    return m.group(1) if m else None


def cmd_cpu(args):
    args.out.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as pw:
        browser, page, server = open_game(pw, 'chromium', args.sites[0], args)
        cdp = page.context.new_cdp_session(page)
        cdp.send('Profiler.enable')
        cdp.send('Profiler.setSamplingInterval', {'interval': 250})
        cdp.send('Profiler.start')
        page.wait_for_timeout(int(args.seconds * 1000))
        prof = cdp.send('Profiler.stop')['profile']
        browser.close(); server.shutdown()
    (args.out / 'profile.cpuprofile').write_text(json.dumps(prof))
    nodes = {n['id']: n for n in prof['nodes']}
    parent = {c: n['id'] for n in prof['nodes'] for c in n.get('children', [])}
    clean = lambda s: re.sub(r'\[[0-9a-f]{16}\]|::h[0-9a-f]{16}$', '', s)
    total = sum(prof['timeDeltas'])
    own, systems = collections.Counter(), collections.Counter()
    for sid, dt in zip(prof['samples'], prof['timeDeltas']):
        own[clean(nodes[sid]['callFrame']['functionName']) or '(anonymous)'] += dt
        seen, cur = set(), sid
        while cur is not None:
            s = system_of(clean(nodes[cur]['callFrame']['functionName']))
            if s and s not in seen:
                systems[s] += dt
                seen.add(s)
            cur = parent.get(cur)
    lines = [f"profile {total / 1e6:.2f} s, idle {own['(idle)'] / total * 100:.1f}%", '', 'time by system (inclusive):']
    lines += [f'{t / total * 100:6.2f}%  {s}' for s, t in systems.most_common(40)]
    lines += ['', 'own time by function:']
    lines += [f'{t / total * 100:6.2f}%  {f[:170]}' for f, t in own.most_common(40)]
    text = '\n'.join(lines)
    (args.out / 'summary.txt').write_text(text)
    print(text)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('what', choices=['frames', 'cpu', 'gl', 'shots'])
    ap.add_argument('sites', type=Path, nargs='+')
    ap.add_argument('--mode', choices=['title', 'game'], default='title')
    ap.add_argument('--settle', type=float, default=25, help='seconds before measuring (the scene builds, caches fill)')
    ap.add_argument('--seconds', type=float, default=10)
    ap.add_argument('--query', default='q=bassa')
    ap.add_argument('--out', type=Path, default=Path('out/perf'))
    ap.add_argument('--viewport', default='390x844', help='CSS pixels; 1000 wide or more is a computer')
    ap.add_argument('--dpr', type=float, default=0, help='device pixel ratio (default 3 on a phone, 1 on a computer)')
    ap.add_argument('--view', action='append', default=[], help='shots: name:title|game@<min>:x:y:distance:yaw:pitch')
    ap.add_argument('--hold', type=int, default=90, help='shots: frames at the view before the picture')
    args = ap.parse_args()
    {'frames': cmd_frames, 'cpu': cmd_cpu, 'gl': cmd_gl, 'shots': cmd_shots}[args.what](args)


if __name__ == '__main__':
    main()
