//! Road grading, measured: cross slope and long grade on the roads, cut and
//! fill, build time, and how far houses move.
//! `cargo run --release -p scenario --example grade_stats`

use scenario::{Pos, Scenario};

fn pct(v: &mut [f32], q: f32) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f32 * q) as usize]
}

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let t0 = std::time::Instant::now();
    let scn = Scenario::load_by_id(&dir, scenario::id()).unwrap();
    println!("load (with grading): {:.2} s", t0.elapsed().as_secs_f32());
    let t = &scn.terrain;
    let Some(g) = &t.grade else {
        println!("grading off");
        return;
    };
    println!("segments {}", g.segs.len());

    for drivable in [true, false] {
        let (mut cross_raw, mut cross, mut long_raw, mut long, mut cut) = (vec![], vec![], vec![], vec![], vec![]);
        for s in g.segs.iter().filter(|s| s.drivable == drivable) {
            let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.5 {
                continue;
            }
            let (ux, uy) = (dx / len, dy / len);
            let m = [(s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5];
            let w = s.flat - 0.3;
            let l = Pos { x: m[0] - uy * w, y: m[1] + ux * w };
            let r = Pos { x: m[0] + uy * w, y: m[1] - ux * w };
            cross_raw.push(((t.raw_height_at(l) - t.raw_height_at(r)) / (2.0 * w)).abs() * 100.0);
            cross.push(((t.height_at(l) - t.height_at(r)) / (2.0 * w)).abs() * 100.0);
            let (a, b) = (Pos { x: s.a[0], y: s.a[1] }, Pos { x: s.b[0], y: s.b[1] });
            long_raw.push(((t.raw_height_at(b) - t.raw_height_at(a)) / len).abs() * 100.0);
            long.push(((t.height_at(b) - t.height_at(a)) / len).abs() * 100.0);
            let mp = Pos { x: m[0], y: m[1] };
            cut.push(t.height_at(mp) - t.raw_height_at(mp));
        }
        let mut abs_cut: Vec<f32> = cut.iter().map(|c| c.abs()).collect();
        println!(
            "{}: {} segs | cross slope % p50/p95/max raw {:.1}/{:.1}/{:.1} graded {:.2}/{:.2}/{:.2} | long grade % p95/max raw {:.1}/{:.1} graded {:.1}/{:.1} | |cut/fill| m p50/p95/max {:.2}/{:.2}/{:.2}",
            if drivable { "drivable" } else { "track" },
            cross.len(),
            pct(&mut cross_raw.clone(), 0.5), pct(&mut cross_raw.clone(), 0.95), pct(&mut cross_raw, 1.0),
            pct(&mut cross.clone(), 0.5), pct(&mut cross.clone(), 0.95), pct(&mut cross, 1.0),
            pct(&mut long_raw.clone(), 0.95), pct(&mut long_raw, 1.0),
            pct(&mut long.clone(), 0.95), pct(&mut long, 1.0),
            pct(&mut abs_cut.clone(), 0.5), pct(&mut abs_cut.clone(), 0.95), pct(&mut abs_cut, 1.0),
        );
    }

    // spots to look at: steep cross slopes on highways, far apart
    let mut spots: Vec<(f32, [f32; 2], f32)> = Vec::new();
    for s in g.segs.iter().filter(|s| s.drivable) {
        let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.5 { continue; }
        let m = [(s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5];
        let w = 6.0;
        let l = Pos { x: m[0] - dy / len * w, y: m[1] + dx / len * w };
        let r = Pos { x: m[0] + dy / len * w, y: m[1] - dx / len * w };
        let cross = ((t.raw_height_at(l) - t.raw_height_at(r)) / 12.0).abs();
        spots.push((cross, m, dy.atan2(dx).to_degrees()));
    }
    spots.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut chosen: Vec<(f32, [f32; 2], f32)> = Vec::new();
    for s in spots.into_iter().filter(|s| s.0 < 0.8) {
        if chosen.iter().all(|c| (c.1[0] - s.1[0]).hypot(c.1[1] - s.1[1]) > 600.0) { chosen.push(s); }
        if chosen.len() == 6 { break; }
    }
    for c in &chosen { println!("spot: cross {:.0}% at {:.0}:{:.0} heading {:.0}°", c.0 * 100.0, c.1[0], c.1[1], c.2); }

    // houses: how much the ground under them moved
    let mut moved: Vec<f32> = scn
        .vectors
        .buildings
        .iter()
        .map(|b| {
            let p = Pos { x: b.centroid[0], y: b.centroid[1] };
            (t.height_at(p) - t.raw_height_at(p)).abs()
        })
        .collect();
    let n = moved.len();
    let over = moved.iter().filter(|m| **m > 0.5).count();
    println!("houses {n}: ground moved p50/p95/max {:.2}/{:.2}/{:.2} m, {over} by more than 0.5 m", pct(&mut moved.clone(), 0.5), pct(&mut moved.clone(), 0.95), pct(&mut moved, 1.0));

    // continuity: the steepest step over 0.25 m anywhere near the roads
    let mut worst = (0.0f32, [0.0f32; 2]);
    for s in &g.segs {
        for k in 0..40 {
            let ang = k as f32 * 0.7;
            let r = (k as f32 * 0.9) % (s.flat + 32.0);
            let p = Pos { x: s.a[0] + r * ang.cos(), y: s.a[1] + r * ang.sin() };
            let q = Pos { x: p.x + 0.25, y: p.y };
            let step = (t.height_at(q) - t.height_at(p)).abs() / 0.25;
            if step > worst.0 {
                worst = (step, [p.x, p.y]);
            }
        }
    }
    // ground made steeper than 1.5 m/m (56°) by the grading, on the DEM grid
    let (mut steep_raw, mut steep) = (0, 0);
    for r in 1..t.rows - 1 {
        for c in 1..t.cols - 1 {
            let p = Pos { x: c as f32 * t.posting, y: t.height_m - r as f32 * t.posting };
            if g.near([p.x, p.y]).is_empty() { continue; }
            let d = 2.5;
            let sl = |h: &dyn Fn(Pos) -> f32| {
                let gx = (h(Pos { x: p.x + d, ..p }) - h(Pos { x: p.x - d, ..p })) / (2.0 * d);
                let gy = (h(Pos { y: p.y + d, ..p }) - h(Pos { y: p.y - d, ..p })) / (2.0 * d);
                (gx * gx + gy * gy).sqrt()
            };
            steep_raw += (sl(&|q| t.raw_height_at(q)) > 1.5) as usize;
            steep += (sl(&|q| t.height_at(q)) > 1.5) as usize;
        }
    }
    println!("near-road samples steeper than 1.5 m/m: raw {steep_raw}, graded {steep}");
    println!("steepest local slope near roads: {:.2} m/m at {:?}", worst.0, worst.1);
    if std::env::var("PROBE").is_ok() { probe(t, worst.1); }

    let t1 = std::time::Instant::now();
    let mut acc = 0.0;
    for i in 0..1_000_000 {
        let s = &g.segs[i % g.segs.len()];
        acc += t.height_at(Pos { x: s.a[0] + (i % 17) as f32, y: s.a[1] });
    }
    println!("height_at near roads: {:.0} ns per call ({acc:.0})", t1.elapsed().as_secs_f64() * 1e9 / 1e6);
}

#[allow(dead_code)]
pub fn probe(t: &scenario::Terrain, p: [f32; 2]) {
    for k in -8..=8 {
        let q = Pos { x: p[0] + k as f32 * 0.5, y: p[1] };
        println!("  x {:.1}: graded {:.2} raw {:.2}", q.x, t.height_at(q), t.raw_height_at(q));
    }
    if let Some(g) = &t.grade {
        for dx in [-0.5f32, 0.0, 0.5] {
            let q = Pos { x: p[0] + dx, y: p[1] };
            let z = g.apply_with([q.x, q.y], t.raw_height_at(q), |k, out, zs, a, w| {
                if w > 1e-3 { println!("    x {:.1} seg {k} out {out:.2} z {zs:.2} a {a:.3} w {w:.4}", q.x) }
            });
            println!("   => {z:.2}");
        }
        for &k in g.near(p) {
            let s = &g.segs[k as usize];
            let (d, z) = s.nearest(p);
            if d < 40.0 {
                println!("  seg {k} d {d:.1} z {z:.2} flat {} a {:?} b {:?}", s.flat, s.a, s.b);
            }
        }
    }
}
