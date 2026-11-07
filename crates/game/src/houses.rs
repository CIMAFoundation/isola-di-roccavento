//! The houses of the towns in the realistic style (`KIOSK_SCANS=1`,
//! 2026-11-06): built facade by facade instead of extruded boxes.
//!
//! Every footprint of the scenario is a rectangle, and each keeps its place
//! and its size (the fire and the exposure of the houses read them from the
//! data, never from here). What changes is what stands on it, chosen from the
//! house's id, its tag and where it is in its town, always the same for the
//! same house:
//!
//! - **types**: the tall narrow terrace house of the old centres (*casa a
//!   schiera*), the *palazzotto* with its cornice and painted frames, the
//!   house with a loggia or balconies, the low detached house with a hipped
//!   roof, the rustic house in bare stone at the edge of the town, the shop
//!   with its window and awning;
//! - **roofs**: two slopes (along or across), hipped, flat terrace behind a
//!   parapet (sometimes with a little room on it), in clay or slate;
//! - **walls**: windows set into the wall (jambs, sill, head), louvred
//!   shutters open, closed or half, frames and bars, sills of slate or
//!   marble, doors with steps, stone portals and fanlights, string courses,
//!   cornices of three steps, painted quoins and frames (trompe-l'œil), a
//!   few painted false windows, balconies with iron railings, chimneys;
//! - **colours**: the Ligurian washes (ochre, old rose, Genoa yellow,
//!   Pompeian red, cream, straw...), and how worn each house is (damp at the
//!   foot, flaked plaster: `shaders/house.wesl`, through the house's uv).
//!
//! All of a house lands in its own vertex range of the chunk, its glass in
//! one run inside it, so the fire's recolouring and the lit windows of
//! `buildings::damage` work as they did.

use std::collections::HashMap;

use scenario::{Building, Pos, Scenario};

use super::{hash01, linear, Builder, GLASS, SHUTTER, STOREY_M, TOY_HEIGHT, TOY_SCALE};
use crate::materials::house::HousePart;
use crate::materials::masonry::Part;

const METAL: u8 = 2;
const CLOTH: u8 = 7;
const WOOD: u8 = 10;

/// What the houses need to know of their town: where each town's centre is
/// and how far it reaches, and the roads (a house turns its door to the
/// nearest).
pub(super) struct Context {
    centres: HashMap<String, (Pos, f32)>,
    roads: Vec<[[f32; 2]; 2]>,
}

impl Context {
    pub(super) fn new(scn: &Scenario) -> Context {
        let mut by_town: HashMap<String, Vec<Pos>> = HashMap::new();
        for b in &scn.vectors.buildings {
            if let Some(l) = &b.locality {
                by_town.entry(l.clone()).or_default().push(Pos { x: b.centroid[0], y: b.centroid[1] });
            }
        }
        let mut centres = HashMap::new();
        for (name, ps) in by_town {
            let n = ps.len() as f32;
            let c = Pos { x: ps.iter().map(|p| p.x).sum::<f32>() / n, y: ps.iter().map(|p| p.y).sum::<f32>() / n };
            let mut d: Vec<f32> = ps.iter().map(|p| ((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt()).collect();
            d.sort_by(|a, b| a.total_cmp(b));
            let reach = d.get(d.len() * 4 / 5).copied().unwrap_or(100.0).max(30.0);
            centres.insert(name, (c, reach));
        }
        let mut roads = Vec::new();
        for r in &scn.vectors.roads {
            for w in r.line.windows(2) {
                roads.push([w[0], w[1]]);
            }
        }
        Context { centres, roads }
    }

    /// How far out in its town `b` stands: 0 at the centre, 1 at the edge of
    /// four houses in five.
    pub(super) fn outskirts(&self, b: &Building) -> f32 {
        let Some((c, r)) = b.locality.as_ref().and_then(|l| self.centres.get(l)) else { return 0.5 };
        ((b.centroid[0] - c.x).powi(2) + (b.centroid[1] - c.y).powi(2)).sqrt() / r
    }

    pub(super) fn road_distance(&self, p: [f32; 2]) -> f32 {
        let mut best = f32::MAX;
        for [a, b] in &self.roads {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l2 = (dx * dx + dy * dy).max(1e-6);
            let k = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0);
            let (qx, qy) = (a[0] + dx * k - p[0], a[1] + dy * k - p[1]);
            best = best.min(qx * qx + qy * qy);
        }
        best.sqrt()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Type {
    /// Tall and narrow, of the old centres.
    Schiera,
    /// A town house of some standing: cornice, bands, painted frames.
    Palazzotto,
    /// An ordinary house of two or three storeys.
    Casa,
    /// With a loggia under the roof, or balconies.
    Loggia,
    /// Low and detached, hipped.
    Bassa,
    /// Bare stone, at the edge of the town.
    Rustico,
    /// A shop on the ground floor, flats above.
    Bottega,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Roof {
    /// Two slopes, the ridge along the long side (or across, `across`).
    Gable { across: bool },
    Hip,
    /// Flat, behind a parapet.
    Terrace,
}

/// Everything about a house that is chosen once (also by the downloaded
/// models, `house_models`: their type, storeys and colours).
pub(super) struct Style {
    pub(super) ty: Type,
    pub(super) storeys: usize,
    roof: Roof,
    pub(super) slate: bool,
    pub(super) wall: [f32; 3],
    pub(super) trim: [f32; 3],
    pub(super) roof_c: [f32; 3],
    pub(super) shutters: Option<[f32; 3]>,
    pub(super) frame: [f32; 3],
    pub(super) sill: [f32; 3],
    pub(super) door: [f32; 3],
    /// Painted frames round the windows.
    painted_frames: bool,
    /// Painted quoins at the corners.
    quoins: bool,
    /// String courses at each floor.
    bands: bool,
    /// A cornice of three steps under the eave (else a plain one).
    cornice: bool,
    /// 0 fresh .. 1 old (`house.wesl`).
    pub(super) worn: f32,
}

/// Ligurian washes (sRGB, for the photographed plaster: a little lighter
/// than the colour seen, the photograph adds its shade).
const WALLS: [[f32; 3]; 14] = [
    [0.88, 0.70, 0.44], // ochre
    [0.92, 0.78, 0.54], // pale ochre
    [0.94, 0.80, 0.46], // Genoa yellow
    [0.86, 0.64, 0.58], // old rose
    [0.89, 0.62, 0.48], // salmon
    [0.70, 0.36, 0.28], // Pompeian red
    [0.78, 0.46, 0.36], // faded Genoa red
    [0.93, 0.89, 0.79], // cream
    [0.91, 0.83, 0.63], // straw
    [0.77, 0.78, 0.69], // grey-green
    [0.93, 0.75, 0.60], // peach
    [0.80, 0.56, 0.38], // sienna
    [0.74, 0.79, 0.80], // pale Ligurian blue
    [0.88, 0.72, 0.66], // dusty pink
];

pub(super) const SHUTTERS: [[f32; 3]; 7] = [
    [0.18, 0.36, 0.24], // Genoa green
    [0.12, 0.26, 0.18], // dark green
    [0.42, 0.52, 0.40], // sage
    [0.40, 0.25, 0.14], // brown
    [0.30, 0.20, 0.12], // walnut
    [0.55, 0.57, 0.54], // grey
    [0.22, 0.32, 0.36], // grey-blue
];

const CLAY: [[f32; 3]; 7] = [
    [0.64, 0.33, 0.22],
    [0.70, 0.39, 0.26],
    [0.57, 0.30, 0.22],
    [0.68, 0.44, 0.32],
    [0.53, 0.35, 0.28],
    [0.61, 0.37, 0.28],
    [0.50, 0.40, 0.34],
];

const SLATE: [[f32; 3]; 3] = [[0.42, 0.43, 0.45], [0.35, 0.36, 0.38], [0.47, 0.46, 0.45]];

const DOORS: [[f32; 3]; 5] = [[0.36, 0.22, 0.12], [0.14, 0.24, 0.16], [0.36, 0.12, 0.10], [0.50, 0.36, 0.22], [0.22, 0.16, 0.12]];

fn pick<const N: usize>(v: &[[f32; 3]; N], h: f32) -> [f32; 3] {
    v[((h * N as f32) as usize).min(N - 1)]
}

/// The colour `c` (sRGB) a little off, by `h` in 0..1, as no two washes are
/// the same.
fn jitter(c: [f32; 3], h: f32, k: f32) -> [f32; 3] {
    let f = 1.0 + (h - 0.5) * k;
    let g = 1.0 + (hash01((h * 1e6) as u64, 0x77) - 0.5) * k * 0.5;
    [(c[0] * f).min(1.0), (c[1] * f * g).min(1.0), (c[2] * f / g).min(1.0)]
}

pub(super) fn style(b: &Building, area: f32, outskirts: f32) -> Style {
    let h = |s: u64| hash01(b.id as u64, 0x4800 + s);
    let tag = b.kind.as_deref().unwrap_or("house");
    let ty = match tag {
        "terrace" => Type::Schiera,
        "shop" => Type::Bottega,
        "apartments" => Type::Palazzotto,
        "villa" if outskirts > 0.8 && h(1) < 0.5 => Type::Rustico,
        "villa" if h(2) < 0.3 => Type::Loggia,
        "villa" => Type::Bassa,
        _ if outskirts > 0.95 && h(1) < 0.35 => Type::Rustico,
        _ if area > 105.0 && h(2) < 0.45 => Type::Palazzotto,
        _ if h(3) < 0.4 => Type::Loggia,
        _ if h(3) < 0.55 => Type::Bassa,
        _ => Type::Casa,
    };
    let storeys = match ty {
        Type::Schiera => 3 + (h(4) < 0.45) as usize,
        Type::Palazzotto => 3 + (h(4) < 0.5) as usize,
        Type::Bottega => 3 + (h(4) < 0.3) as usize,
        Type::Casa => 2 + (h(4) < 0.5) as usize,
        Type::Loggia => 2 + (h(4) < 0.45) as usize,
        Type::Bassa => 1 + (h(4) < 0.7) as usize,
        Type::Rustico => 1 + (h(4) < 0.6) as usize,
    };
    let r = h(5);
    let roof = match ty {
        Type::Schiera | Type::Bottega => {
            if r < 0.45 { Roof::Gable { across: h(6) < 0.3 } } else if r < 0.75 { Roof::Hip } else { Roof::Terrace }
        }
        Type::Palazzotto => if r < 0.7 { Roof::Hip } else { Roof::Gable { across: false } },
        Type::Casa => if r < 0.42 { Roof::Hip } else if r < 0.88 { Roof::Gable { across: h(6) < 0.2 } } else { Roof::Terrace },
        Type::Loggia => if r < 0.6 { Roof::Gable { across: false } } else { Roof::Hip },
        Type::Bassa => if r < 0.68 { Roof::Hip } else if r < 0.9 { Roof::Gable { across: false } } else { Roof::Terrace },
        Type::Rustico => if r < 0.85 { Roof::Gable { across: false } } else { Roof::Hip },
    };
    let slate = match ty {
        Type::Palazzotto => h(7) < 0.4,
        Type::Schiera | Type::Bottega => h(7) < 0.28,
        Type::Casa | Type::Loggia => h(7) < 0.18,
        Type::Bassa => h(7) < 0.08,
        Type::Rustico => false,
    };
    let wall = match ty {
        Type::Rustico => jitter([0.62, 0.58, 0.52], h(8), 0.2),
        _ => jitter(pick(&WALLS, h(9)), h(8), 0.12),
    };
    let trim = if h(10) < 0.55 {
        jitter([0.93, 0.91, 0.85], h(11), 0.06)
    } else if h(10) < 0.75 {
        jitter([0.82, 0.80, 0.74], h(11), 0.06)
    } else {
        // a shade of the wash, lighter or darker
        let k = if h(11) < 0.5 { 1.14 } else { 0.8 };
        [(wall[0] * k).min(1.0), (wall[1] * k).min(1.0), (wall[2] * k).min(1.0)]
    };
    let shutters = match ty {
        Type::Rustico => (h(12) < 0.7).then(|| jitter(pick(&[SHUTTERS[3], SHUTTERS[4]], h(13)), h(14), 0.2)),
        _ => (h(12) < 0.88).then(|| jitter(pick(&SHUTTERS, h(13)), h(14), 0.15)),
    };
    let frame = match (ty, shutters) {
        (Type::Rustico, _) => [0.38, 0.26, 0.16],
        (_, Some(s)) if h(15) < 0.2 => s,
        _ => [0.86, 0.84, 0.79],
    };
    let sill = if ty == Type::Rustico || h(16) < 0.55 { [0.36, 0.36, 0.37] } else { [0.80, 0.78, 0.73] };
    let roof_c = if slate { jitter(pick(&SLATE, h(17)), h(18), 0.1) } else { jitter(pick(&CLAY, h(17)), h(18), 0.12) };
    let worn = match ty {
        Type::Rustico => 0.4 + 0.6 * h(19),
        Type::Palazzotto => h(19) * 0.9,
        _ => h(19),
    };
    let fancy = matches!(ty, Type::Palazzotto | Type::Schiera | Type::Bottega | Type::Casa);
    Style {
        ty,
        storeys,
        roof,
        slate,
        wall,
        trim,
        roof_c,
        shutters,
        frame,
        sill,
        door: pick(&DOORS, h(20)),
        painted_frames: ty != Type::Rustico && h(21) < if ty == Type::Palazzotto { 0.9 } else { 0.6 },
        quoins: fancy && h(22) < if ty == Type::Palazzotto { 0.7 } else { 0.3 },
        bands: match ty {
            Type::Palazzotto => h(23) < 0.85,
            Type::Rustico | Type::Bassa => false,
            _ => h(23) < 0.35,
        },
        cornice: ty != Type::Rustico && (ty == Type::Palazzotto || h(24) < 0.6),
        worn,
    }
}

/// The house's own frame: its centre, `u` along the long side, `v` across.
#[derive(Clone, Copy)]
struct Frame {
    c: [f32; 2],
    u: [f32; 2],
    v: [f32; 2],
}

impl Frame {
    fn w(&self, s: f32, t: f32, y: f32) -> [f32; 3] {
        [self.c[0] + s * self.u[0] + t * self.v[0], y, -(self.c[1] + s * self.u[1] + t * self.v[1])]
    }
    fn dir(&self, s: f32, t: f32, y: f32) -> [f32; 3] {
        [s * self.u[0] + t * self.v[0], y, -(s * self.u[1] + t * self.v[1])]
    }
    fn ground(&self, scn: &Scenario, s: f32, t: f32) -> f32 {
        let p = self.w(s, t, 0.0);
        scn.terrain.height_at(Pos { x: p[0], y: -p[2] })
    }
}

/// One side of the house: starting at `o` (in the frame), running along `d`
/// for `len`, facing `n`.
#[derive(Clone, Copy)]
struct Facade {
    o: [f32; 2],
    d: [f32; 2],
    n: [f32; 2],
    len: f32,
}

impl Facade {
    fn st(&self, x: f32, dd: f32) -> (f32, f32) {
        (self.o[0] + self.d[0] * x + self.n[0] * dd, self.o[1] + self.d[1] * x + self.n[1] * dd)
    }
}

/// A point of facade `f`: `x` along it, at height `y`, `dd` out of the wall.
fn fp(fr: &Frame, f: &Facade, x: f32, y: f32, dd: f32) -> [f32; 3] {
    let (s, t) = f.st(x, dd);
    fr.w(s, t, y)
}

/// A quad wound to face along `n`.
pub(super) fn face(out: &mut Builder, q: [[f32; 3]; 4], n: [f32; 3], col: [f32; 3]) {
    let e1 = [q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]];
    let e2 = [q[2][0] - q[0][0], q[2][1] - q[0][1], q[2][2] - q[0][2]];
    let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    if c[0] * n[0] + c[1] * n[1] + c[2] * n[2] >= 0.0 {
        out.quad(q[0], q[1], q[2], q[3], col);
    } else {
        out.quad(q[0], q[3], q[2], q[1], col);
    }
}

/// A triangle wound to face along `n`.
fn tri(out: &mut Builder, a: [f32; 3], b: [f32; 3], c: [f32; 3], n: [f32; 3], col: [f32; 3]) {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let x = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    if x[0] * n[0] + x[1] * n[1] + x[2] * n[2] >= 0.0 {
        out.tri(a, b, c, col);
    } else {
        out.tri(a, c, b, col);
    }
}

const FRONT: u8 = 1;
const TOP: u8 = 2;
const BOTTOM: u8 = 4;
const ENDS: u8 = 8;
const BACK: u8 = 16;

/// A box on facade `f`: `x0..x1` along it, `y0..y1` up, `d0..d1` out of it.
#[allow(clippy::too_many_arguments)]
fn fbox(out: &mut Builder, fr: &Frame, f: &Facade, x0: f32, x1: f32, y0: f32, y1: f32, d0: f32, d1: f32, faces: u8, col: [f32; 3]) {
    let p = |x: f32, y: f32, d: f32| fp(fr, f, x, y, d);
    let nout = fr.dir(f.n[0], f.n[1], 0.0);
    let along = fr.dir(f.d[0], f.d[1], 0.0);
    if faces & FRONT != 0 {
        face(out, [p(x0, y0, d1), p(x1, y0, d1), p(x1, y1, d1), p(x0, y1, d1)], nout, col);
    }
    if faces & BACK != 0 {
        face(out, [p(x0, y0, d0), p(x1, y0, d0), p(x1, y1, d0), p(x0, y1, d0)], nout.map(|v| -v), col);
    }
    if faces & TOP != 0 {
        face(out, [p(x0, y1, d0), p(x1, y1, d0), p(x1, y1, d1), p(x0, y1, d1)], [0.0, 1.0, 0.0], col);
    }
    if faces & BOTTOM != 0 {
        face(out, [p(x0, y0, d0), p(x1, y0, d0), p(x1, y0, d1), p(x0, y0, d1)], [0.0, -1.0, 0.0], col);
    }
    if faces & ENDS != 0 {
        face(out, [p(x0, y0, d0), p(x0, y0, d1), p(x0, y1, d1), p(x0, y1, d0)], along.map(|v| -v), col);
        face(out, [p(x1, y0, d0), p(x1, y0, d1), p(x1, y1, d1), p(x1, y1, d0)], along, col);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hole {
    Window,
    /// A window down to the floor, onto a balcony.
    French,
    Door,
    Shop,
    /// An open arch of a loggia, deep, with the room's wall behind.
    Loggia,
}

#[derive(Clone, Copy)]
struct Opening {
    x0: f32,
    x1: f32,
    y0: f32,
    /// The top: of the rectangle, or the crown of the arch.
    y1: f32,
    arch: bool,
    depth: f32,
    hole: Hole,
    /// 0 open, 1 closed, 2 one leaf closed (shutters).
    shut: u8,
    balcony: bool,
}

impl Opening {
    /// Where the arch springs (the rectangle's top without one).
    fn spring(&self) -> f32 {
        if self.arch { self.y1 - (self.x1 - self.x0) * 0.5 } else { self.y1 }
    }
}

/// A painted false window (trompe-l'œil): closed shutters and a frame.
#[derive(Clone, Copy)]
struct Fake {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

const ARCH_SEGS: usize = 8;

fn arc(o: &Opening) -> Vec<(f32, f32)> {
    let (xc, r, ys) = ((o.x0 + o.x1) * 0.5, (o.x1 - o.x0) * 0.5, o.spring());
    (0..=ARCH_SEGS)
        .map(|k| {
            let a = std::f32::consts::PI * k as f32 / ARCH_SEGS as f32;
            (xc - r * a.cos(), ys + r * a.sin())
        })
        .collect()
}

/// Draw the house on footprint `b` (a rectangle). Returns the vertex range
/// of its glass, or `None` to fall back to the plain extrusion.
pub(super) fn emit(scn: &Scenario, b: &Building, ctx: &Context, ring: &[Pos], out: &mut Builder) -> Option<(u32, u32)> {
    if ring.len() != 4 {
        return None;
    }
    let area = b.area();
    // the frame, from the longest edge of the (real) footprint
    let mut best = (0.0, [1.0, 0.0]);
    for i in 0..4 {
        let (a, c) = (ring[i], ring[(i + 1) % 4]);
        let (dx, dy) = (c.x - a.x, c.y - a.y);
        let l = (dx * dx + dy * dy).sqrt();
        if l > best.0 {
            best = (l, [dx / l, dy / l]);
        }
    }
    let u = best.1;
    let v = [-u[1], u[0]];
    let c0 = [ring.iter().map(|p| p.x).sum::<f32>() / 4.0, ring.iter().map(|p| p.y).sum::<f32>() / 4.0];
    let (mut smin, mut smax, mut tmin, mut tmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in ring {
        let (s, t) = ((p.x - c0[0]) * u[0] + (p.y - c0[1]) * u[1], (p.x - c0[0]) * v[0] + (p.y - c0[1]) * v[1]);
        smin = smin.min(s);
        smax = smax.max(s);
        tmin = tmin.min(t);
        tmax = tmax.max(t);
    }
    let (sm, tm) = ((smin + smax) * 0.5, (tmin + tmax) * 0.5);
    let fr = Frame { c: [c0[0] + u[0] * sm + v[0] * tm, c0[1] + u[1] * sm + v[1] * tm], u, v };
    // the diorama's footprint magnification (`TOY_SCALE`), as the plain houses
    let (a_half, b_half) = ((smax - smin) * 0.5 * TOY_SCALE, (tmax - tmin) * 0.5 * TOY_SCALE);
    if a_half < 2.0 || b_half < 2.0 {
        return None;
    }

    let st = style(b, area, ctx.outskirts(b));
    let h = |s: u64| hash01(b.id as u64, 0x4900 + s);

    // ground: the base below the lowest corner, the floors from the highest
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (s, t) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (0.0, 0.0)] {
        let g = fr.ground(scn, s * a_half, t * b_half);
        lo = lo.min(g);
        hi = hi.max(g);
    }
    let base = lo - 1.5;
    let storey = STOREY_M * TOY_HEIGHT;
    let mut bands = Vec::new();
    let mut y = hi;
    for k in 0..st.storeys {
        let tall = if k == 0 && matches!(st.ty, Type::Palazzotto | Type::Bottega) { 1.12 } else if k == 1 && st.ty == Type::Palazzotto { 1.08 } else { 1.0 };
        let hb = storey * tall * (0.94 + 0.1 * h(30 + k as u64)) * if st.ty == Type::Rustico { 0.85 } else { 1.0 };
        bands.push((y, hb));
        y += hb;
    }
    let eave = y;
    let parapet = if st.roof == Roof::Terrace { 1.9 } else { 0.0 };
    let wall_top = eave + parapet;
    let plinth_top = hi + if st.ty == Type::Rustico { 0.0 } else { 1.1 + 0.4 * h(40) };

    out.house_span(base, (st.worn * 0.999).clamp(0.0, 0.999));

    // the four sides, counter-clockwise from the one facing -v
    let facades = [
        Facade { o: [-a_half, -b_half], d: [1.0, 0.0], n: [0.0, -1.0], len: 2.0 * a_half },
        Facade { o: [a_half, -b_half], d: [0.0, 1.0], n: [1.0, 0.0], len: 2.0 * b_half },
        Facade { o: [a_half, b_half], d: [-1.0, 0.0], n: [0.0, 1.0], len: 2.0 * a_half },
        Facade { o: [-a_half, b_half], d: [0.0, -1.0], n: [-1.0, 0.0], len: 2.0 * b_half },
    ];
    // the front: the side nearest a road
    let mut front = 0;
    let mut best_d = f32::MAX;
    for (k, f) in facades.iter().enumerate() {
        let (s, t) = f.st(f.len * 0.5, 6.0);
        let p = fr.w(s, t, 0.0);
        let d = ctx.road_distance([p[0], -p[2]]);
        if d < best_d {
            best_d = d;
            front = k;
        }
    }
    let back = (front + 2) % 4;
    // a terrace house's flanks are often blind (party walls)
    let blind_sides = st.ty == Type::Schiera && h(41) < 0.55;

    let wall_part = if st.ty == Type::Rustico { HousePart::Rustic as u8 } else { HousePart::Plaster as u8 };
    let wall_c = linear(st.wall);
    let trim_c = linear(st.trim);
    let plinth_c = {
        let w = linear([st.wall[0] * 0.55 + 0.2, st.wall[1] * 0.55 + 0.18, st.wall[2] * 0.55 + 0.16]);
        [w[0] * 0.75, w[1] * 0.73, w[2] * 0.70]
    };

    let mut glass: Vec<([[f32; 3]; 4], [f32; 3], bool)> = Vec::new();

    for (fi, f) in facades.iter().enumerate() {
        let is_front = fi == front;
        let blind = blind_sides && fi != front && fi != back;
        let nout = fr.dir(f.n[0], f.n[1], 0.0);
        let hw = |s: u64| hash01(b.id as u64 ^ ((fi as u64 + 1) << 20), 0x4A00 + s);

        // columns of windows
        let (pitch, ww): (f32, f32) = match st.ty {
            Type::Schiera => (5.0, 2.0),
            Type::Palazzotto => (5.8, 2.2),
            Type::Bottega => (5.6, 2.1),
            Type::Casa => (6.0, 2.2),
            Type::Loggia => (6.2, 2.2),
            Type::Bassa => (6.6, 2.4),
            Type::Rustico => (6.8, 1.6),
        };
        let margin = 2.6;
        let room = f.len - 2.0 * margin;
        let ncol = if room < 0.0 { if f.len > 5.0 { 1 } else { 0 } } else { (room / pitch).floor() as usize + 1 };
        let step = if ncol > 1 { room / (ncol - 1) as f32 } else { 0.0 };
        let ww = if ncol > 1 { ww.min(step * 0.62) } else { ww };
        let cols: Vec<f32> = (0..ncol).map(|i| if ncol == 1 { f.len * 0.5 } else { margin + step * i as f32 }).collect();
        let door_col = if is_front && ncol > 0 { Some(if ncol % 2 == 1 { ncol / 2 } else { (ncol / 2).saturating_sub((hw(1) < 0.5) as usize) }) } else { None };
        // a loggia under the roof on the front, or balconies
        let loggia_top = st.ty == Type::Loggia && hw(2) < 0.55 && st.storeys >= 2;

        let mut ops: Vec<Opening> = Vec::new();
        let mut fakes: Vec<Fake> = Vec::new();
        for (r, &(band_lo, band)) in bands.iter().enumerate() {
            for (ci, &cx) in cols.iter().enumerate() {
                let hk = |s: u64| hash01(b.id as u64 ^ ((fi as u64 + 1) << 20) ^ ((r as u64 + 1) << 26) ^ ((ci as u64 + 1) << 32), 0x4B00 + s);
                let ground_here = {
                    let (s, t) = f.st(cx, 0.6);
                    fr.ground(scn, s, t)
                };
                if r == 0 && door_col == Some(ci) {
                    let arch = match st.ty {
                        Type::Palazzotto => hk(1) < 0.75,
                        Type::Rustico => hk(1) < 0.5,
                        Type::Bottega => false,
                        _ => hk(1) < 0.3,
                    };
                    let dw = if arch { 3.0 } else { 2.6 };
                    let y0 = ground_here.max(base + 0.5) + 0.25;
                    let y1 = (y0 + band * 0.72).min(band_lo + band * 0.86).max(y0 + 4.2);
                    ops.push(Opening { x0: cx - dw * 0.5, x1: cx + dw * 0.5, y0, y1, arch, depth: 0.5, hole: Hole::Door, shut: 0, balcony: false });
                    continue;
                }
                if r == 0 && is_front && st.ty == Type::Bottega {
                    let sw = (ww * 1.7).min(if ncol > 1 { step - 1.0 } else { f.len - 3.0 });
                    let y0 = ground_here.max(base + 0.5) + 0.7;
                    let y1 = (band_lo + band * 0.8).max(y0 + 3.5);
                    ops.push(Opening { x0: cx - sw * 0.5, x1: cx + sw * 0.5, y0, y1, arch: hk(2) < 0.35, depth: 0.45, hole: Hole::Shop, shut: 0, balcony: false });
                    continue;
                }
                let top_row = r + 1 == bands.len();
                if loggia_top && is_front && top_row {
                    let lw = if ncol > 1 { (step * 0.72).min(4.4) } else { 3.6 };
                    let y0 = band_lo + 0.3;
                    let y1 = band_lo + band * 0.86;
                    ops.push(Opening { x0: cx - lw * 0.5, x1: cx + lw * 0.5, y0, y1, arch: true, depth: 2.8, hole: Hole::Loggia, shut: 0, balcony: false });
                    continue;
                }
                if blind && r > 0 {
                    // a painted false window on some blind flanks
                    if st.painted_frames && hk(3) < 0.35 {
                        let (y0, y1) = (band_lo + band * 0.27, band_lo + band * 0.77);
                        fakes.push(Fake { x0: cx - ww * 0.5, x1: cx + ww * 0.5, y0, y1 });
                    }
                    continue;
                }
                if blind {
                    continue;
                }
                // the back and flanks lose a window now and then
                if !is_front && hk(4) < 0.12 {
                    continue;
                }
                if r == 0 && ground_here > band_lo + band * 0.2 {
                    continue; // the hill is up to the sill
                }
                if st.painted_frames && hk(5) < 0.08 && r > 0 {
                    let (y0, y1) = (band_lo + band * 0.27, band_lo + band * 0.77);
                    fakes.push(Fake { x0: cx - ww * 0.5, x1: cx + ww * 0.5, y0, y1 });
                    continue;
                }
                // balconies: the loggia houses' upper floors, some others'
                let balcony = r > 0
                    && is_front
                    && match st.ty {
                        Type::Loggia => !loggia_top,
                        Type::Palazzotto => r == 1 && door_col == Some(ci),
                        Type::Schiera | Type::Bottega => hk(6) < 0.3,
                        Type::Casa => r == 1 && hk(6) < 0.35,
                        _ => false,
                    };
                let tall = if st.ty == Type::Palazzotto && r == 1 { 0.58 } else if st.ty == Type::Rustico { 0.4 } else { 0.5 };
                let mut y0 = band_lo + band * if r == 0 { 0.3 } else { 0.26 };
                let y1 = y0 + band * tall;
                let w = if st.ty == Type::Rustico && r == 0 && hk(7) < 0.5 { ww * 0.7 } else { ww };
                if balcony {
                    y0 = band_lo + 0.2;
                }
                let shut = if st.shutters.is_none() {
                    0
                } else {
                    let s = hk(8);
                    if s < 0.22 { 1 } else if s < 0.32 { 2 } else { 0 }
                };
                let arch = st.ty == Type::Palazzotto && r == 0 && hk(9) < 0.5;
                ops.push(Opening {
                    x0: cx - w * 0.5,
                    x1: cx + w * 0.5,
                    y0,
                    y1,
                    arch,
                    depth: if st.ty == Type::Rustico { 0.6 } else { 0.38 },
                    hole: if balcony { Hole::French } else { Hole::Window },
                    shut,
                    balcony,
                });
            }
        }

        // the wall round the openings
        out.part = wall_part;
        let mut ys: Vec<f32> = vec![base, wall_top];
        if plinth_top > base && plinth_top < wall_top {
            ys.push(plinth_top);
        }
        for o in &ops {
            ys.push(o.y0.max(base));
            ys.push(o.y1.min(wall_top));
        }
        ys.sort_by(|a, b| a.total_cmp(b));
        ys.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        let mut ledge: Vec<(f32, f32)> = Vec::new();
        for w in ys.windows(2) {
            let (ya, yb) = (w[0], w[1]);
            let mut cut: Vec<(f32, f32)> = ops.iter().filter(|o| o.y0 <= ya + 1e-3 && o.y1 >= yb - 1e-3).map(|o| (o.x0, o.x1)).collect();
            cut.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut x = 0.0;
            let plinth = yb <= plinth_top + 1e-3 && st.ty != Type::Rustico;
            out.part = if plinth { Part::Stone as u8 } else { wall_part };
            let col = if plinth { plinth_c } else { wall_c };
            let mut gaps = Vec::new();
            for (a, c) in cut.iter().copied().chain(std::iter::once((f.len, f.len))) {
                if a > x + 1e-3 {
                    gaps.push((x, a));
                }
                x = x.max(c);
            }
            for &(xa, xb) in &gaps {
                face(out, [fp(&fr, f, xa, ya, 0.0), fp(&fr, f, xb, ya, 0.0), fp(&fr, f, xb, yb, 0.0), fp(&fr, f, xa, yb, 0.0)], nout, col);
            }
            if (ya - plinth_top).abs() < 1e-3 {
                ledge = gaps;
            }
        }
        // the plinth's ledge
        out.part = Part::Stone as u8;
        if st.ty != Type::Rustico {
            for &(xa, xb) in &ledge {
                fbox(out, &fr, f, xa, xb, plinth_top - 0.18, plinth_top, 0.0, 0.12, FRONT | TOP | BOTTOM, plinth_c);
            }
        }
        // gable triangles on the ends a two-slope roof leaves open
        let pitch = if st.slate { 0.5 } else { 0.42 };
        if let Roof::Gable { across } = st.roof {
            let end = if across { fi == 0 || fi == 2 } else { fi == 1 || fi == 3 };
            if end {
                let half = if across { a_half } else { b_half };
                out.part = wall_part;
                tri(out, fp(&fr, f, 0.0, eave, 0.0), fp(&fr, f, f.len, eave, 0.0), fp(&fr, f, f.len * 0.5, eave + half * pitch, 0.0), nout, wall_c);
            }
        }

        // the openings' sides
        for o in &ops {
            let reveal_c = if st.ty == Type::Rustico { wall_c } else { [wall_c[0] * 0.92, wall_c[1] * 0.92, wall_c[2] * 0.92] };
            out.part = if st.ty == Type::Rustico { HousePart::Rustic as u8 } else { HousePart::Reveal as u8 };
            let d = -o.depth;
            let ys = o.spring();
            let along = fr.dir(f.d[0], f.d[1], 0.0);
            let y0 = o.y0.max(base);
            face(out, [fp(&fr, f, o.x0, y0, 0.0), fp(&fr, f, o.x0, y0, d), fp(&fr, f, o.x0, ys, d), fp(&fr, f, o.x0, ys, 0.0)], along, reveal_c);
            face(out, [fp(&fr, f, o.x1, y0, 0.0), fp(&fr, f, o.x1, y0, d), fp(&fr, f, o.x1, ys, d), fp(&fr, f, o.x1, ys, 0.0)], along.map(|v| -v), reveal_c);
            // the floor of the opening (a loggia's in terracotta)
            if o.hole == Hole::Loggia {
                out.part = Part::Stone as u8;
                face(out, [fp(&fr, f, o.x0, y0, 0.0), fp(&fr, f, o.x1, y0, 0.0), fp(&fr, f, o.x1, y0, d), fp(&fr, f, o.x0, y0, d)], [0.0, 1.0, 0.0], linear([0.55, 0.32, 0.22]));
                out.part = HousePart::Reveal as u8;
            } else {
                face(out, [fp(&fr, f, o.x0, y0, 0.0), fp(&fr, f, o.x1, y0, 0.0), fp(&fr, f, o.x1, y0, d), fp(&fr, f, o.x0, y0, d)], [0.0, 1.0, 0.0], reveal_c);
            }
            if o.arch {
                let pts = arc(o);
                let (xc, ys_) = ((o.x0 + o.x1) * 0.5, o.spring());
                for k in 0..ARCH_SEGS {
                    let (a, c) = (pts[k], pts[k + 1]);
                    // intrados, facing the arch's centre
                    let mid = ((a.0 + c.0) * 0.5, (a.1 + c.1) * 0.5);
                    let (nx, ny) = (xc - mid.0, ys_ - mid.1);
                    let n = [along[0] * nx, ny, along[2] * nx];
                    face(out, [fp(&fr, f, a.0, a.1, 0.0), fp(&fr, f, c.0, c.1, 0.0), fp(&fr, f, c.0, c.1, d), fp(&fr, f, a.0, a.1, d)], n, reveal_c);
                    // the spandrel of wall between the arch and the opening's box
                    out.part = wall_part;
                    let corner = if k < ARCH_SEGS / 2 { (o.x0, o.y1) } else { (o.x1, o.y1) };
                    tri(out, fp(&fr, f, corner.0, corner.1, 0.0), fp(&fr, f, a.0, a.1, 0.0), fp(&fr, f, c.0, c.1, 0.0), nout, wall_c);
                    out.part = HousePart::Reveal as u8;
                }
                // the last spandrel triangle between the crown and both corners
                out.part = wall_part;
                let crown = pts[ARCH_SEGS / 2];
                tri(out, fp(&fr, f, o.x0, o.y1, 0.0), fp(&fr, f, crown.0, crown.1, 0.0), fp(&fr, f, o.x1, o.y1, 0.0), nout, wall_c);
            } else {
                face(out, [fp(&fr, f, o.x0, o.y1, 0.0), fp(&fr, f, o.x1, o.y1, 0.0), fp(&fr, f, o.x1, o.y1, d), fp(&fr, f, o.x0, o.y1, d)], [0.0, -1.0, 0.0], reveal_c);
            }

            // what is at the back of the opening
            let back_d = d + 0.01;
            let (gx0, gx1) = (o.x0, o.x1);
            match o.hole {
                Hole::Window | Hole::French | Hole::Shop => {
                    glass.push(([fp(&fr, f, gx0, y0, back_d), fp(&fr, f, gx1, y0, back_d), fp(&fr, f, gx1, ys, back_d), fp(&fr, f, gx0, ys, back_d)], nout, false));
                    if o.arch {
                        let pts = arc(o);
                        let cxy = ((o.x0 + o.x1) * 0.5, ys);
                        for k in 0..ARCH_SEGS {
                            glass.push(([fp(&fr, f, cxy.0, cxy.1, back_d), fp(&fr, f, pts[k].0, pts[k].1, back_d), fp(&fr, f, pts[k + 1].0, pts[k + 1].1, back_d), fp(&fr, f, pts[k + 1].0, pts[k + 1].1, back_d)], nout, true));
                        }
                    }
                }
                Hole::Door => {
                    // two leaves of wood, a dark seam between; a fanlight in an arch
                    out.part = WOOD;
                    let door = linear(st.door);
                    let xm = (gx0 + gx1) * 0.5;
                    for (a, c) in [(gx0, xm - 0.04), (xm + 0.04, gx1)] {
                        face(out, [fp(&fr, f, a, y0, back_d), fp(&fr, f, c, y0, back_d), fp(&fr, f, c, ys, back_d), fp(&fr, f, a, ys, back_d)], nout, door);
                        // two raised panels
                        let (pa, pc) = (a + 0.22, c - 0.22);
                        let hgt = ys - y0;
                        for (q0, q1) in [(0.1, 0.42), (0.52, 0.9)] {
                            let (ya, yb) = (y0 + hgt * q0, y0 + hgt * q1);
                            fbox(out, &fr, f, pa, pc, ya, yb, back_d, back_d + 0.07, FRONT | TOP | BOTTOM | ENDS, [door[0] * 1.1, door[1] * 1.1, door[2] * 1.1]);
                        }
                    }
                    out.part = Part::Frame as u8;
                    face(out, [fp(&fr, f, xm - 0.04, y0, back_d - 0.01), fp(&fr, f, xm + 0.04, y0, back_d - 0.01), fp(&fr, f, xm + 0.04, ys, back_d - 0.01), fp(&fr, f, xm - 0.04, ys, back_d - 0.01)], nout, [0.02, 0.02, 0.02]);
                    if o.arch {
                        let pts = arc(o);
                        let cxy = ((o.x0 + o.x1) * 0.5, ys);
                        for k in 0..ARCH_SEGS {
                            glass.push(([fp(&fr, f, cxy.0, cxy.1, back_d), fp(&fr, f, pts[k].0, pts[k].1, back_d), fp(&fr, f, pts[k + 1].0, pts[k + 1].1, back_d), fp(&fr, f, pts[k + 1].0, pts[k + 1].1, back_d)], nout, true));
                        }
                    } else {
                        // the head of the door: a lintel of stone or a painted band
                        out.part = Part::Stone as u8;
                        let lc = linear(st.sill);
                        fbox(out, &fr, f, gx0 - 0.5, gx1 + 0.5, o.y1, o.y1 + 0.55, 0.0, 0.1, FRONT | BOTTOM | TOP | ENDS, lc);
                    }
                    // a step of stone
                    out.part = Part::Stone as u8;
                    let g = {
                        let (s, t) = f.st(xm, 0.8);
                        fr.ground(scn, s, t)
                    };
                    if y0 > g - 0.2 {
                        fbox(out, &fr, f, gx0 - 0.5, gx1 + 0.5, g - 0.4, y0, 0.0, 0.9, FRONT | TOP | ENDS, linear(st.sill));
                    }
                    // a portal of stone on the grander houses
                    if matches!(st.ty, Type::Palazzotto | Type::Rustico) {
                        let pc = linear([0.62, 0.60, 0.55]);
                        for (a, c) in [(gx0 - 0.6, gx0), (gx1, gx1 + 0.6)] {
                            fbox(out, &fr, f, a, c, y0, ys, 0.0, 0.14, FRONT | ENDS, pc);
                        }
                        if o.arch {
                            // keystone
                            let crown = (o.x0 + o.x1) * 0.5;
                            fbox(out, &fr, f, crown - 0.35, crown + 0.35, o.y1 - 0.3, o.y1 + 0.6, 0.0, 0.16, FRONT | ENDS | BOTTOM | TOP, pc);
                        }
                    }
                }
                Hole::Loggia => {
                    // the room's wall behind, with a glazed door in it
                    out.part = HousePart::Reveal as u8;
                    let bk = [wall_c[0] * 0.55, wall_c[1] * 0.55, wall_c[2] * 0.55];
                    face(out, [fp(&fr, f, gx0, y0, back_d), fp(&fr, f, gx1, y0, back_d), fp(&fr, f, gx1, o.y1, back_d), fp(&fr, f, gx0, o.y1, back_d)], nout, bk);
                    let xm = (gx0 + gx1) * 0.5;
                    glass.push(([fp(&fr, f, xm - 0.9, y0, back_d + 0.02), fp(&fr, f, xm + 0.9, y0, back_d + 0.02), fp(&fr, f, xm + 0.9, y0 + 4.6, back_d + 0.02), fp(&fr, f, xm - 0.9, y0 + 4.6, back_d + 0.02)], nout, false));
                    // a low balustrade in the arch
                    out.part = wall_part;
                    fbox(out, &fr, f, gx0, gx1, y0, y0 + 2.3, -0.45, -0.05, FRONT | TOP, wall_c);
                    out.part = Part::Stone as u8;
                    fbox(out, &fr, f, gx0, gx1, y0 + 2.3, y0 + 2.5, -0.5, 0.05, FRONT | TOP | BOTTOM, linear(st.sill));
                }
            }

            // joinery: frame, mullion, transom (in the frame's paint)
            if matches!(o.hole, Hole::Window | Hole::French | Hole::Shop) {
                out.part = Part::Frame as u8;
                let fc = linear(if o.hole == Hole::Shop { [0.20, 0.24, 0.20] } else { st.frame });
                let jd = back_d + 0.06;
                let bw = 0.16;
                fbox(out, &fr, f, gx0, gx0 + bw, y0, ys, back_d, jd, FRONT | ENDS, fc);
                fbox(out, &fr, f, gx1 - bw, gx1, y0, ys, back_d, jd, FRONT | ENDS, fc);
                fbox(out, &fr, f, gx0, gx1, y0, y0 + bw, back_d, jd, FRONT | TOP, fc);
                if !o.arch {
                    fbox(out, &fr, f, gx0, gx1, ys - bw, ys, back_d, jd, FRONT | BOTTOM, fc);
                }
                let xm = (gx0 + gx1) * 0.5;
                fbox(out, &fr, f, xm - 0.06, xm + 0.06, y0, ys, back_d, jd, FRONT | ENDS, fc);
                let yt = y0 + (ys - y0) * if o.hole == Hole::French { 0.72 } else { 0.66 };
                fbox(out, &fr, f, gx0, gx1, yt - 0.05, yt + 0.05, back_d, jd, FRONT | TOP | BOTTOM, fc);
            }

            // sill (windows)
            if o.hole == Hole::Window {
                out.part = Part::Stone as u8;
                fbox(out, &fr, f, o.x0 - 0.3, o.x1 + 0.3, y0 - 0.22, y0 + 0.02, -0.1, 0.28, FRONT | TOP | BOTTOM | ENDS, linear(st.sill));
            }
            // a heavy stone lintel on a rustic house
            if st.ty == Type::Rustico && !o.arch && o.hole != Hole::Door {
                out.part = Part::Stone as u8;
                fbox(out, &fr, f, o.x0 - 0.45, o.x1 + 0.45, o.y1, o.y1 + 0.6, 0.0, 0.08, FRONT | BOTTOM | TOP | ENDS, linear([0.55, 0.52, 0.47]));
            }
            // painted frame and the little cornice over it
            if st.painted_frames && matches!(o.hole, Hole::Window | Hole::French) {
                out.part = HousePart::Paint as u8;
                let pw = 0.38;
                let pd = 0.015;
                let p = |x: f32, y: f32| fp(&fr, f, x, y, pd);
                face(out, [p(o.x0 - pw, y0 - 0.02), p(o.x0, y0 - 0.02), p(o.x0, ys), p(o.x0 - pw, ys)], nout, trim_c);
                face(out, [p(o.x1, y0 - 0.02), p(o.x1 + pw, y0 - 0.02), p(o.x1 + pw, ys), p(o.x1, ys)], nout, trim_c);
                if !o.arch {
                    face(out, [p(o.x0 - pw, o.y1), p(o.x1 + pw, o.y1), p(o.x1 + pw, o.y1 + pw), p(o.x0 - pw, o.y1 + pw)], nout, trim_c);
                    let sty = st.ty == Type::Palazzotto;
                    if sty {
                        // a moulded head, proud, and on the noble floor a painted pediment
                        out.part = HousePart::Paint as u8;
                        fbox(out, &fr, f, o.x0 - pw - 0.15, o.x1 + pw + 0.15, o.y1 + pw, o.y1 + pw + 0.28, 0.0, 0.2, FRONT | TOP | BOTTOM | ENDS, trim_c);
                        if o.hole == Hole::French || (o.y1 - o.y0) > 4.4 {
                            let yb = o.y1 + pw + 0.28;
                            tri(out, p(o.x0 - pw, yb), p(o.x1 + pw, yb), p((o.x0 + o.x1) * 0.5, yb + 1.0), nout, trim_c);
                        }
                    }
                }
            }
            // shutters
            if let (Some(sc), Hole::Window | Hole::French) = (st.shutters, o.hole) {
                let sc = linear(sc);
                out.part = SHUTTER;
                let w2 = (o.x1 - o.x0) * 0.5;
                let (top, bot) = (ys, y0);
                let closed_l = o.shut == 1 || o.shut == 2;
                let closed_r = o.shut == 1;
                // closed: in the opening, at its front
                let inset = -0.12;
                if closed_l {
                    fbox(out, &fr, f, o.x0, o.x0 + w2 - 0.02, bot, top, inset - 0.05, inset, FRONT, sc);
                } else {
                    fbox(out, &fr, f, o.x0 - w2 - 0.06, o.x0 - 0.06, bot, top, 0.02, 0.1, FRONT | ENDS | TOP, sc);
                }
                if closed_r {
                    fbox(out, &fr, f, o.x0 + w2 + 0.02, o.x1, bot, top, inset - 0.05, inset, FRONT, sc);
                } else {
                    fbox(out, &fr, f, o.x1 + 0.06, o.x1 + w2 + 0.06, bot, top, 0.02, 0.1, FRONT | ENDS | TOP, sc);
                }
            }
            // a balcony: slab, brackets, iron railing
            if o.balcony {
                let fl = o.y0;
                let (bx0, bx1) = (o.x0 - 0.9, o.x1 + 0.9);
                let depth = 1.7;
                out.part = Part::Stone as u8;
                let slab = linear(st.sill);
                fbox(out, &fr, f, bx0, bx1, fl - 0.3, fl, 0.0, depth, FRONT | TOP | BOTTOM | ENDS, slab);
                for x in [bx0 + 0.35, bx1 - 0.65] {
                    fbox(out, &fr, f, x, x + 0.3, fl - 1.1, fl - 0.3, 0.0, depth * 0.7, FRONT | BOTTOM | ENDS, slab);
                }
                out.part = METAL;
                let iron = [0.025, 0.025, 0.028];
                let rh = 2.4;
                let rd = depth - 0.1;
                fbox(out, &fr, f, bx0 + 0.05, bx1 - 0.05, fl + rh - 0.12, fl + rh, rd - 0.07, rd + 0.05, FRONT | TOP | BOTTOM | BACK, iron);
                fbox(out, &fr, f, bx0 + 0.05, bx1 - 0.05, fl + 0.15, fl + 0.25, rd - 0.05, rd + 0.03, FRONT | TOP | BACK, iron);
                let n = ((bx1 - bx0 - 0.2) / 0.42).round().max(2.0) as usize;
                for k in 0..=n {
                    let x = bx0 + 0.1 + (bx1 - bx0 - 0.2) * k as f32 / n as f32;
                    fbox(out, &fr, f, x - 0.035, x + 0.035, fl, fl + rh - 0.12, rd - 0.035, rd + 0.035, FRONT | BACK | ENDS, iron);
                }
                // the sides
                for x in [bx0 + 0.1, bx1 - 0.1] {
                    for k in 0..4 {
                        let dd = 0.2 + (rd - 0.2) * k as f32 / 4.0;
                        fbox(out, &fr, f, x - 0.035, x + 0.035, fl, fl + rh - 0.12, dd - 0.035, dd + 0.035, FRONT | BACK | ENDS, iron);
                    }
                    fbox(out, &fr, f, x - 0.05, x + 0.05, fl + rh - 0.12, fl + rh, 0.0, rd, TOP | ENDS | BOTTOM, iron);
                }
            }
            // a shop's awning
            if o.hole == Hole::Shop {
                out.part = CLOTH;
                let stripes = [[0.62, 0.10, 0.08], [0.10, 0.30, 0.16], [0.12, 0.20, 0.42], [0.55, 0.38, 0.08]];
                let pc = stripes[(h(50) * 4.0) as usize % 4];
                let pale = [0.80, 0.77, 0.70];
                let (top, low, reach) = (o.y1 + 1.2, o.y1 - 0.3, 2.6);
                let cnt = 7;
                for k in 0..cnt {
                    let xa = o.x0 - 0.3 + (o.x1 - o.x0 + 0.6) * k as f32 / cnt as f32;
                    let xb = o.x0 - 0.3 + (o.x1 - o.x0 + 0.6) * (k + 1) as f32 / cnt as f32;
                    let col = if k % 2 == 0 { pc } else { pale };
                    face(out, [fp(&fr, f, xa, top, 0.05), fp(&fr, f, xb, top, 0.05), fp(&fr, f, xb, low, reach), fp(&fr, f, xa, low, reach)], [nout[0], 1.0, nout[2]], col);
                    face(out, [fp(&fr, f, xa, low - 0.6, reach), fp(&fr, f, xb, low - 0.6, reach), fp(&fr, f, xb, low, reach), fp(&fr, f, xa, low, reach)], nout, col);
                }
                // the sign
                if door_col.is_some() {
                    out.part = HousePart::Paint as u8;
                    fbox(out, &fr, f, o.x0, o.x1, top + 0.4, top + 1.6, 0.0, 0.08, FRONT | ENDS | TOP | BOTTOM, linear([0.30, 0.24, 0.16]));
                }
            }
        }

        // painted false windows: closed shutters, a frame, a sill, all paint
        for k in &fakes {
            let p = |x: f32, y: f32, d: f32| fp(&fr, f, x, y, d);
            out.part = HousePart::Paint as u8;
            let pw = 0.38;
            face(out, [p(k.x0 - pw, k.y0 - 0.25, 0.012), p(k.x1 + pw, k.y0 - 0.25, 0.012), p(k.x1 + pw, k.y1 + pw, 0.012), p(k.x0 - pw, k.y1 + pw, 0.012)], nout, trim_c);
            out.part = SHUTTER;
            let sc = linear(st.shutters.unwrap_or([0.18, 0.36, 0.24]));
            let xm = (k.x0 + k.x1) * 0.5;
            face(out, [p(k.x0, k.y0, 0.02), p(xm - 0.03, k.y0, 0.02), p(xm - 0.03, k.y1, 0.02), p(k.x0, k.y1, 0.02)], nout, sc);
            face(out, [p(xm + 0.03, k.y0, 0.02), p(k.x1, k.y0, 0.02), p(k.x1, k.y1, 0.02), p(xm + 0.03, k.y1, 0.02)], nout, sc);
            out.part = HousePart::Paint as u8;
            face(out, [p(xm - 0.03, k.y0, 0.018), p(xm + 0.03, k.y0, 0.018), p(xm + 0.03, k.y1, 0.018), p(xm - 0.03, k.y1, 0.018)], nout, [0.03, 0.03, 0.03]);
        }

        // painted quoins
        if st.quoins {
            out.part = HousePart::Paint as u8;
            // long and short blocks one on the other: a toothed edge, no thin
            // joints to shimmer from afar
            let mut y = plinth_top;
            let mut k = 0;
            while y + 1.2 < eave - 1.2 {
                let w = if k % 2 == 0 { 1.8 } else { 1.1 };
                for (x0, x1) in [(0.0, w), (f.len - w, f.len)] {
                    face(out, [fp(&fr, f, x0, y, 0.012), fp(&fr, f, x1, y, 0.012), fp(&fr, f, x1, y + 1.2, 0.012), fp(&fr, f, x0, y + 1.2, 0.012)], nout, trim_c);
                }
                y += 1.2;
                k += 1;
            }
        }
        // string courses at the floors
        if st.bands {
            out.part = HousePart::Paint as u8;
            for &(band_lo, _) in bands.iter().skip(1) {
                fbox(out, &fr, f, -0.14, f.len + 0.14, band_lo - 0.42, band_lo - 0.08, 0.0, 0.14, FRONT | TOP | BOTTOM, trim_c);
            }
        }
        // the cornice under the eave (on a terrace, under the parapet)
        out.part = HousePart::Paint as u8;
        if st.cornice {
            for (k, (y0, y1, dd)) in [(eave - 1.15, eave - 0.75, 0.16), (eave - 0.75, eave - 0.4, 0.34), (eave - 0.4, eave, 0.55)].into_iter().enumerate() {
                let c = if k == 1 { [trim_c[0] * 0.93, trim_c[1] * 0.93, trim_c[2] * 0.93] } else { trim_c };
                fbox(out, &fr, f, -dd, f.len + dd, y0, y1, 0.0, dd, FRONT | BOTTOM | if st.roof == Roof::Terrace { TOP } else { 0 }, c);
            }
        } else if st.ty != Type::Rustico {
            fbox(out, &fr, f, -0.2, f.len + 0.2, eave - 0.6, eave, 0.0, 0.2, FRONT | BOTTOM | if st.roof == Roof::Terrace { TOP } else { 0 }, trim_c);
        }
        if st.roof == Roof::Terrace {
            // the parapet's coping and its inner face
            out.part = Part::Stone as u8;
            fbox(out, &fr, f, -0.15, f.len + 0.15, wall_top, wall_top + 0.25, -0.55, 0.15, FRONT | TOP | BOTTOM, linear(st.sill));
            out.part = wall_part;
            face(out, [fp(&fr, f, 0.0, eave, -0.45), fp(&fr, f, f.len, eave, -0.45), fp(&fr, f, f.len, wall_top, -0.45), fp(&fr, f, 0.0, wall_top, -0.45)], nout.map(|v| -v), wall_c);
        }
    }

    // the roof
    let roof_part = if st.slate { HousePart::Slate as u8 } else { Part::Tiles as u8 };
    let roof_c = linear(st.roof_c);
    let pitch = if st.slate { 0.5 } else { 0.42 };
    let eave_e = match st.ty {
        Type::Palazzotto => 1.5,
        Type::Rustico => 0.9,
        _ => 1.15,
    };
    let mut chimneys: Vec<(f32, f32, f32)> = Vec::new();
    match st.roof {
        Roof::Terrace => {
            out.part = Part::Stone as u8;
            let deck = linear([0.50, 0.32, 0.24]);
            let (aa, bb) = (a_half - 0.45, b_half - 0.45);
            face(out, [fr.w(-aa, -bb, eave + 0.05), fr.w(aa, -bb, eave + 0.05), fr.w(aa, bb, eave + 0.05), fr.w(-aa, bb, eave + 0.05)], [0.0, 1.0, 0.0], deck);
            // a little room on the terrace (the stair's), with its own roof
            if h(60) < 0.6 {
                let (ra, rb) = (a_half * 0.3, b_half * 0.38);
                let (cs, ct) = (a_half * 0.4 * if h(61) < 0.5 { -1.0 } else { 1.0 }, b_half * 0.2);
                let top = eave + storey * 0.7;
                let sub = [
                    Facade { o: [cs - ra, ct - rb], d: [1.0, 0.0], n: [0.0, -1.0], len: 2.0 * ra },
                    Facade { o: [cs + ra, ct - rb], d: [0.0, 1.0], n: [1.0, 0.0], len: 2.0 * rb },
                    Facade { o: [cs + ra, ct + rb], d: [-1.0, 0.0], n: [0.0, 1.0], len: 2.0 * ra },
                    Facade { o: [cs - ra, ct + rb], d: [0.0, -1.0], n: [-1.0, 0.0], len: 2.0 * rb },
                ];
                out.part = wall_part;
                for (k, f) in sub.iter().enumerate() {
                    let nout = fr.dir(f.n[0], f.n[1], 0.0);
                    if k == 0 {
                        // a door onto the terrace
                        let xm = f.len * 0.5;
                        face(out, [fp(&fr, f, 0.0, eave, 0.0), fp(&fr, f, xm - 1.1, eave, 0.0), fp(&fr, f, xm - 1.1, top, 0.0), fp(&fr, f, 0.0, top, 0.0)], nout, wall_c);
                        face(out, [fp(&fr, f, xm + 1.1, eave, 0.0), fp(&fr, f, f.len, eave, 0.0), fp(&fr, f, f.len, top, 0.0), fp(&fr, f, xm + 1.1, top, 0.0)], nout, wall_c);
                        face(out, [fp(&fr, f, xm - 1.1, eave + 4.6, 0.0), fp(&fr, f, xm + 1.1, eave + 4.6, 0.0), fp(&fr, f, xm + 1.1, top, 0.0), fp(&fr, f, xm - 1.1, top, 0.0)], nout, wall_c);
                        out.part = WOOD;
                        face(out, [fp(&fr, f, xm - 1.1, eave, -0.2), fp(&fr, f, xm + 1.1, eave, -0.2), fp(&fr, f, xm + 1.1, eave + 4.6, -0.2), fp(&fr, f, xm - 1.1, eave + 4.6, -0.2)], nout, linear(st.door));
                        out.part = HousePart::Reveal as u8;
                        let along = fr.dir(f.d[0], f.d[1], 0.0);
                        face(out, [fp(&fr, f, xm - 1.1, eave, 0.0), fp(&fr, f, xm - 1.1, eave, -0.2), fp(&fr, f, xm - 1.1, eave + 4.6, -0.2), fp(&fr, f, xm - 1.1, eave + 4.6, 0.0)], along, wall_c);
                        face(out, [fp(&fr, f, xm + 1.1, eave, 0.0), fp(&fr, f, xm + 1.1, eave, -0.2), fp(&fr, f, xm + 1.1, eave + 4.6, -0.2), fp(&fr, f, xm + 1.1, eave + 4.6, 0.0)], along.map(|v| -v), wall_c);
                        face(out, [fp(&fr, f, xm - 1.1, eave + 4.6, 0.0), fp(&fr, f, xm + 1.1, eave + 4.6, 0.0), fp(&fr, f, xm + 1.1, eave + 4.6, -0.2), fp(&fr, f, xm - 1.1, eave + 4.6, -0.2)], [0.0, -1.0, 0.0], wall_c);
                        out.part = wall_part;
                    } else {
                        face(out, [fp(&fr, f, 0.0, eave, 0.0), fp(&fr, f, f.len, eave, 0.0), fp(&fr, f, f.len, top, 0.0), fp(&fr, f, 0.0, top, 0.0)], nout, wall_c);
                    }
                    out.part = HousePart::Paint as u8;
                    fbox(out, &fr, f, -0.3, f.len + 0.3, top - 0.45, top, 0.0, 0.3, FRONT | BOTTOM, trim_c);
                    out.part = wall_part;
                }
                hip_roof(out, &fr, cs, ct, ra, rb, 0.6, top, pitch, roof_part, roof_c, true);
            }
            chimneys.push((-a_half * 0.55, b_half * 0.6, eave + parapet));
        }
        Roof::Hip => {
            hip_roof(out, &fr, 0.0, 0.0, a_half, b_half, eave_e, eave, pitch, roof_part, roof_c, true);
            let roof_y = |s: f32, t: f32| eave + (b_half - t.abs()).min(a_half - s.abs()) * pitch + 0.3;
            let s = a_half * (0.3 + 0.3 * h(62)) * if h(63) < 0.5 { -1.0 } else { 1.0 };
            let t = b_half * 0.35 * if h(64) < 0.5 { -1.0 } else { 1.0 };
            chimneys.push((s, t, roof_y(s, t)));
            if st.ty == Type::Palazzotto && h(65) < 0.6 {
                chimneys.push((-s, -t, roof_y(-s, -t)));
            }
        }
        Roof::Gable { across } => {
            gable_roof(out, &fr, a_half, b_half, eave_e, eave, pitch, roof_part, roof_c, across);
            let (s, t) = if across {
                (a_half * 0.35 * if h(63) < 0.5 { -1.0 } else { 1.0 }, b_half * (0.5 + 0.3 * h(62)) * if h(64) < 0.5 { -1.0 } else { 1.0 })
            } else {
                (a_half * (0.5 + 0.35 * h(62)) * if h(63) < 0.5 { -1.0 } else { 1.0 }, b_half * 0.3 * if h(64) < 0.5 { -1.0 } else { 1.0 })
            };
            let ry = if across { eave + (a_half - s.abs()) * pitch + 0.3 } else { eave + (b_half - t.abs()) * pitch + 0.3 };
            chimneys.push((s, t, ry));
        }
    }
    if h(66) < 0.85 {
        let ridge = eave + b_half.min(a_half) * pitch;
        for &(s, t, ry) in &chimneys {
            chimney(out, &fr, s, t, ry - 1.2, ridge.max(ry) + 2.6 + h(67) * 1.5, wall_c, st.ty == Type::Rustico, roof_c, roof_part);
        }
    }

    // the glass, in one run (the lit windows of `buildings::damage`)
    out.part = GLASS;
    let start = out.positions.len() as u32;
    let gc = [0.06, 0.07, 0.10];
    for (q, n, is_tri) in glass {
        if is_tri {
            tri(out, q[0], q[1], q[2], n, gc);
        } else {
            face(out, q, n, gc);
        }
    }
    let end = out.positions.len() as u32;
    out.part = 0;
    out.house_span(-1.0e4, 0.5);
    Some((start, end))
}

/// A hipped roof over the rectangle of half sides `a` x `b` centred at
/// `(cs, ct)`, its foot at `eave` on the walls and overhanging by `e`: four
/// slopes, the ridge and hips capped in tiles, the soffit and the fascia.
#[allow(clippy::too_many_arguments)]
fn hip_roof(out: &mut Builder, fr: &Frame, cs: f32, ct: f32, a: f32, b: f32, e: f32, eave: f32, pitch: f32, part: u8, col: [f32; 3], full: bool) {
    let th = 0.32;
    let (ea, eb) = (a + e, b + e);
    // slopes on the long sides (along s if a >= b)
    let (ra, rb) = if a >= b { (a - b, 0.0) } else { (0.0, b - a) };
    let low = eave - e * pitch + th;
    let top = eave + a.min(b) * pitch + th;
    let w = |s: f32, t: f32, y: f32| fr.w(cs + s, ct + t, y);
    out.part = part;
    let up = |nx: f32, nt: f32| {
        let d = fr.dir(nx, nt, 0.0);
        [d[0] * pitch, 1.0, d[2] * pitch]
    };
    face(out, [w(-ea, -eb, low), w(ea, -eb, low), w(ra, -rb, top), w(-ra, -rb, top)], up(0.0, -1.0), col);
    face(out, [w(ea, eb, low), w(-ea, eb, low), w(-ra, rb, top), w(ra, rb, top)], up(0.0, 1.0), col);
    face(out, [w(ea, -eb, low), w(ea, eb, low), w(ra, rb, top), w(ra, -rb, top)], up(1.0, 0.0), col);
    face(out, [w(-ea, eb, low), w(-ea, -eb, low), w(-ra, -rb, top), w(-ra, rb, top)], up(-1.0, 0.0), col);
    // caps of half-round tiles along the ridge and the hips
    let cap = [col[0] * 0.85, col[1] * 0.82, col[2] * 0.8];
    ridge_cap(out, w(-ra, -rb, top), w(ra, rb, top), cap);
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        ridge_cap(out, w(sx * ea, sy * eb, low), w(sx * ra, sy * rb, top), cap);
    }
    if !full {
        return;
    }
    // the soffit, in the shade under the overhang, and the fascia
    out.part = HousePart::Reveal as u8;
    let soffit = [col[0] * 0.5, col[1] * 0.5, col[2] * 0.5];
    let ys = eave - e * pitch;
    let corners_w = [(-a, -b), (a, -b), (a, b), (-a, b)];
    let corners_e = [(-ea, -eb), (ea, -eb), (ea, eb), (-ea, eb)];
    for k in 0..4 {
        let (p0, p1) = (corners_w[k], corners_w[(k + 1) % 4]);
        let (q0, q1) = (corners_e[k], corners_e[(k + 1) % 4]);
        face(out, [w(p0.0, p0.1, eave), w(p1.0, p1.1, eave), w(q1.0, q1.1, ys), w(q0.0, q0.1, ys)], [0.0, -1.0, 0.0], soffit);
        let n = [fr.dir(0.0, -1.0, 0.0), fr.dir(1.0, 0.0, 0.0), fr.dir(0.0, 1.0, 0.0), fr.dir(-1.0, 0.0, 0.0)][k];
        out.part = part;
        face(out, [w(q0.0, q0.1, ys), w(q1.0, q1.1, ys), w(q1.0, q1.1, low), w(q0.0, q0.1, low)], n, [col[0] * 0.7, col[1] * 0.7, col[2] * 0.7]);
        out.part = HousePart::Reveal as u8;
    }
}

/// A two-slope roof over the house: the ridge along `s` (across: along `t`),
/// overhanging by `e` on all sides, with the soffits, the fascias and the
/// barge boards under the verges.
#[allow(clippy::too_many_arguments)]
fn gable_roof(out: &mut Builder, fr: &Frame, a: f32, b: f32, e: f32, eave: f32, pitch: f32, part: u8, col: [f32; 3], across: bool) {
    let th = 0.32;
    // in the roof's own axes: `x` along the ridge, `z` down the slopes
    let (la, lb) = if across { (b, a) } else { (a, b) };
    let w = |x: f32, z: f32, y: f32| if across { fr.w(z, x, y) } else { fr.w(x, z, y) };
    let dir = |x: f32, z: f32| if across { fr.dir(z, x, 0.0) } else { fr.dir(x, z, 0.0) };
    let (ea, eb) = (la + e, lb + e);
    let low = eave - e * pitch;
    let top = eave + lb * pitch;
    out.part = part;
    for s in [-1.0f32, 1.0] {
        let d = dir(0.0, s);
        let n = [d[0] * pitch, 1.0, d[2] * pitch];
        face(out, [w(-ea, s * eb, low + th), w(ea, s * eb, low + th), w(ea, 0.0, top + th), w(-ea, 0.0, top + th)], n, col);
        // underneath, past the wall: the soffit
        out.part = HousePart::Reveal as u8;
        let soffit = [col[0] * 0.5, col[1] * 0.5, col[2] * 0.5];
        face(out, [w(-ea, s * lb, eave), w(ea, s * lb, eave), w(ea, s * eb, low), w(-ea, s * eb, low)], [0.0, -1.0, 0.0], soffit);
        // the fascia along the eave
        out.part = part;
        face(out, [w(-ea, s * eb, low), w(ea, s * eb, low), w(ea, s * eb, low + th), w(-ea, s * eb, low + th)], d, [col[0] * 0.7, col[1] * 0.7, col[2] * 0.7]);
        // the verges: the roof's underside past the gable, and its edge
        for x in [-1.0f32, 1.0] {
            let along = dir(x, 0.0);
            out.part = HousePart::Reveal as u8;
            face(out, [w(x * la, 0.0, top), w(x * ea, 0.0, top), w(x * ea, s * eb, low), w(x * la, s * eb, low)], [0.0, -1.0, 0.0], soffit);
            out.part = part;
            face(out, [w(x * ea, 0.0, top), w(x * ea, s * eb, low), w(x * ea, s * eb, low + th), w(x * ea, 0.0, top + th)], along, [col[0] * 0.7, col[1] * 0.7, col[2] * 0.7]);
        }
        // the gable's top under the verge's soffit, so no sky shows there
        out.part = part;
    }
    let cap = [col[0] * 0.85, col[1] * 0.82, col[2] * 0.8];
    ridge_cap(out, w(-ea, 0.0, top + th), w(ea, 0.0, top + th), cap);
}

/// A row of half-round ridge tiles from `a` to `b`: a low ridge of two faces.
fn ridge_cap(out: &mut Builder, a: [f32; 3], b: [f32; 3], col: [f32; 3]) {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    let l = (dx * dx + dz * dz).sqrt();
    if l < 0.1 {
        return;
    }
    let (px, pz) = (-dz / l * 0.32, dx / l * 0.32);
    let lift = 0.28;
    let up = |p: [f32; 3], k: f32| [p[0], p[1] + k, p[2]];
    let side = |p: [f32; 3], s: f32| [p[0] + px * s, p[1] - 0.04, p[2] + pz * s];
    for s in [-1.0f32, 1.0] {
        face(out, [side(a, s), side(b, s), up(b, lift), up(a, lift)], [px * s, 0.6, pz * s], col);
    }
}

/// A chimney: a plastered shaft from `y0` to `y1`, a stone cap and a little
/// roof of tiles on it.
#[allow(clippy::too_many_arguments)]
fn chimney(out: &mut Builder, fr: &Frame, s: f32, t: f32, y0: f32, y1: f32, wall: [f32; 3], rustic: bool, roof_c: [f32; 3], roof_part: u8) {
    let (hs, ht) = (0.75, 0.6);
    let sides = [
        Facade { o: [s - hs, t - ht], d: [1.0, 0.0], n: [0.0, -1.0], len: 2.0 * hs },
        Facade { o: [s + hs, t - ht], d: [0.0, 1.0], n: [1.0, 0.0], len: 2.0 * ht },
        Facade { o: [s + hs, t + ht], d: [-1.0, 0.0], n: [0.0, 1.0], len: 2.0 * hs },
        Facade { o: [s - hs, t + ht], d: [0.0, -1.0], n: [-1.0, 0.0], len: 2.0 * ht },
    ];
    for f in &sides {
        out.part = if rustic { HousePart::Rustic as u8 } else { HousePart::Plaster as u8 };
        fbox(out, fr, f, 0.0, f.len, y0, y1, -0.01, 0.0, FRONT, wall);
        out.part = Part::Stone as u8;
        fbox(out, fr, f, -0.2, f.len + 0.2, y1, y1 + 0.25, 0.0, 0.2, FRONT | BOTTOM, linear([0.6, 0.58, 0.54]));
        // the vents: dark gaps under the little roof
        out.part = HousePart::Reveal as u8;
        fbox(out, fr, f, 0.15, f.len - 0.15, y1 + 0.25, y1 + 0.8, -0.3, -0.29, FRONT, [0.02, 0.02, 0.02]);
    }
    out.part = Part::Stone as u8;
    face(out, [fr.w(s - hs - 0.2, t - ht - 0.2, y1 + 0.25), fr.w(s + hs + 0.2, t - ht - 0.2, y1 + 0.25), fr.w(s + hs + 0.2, t + ht + 0.2, y1 + 0.25), fr.w(s - hs - 0.2, t + ht + 0.2, y1 + 0.25)], [0.0, 1.0, 0.0], linear([0.6, 0.58, 0.54]));
    // posts at the corners holding the little roof
    for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let f = Facade { o: [s + cx * (hs - 0.1) - 0.1, t + cy * (ht - 0.1) - 0.1], d: [1.0, 0.0], n: [0.0, -1.0], len: 0.2 };
        fbox(out, fr, &f, 0.0, 0.2, y1 + 0.25, y1 + 0.8, -0.2, 0.0, FRONT | BACK | ENDS, [wall[0] * 0.8, wall[1] * 0.8, wall[2] * 0.8]);
    }
    hip_roof(out, fr, s, t, hs, ht, 0.25, y1 + 0.8, 0.5, roof_part, roof_c, false);
}
