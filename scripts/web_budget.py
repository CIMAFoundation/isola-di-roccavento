#!/usr/bin/env python3
"""What a built site's page costs in memory, at rest in a game. Headless
Chromium, no window, no sound (the helpers of scripts/perf_web.py).

  .venv/bin/python scripts/web_budget.py <site> <query> [phone] [top]
      e.g. target/web 'scans=1' phone

Prints, for the page with those address parameters:
- what it asks of the GPU: every texture and buffer alive, in MB (the WebGL2
  calls are wrapped in the page: the sizes asked for, not what a driver
  keeps); with "top", the largest textures one by one, the buffers by the
  size of their vertex (on WebGL2 every mesh has its own vertex buffer, so
  a vertex size is a kind of mesh), and the categories of the graphics
  process's memory;
- the page's own memory: the wasm's linear memory and the data files still
  held in JavaScript;
- the browser's renderer and GPU processes (the macOS footprint tool,
  phys_footprint; elsewhere their resident size), and of the GPU process
  the memory taken from the graphics card (IOAccelerator): it can be
  several times what the page asked for (2026-11-13: 845 MB for 197 asked,
  the browser rewriting every draw's indices for WebGL's provoking vertex;
  see web/index.html).

A phone closes a page that takes too much and says nothing: this is how a
level's cost is known before it is tried on one (2026-11-13: the realistic
style at the computers' level asked 444 MB of the GPU where a phone's page
asks 150, and an iPhone closed it). "phone": a phone's viewport and touch,
so the page takes its light profile. Only WebGL2 is counted (gpu=0 is added
on a computer's viewport).
"""
import argparse, re, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import perf_web as pw
from playwright.sync_api import sync_playwright

# Every texture, renderbuffer and buffer alive, by the size it was made with.
ACCOUNT = r"""
(() => {
  const P = WebGL2RenderingContext.prototype;
  const S = window.__gl = { tex: new Map(), buf: new Map(), bt: {}, bb: {}, rb: new Map(), brb: null, stride: new Map() };
  // bytes a texel, by internal format (RGBA8, SRGB8_ALPHA8, RGBA16F, R16F, RG16F, R32F, DEPTH32F, ...)
  const bpp = f => ({0x8058:4,0x8C43:4,0x881A:8,0x822D:2,0x822F:4,0x822E:4,0x8CAC:4,0x81A6:4,0x8229:1,0x822B:2,0x8814:16,0x8C3A:4,0x8059:4,0x88F0:4,0x8CAD:5,0x81A5:2,0x8D62:2,0x8051:3,0x8C41:3}[f] || 4);
  const mips = (levels, three) => levels > 1 ? (three ? 8 / 7 : 4 / 3) : 1;
  const wrap = (name, fn) => { const o = P[name]; P[name] = function (...a) { try { fn.apply(this, a); } catch (e) {} return o.apply(this, a); }; };
  wrap('bindTexture', (t, x) => { S.bt[t] = x; });
  wrap('bindBuffer', (t, x) => { S.bb[t] = x; });
  wrap('bindRenderbuffer', (t, x) => { S.brb = x; });
  wrap('texStorage2D', (t, l, f, w, h) => { S.tex.set(S.bt[t], { mb: w * h * bpp(f) * mips(l) * (t === 0x8513 ? 6 : 1) / 1048576, d: `${w}x${h}${t === 0x8513 ? ' cube' : ''} fmt 0x${f.toString(16)} levels ${l}` }); });
  wrap('texStorage3D', (t, l, f, w, h, d) => { S.tex.set(S.bt[t], { mb: w * h * d * bpp(f) * mips(l, t === 0x806F) / 1048576, d: `${w}x${h}x${d} fmt 0x${f.toString(16)} levels ${l}` }); });
  wrap('renderbufferStorageMultisample', (t, s, f, w, h) => { S.rb.set(S.brb, { mb: w * h * bpp(f) * Math.max(s, 1) / 1048576, d: `renderbuffer ${w}x${h} x${s} fmt 0x${f.toString(16)}` }); });
  wrap('renderbufferStorage', (t, f, w, h) => { S.rb.set(S.brb, { mb: w * h * bpp(f) / 1048576, d: `renderbuffer ${w}x${h} fmt 0x${f.toString(16)}` }); });
  S.made = { buffers: 0, buffer_mb: 0, written: 0, written_mb: 0, texels: 0 };
  wrap('bufferData', (t, x) => { const mb = (typeof x === 'number' ? x : (x ? x.byteLength : 0)) / 1048576; S.buf.set(S.bb[t], mb); S.made.buffers++; S.made.buffer_mb += mb; });
  wrap('bufferSubData', (t, o, x) => { S.made.written++; S.made.written_mb += (x ? x.byteLength : 0) / 1048576; });
  wrap('texSubImage2D', () => { S.made.texels++; });
  wrap('texSubImage3D', () => { S.made.texels++; });
  // the vertex a buffer is read with: its stride and how many attributes
  const read = function (i, n, t, a, b, c) { const s = typeof a === 'boolean' ? b : a; const x = S.bb[0x8892]; if (x) S.stride.set(x, s); };
  wrap('vertexAttribPointer', read);
  wrap('vertexAttribIPointer', read);
  wrap('deleteTexture', x => { S.tex.delete(x); });
  wrap('deleteBuffer', x => { S.buf.delete(x); });
  wrap('deleteRenderbuffer', x => { S.rb.delete(x); });
  window.__glReport = () => {
    const tex = [...S.tex.values(), ...S.rb.values()].sort((a, b) => b.mb - a.mb);
    const buf = [...S.buf.values()].sort((a, b) => b - a);
    const kinds = {};
    for (const [b, mb] of S.buf) { const k = S.stride.get(b) ?? 0; const e = kinds[k] ??= [0, 0, 0]; e[0]++; e[1] += mb; e[2] = Math.max(e[2], mb); }
    return { kinds, textures: tex.length, tex_mb: tex.reduce((s, t) => s + t.mb, 0), top: tex.slice(0, 14).map(t => `${t.mb.toFixed(1)} MB  ${t.d}`),
             buffers: buf.length, buf_mb: buf.reduce((s, b) => s + b, 0), top_buf: buf.slice(0, 8).map(b => b.toFixed(1)) };
  };
})();
"""


def processes():
    """The headless browser's renderer and GPU processes: the largest of each kind."""
    out = subprocess.run(['ps', '-Ao', 'pid,rss,command'], capture_output=True, text=True).stdout
    found = {}
    for line in out.splitlines():
        m = re.match(r'\s*(\d+)\s+(\d+)\s+(.*)', line)
        if not m or 'headless' not in m[3]:
            continue
        kind = re.search(r'--type=([\w-]+)', m[3])
        kind = kind[1] if kind else 'browser'
        if kind not in found or int(m[2]) > found[kind][1]:
            found[kind] = (int(m[1]), int(m[2]))
    return found


def footprint(pid):
    try:
        out = subprocess.run(['footprint', '-p', str(pid)], capture_output=True, text=True).stdout
    except FileNotFoundError:
        return None
    m = re.search(r'phys_footprint:\s*([\d.]+)\s*(\w+)', out)
    return float(m[1]) * {'KB': 1 / 1024, 'MB': 1, 'GB': 1024}.get(m[2], 1) if m else None


def main():
    site, query = Path(sys.argv[1]), sys.argv[2]
    phone = 'phone' in sys.argv[3:]
    args = argparse.Namespace(viewport='390x844', dpr=3) if phone else argparse.Namespace(viewport='1440x900', dpr=1)
    if not phone and 'gpu=' not in query:
        query += '&gpu=0'
    with sync_playwright() as p:
        server = pw.serve(site)
        browser, page, touch = pw.new_page(p, 'chromium', args, ACCOUNT)
        page.goto(f'http://127.0.0.1:{server.server_port}/?playtest=1&{query}')
        page.wait_for_function('window.roccaPlaytest?.state && !document.querySelector("#loading")', timeout=240000)
        page.evaluate('delete window.roccaPlaytest.camera')
        pw.start_game(page, touch)
        page.evaluate('window.roccaPlaytest.advance_s = 2400')
        page.wait_for_function('window.roccaPlaytest.advance_s === 0', timeout=240000)
        pw.frames(page, 200)
        time.sleep(3)
        r = page.evaluate('window.__glReport()')
        held = page.evaluate("""({wasm: window.roccaPlaytest.state.wasm_memory_bytes / 1048576, provoking: window.roccaProvoking,
            data: Object.values(window.roccaData ?? {}).reduce((s, b) => s + (b?.byteLength ?? 0), 0) / 1048576})""")
        line = f"{query:<24} GPU asked: {r['textures']} textures {r['tex_mb']:.0f} MB, {r['buffers']} buffers {r['buf_mb']:.0f} MB"
        line += f";  wasm {held['wasm']:.0f} MB, data held {held['data']:.0f} MB"
        for kind, name in (('renderer', 'page'), ('gpu-process', 'graphics process')):
            pid, rss = processes().get(kind, (0, 0))
            fp = footprint(pid) if pid else None
            line += f";  {name} {fp:.0f} MB" if fp else f";  {name} {rss / 1024:.0f} MB resident"
            if kind == 'gpu-process' and pid:
                out = subprocess.run(['footprint', '-p', str(pid)], capture_output=True, text=True).stdout
                m = re.search(r'^\s*([\d.]+) (KB|MB|GB)\s.*IOAccelerator \(graphics\)', out, re.M)
                if m:
                    line += f" (from the graphics card {float(m[1]) * {'KB': 1 / 1024, 'MB': 1, 'GB': 1024}[m[2]]:.0f})"
        line += f";  provoking vertex: {held['provoking']}"
        print(line)
        if 'top' in sys.argv[3:]:
            for t in r['top']:
                print('   ', t)
            print('    largest buffers (MB):', ', '.join(r['top_buf']))
            print('    buffers by bytes a vertex (0: indices, uniforms and the rest):')
            for k, (n, mb, big) in sorted(r['kinds'].items(), key=lambda kv: -kv[1][1]):
                print(f'      {int(k):>3} B: {n:>5} buffers, {mb:6.1f} MB, the largest {big:.1f}')
            pid = processes().get('gpu-process', (0, 0))[0]
            out = subprocess.run(['footprint', '-p', str(pid)], capture_output=True, text=True).stdout if pid else ''
            rows = re.findall(r'^\s*([\d.]+) (KB|MB|GB)\s+[\d.]+ \w+\s+[\d.]+ \w+\s+\d+\s+(.+)$', out, re.M)
            rows = sorted(((float(v) * {'KB': 1 / 1024, 'MB': 1, 'GB': 1024}[u], name.strip()) for v, u, name in rows), reverse=True)
            print('    the graphics process, by category:', '; '.join(f'{name} {mb:.0f} MB' for mb, name in rows[:7]))
        browser.close(); server.shutdown()


if __name__ == '__main__':
    main()
