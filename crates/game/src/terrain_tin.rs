//! The terrain as an adaptive triangle mesh (TIN) instead of a regular grid.
//!
//! Each chunk and level is a Delaunay triangulation, refined greedily: start
//! from the chunk's edge samples and the road kerbs, then keep inserting the
//! DEM sample farthest from the surface until none is farther than the level's
//! tolerance (Garland & Heckbert's greedy insertion). Flat ground and the sea
//! floor take a few large triangles; cliffs, crests and the cut and fill of the
//! roads take many small ones.
//!
//! The kerbs (the edges of the level road bench, `scenario::grade`) are
//! vertices from the start, every few metres along both sides of each road:
//! the carriageway is a flat ribbon with a crisp edge, whatever the DEM's
//! posting (in the browser it is 20 m, wider than a road).
//!
//! The chunk's edge vertices are the base grid's edge samples at the level's
//! stride, exactly as the grid mesh had them: two neighbouring chunks at the
//! same level share their edge, and the skirts cover the cracks between levels.

use spade::{DelaunayTriangulation, HasPosition, HierarchyHintGenerator, Point2, Triangulation};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// The shaped heightfield the mesh is refined against: a window of `cols` by
/// `rows` samples starting at (`c_off`, `r_off`) in grid units. Normals given,
/// or taken from the heights' central differences.
pub(crate) struct GridView<'a> {
    pub cols: usize,
    pub rows: usize,
    pub c_off: usize,
    pub r_off: usize,
    pub posting: f32,
    pub elev: &'a [f32],
    pub normal: Option<&'a [[f32; 3]]>,
}

impl GridView<'_> {
    #[inline]
    fn index(&self, c: usize, r: usize) -> usize {
        (r - self.r_off) * self.cols + (c - self.c_off)
    }

    #[inline]
    fn z(&self, c: usize, r: usize) -> f32 {
        self.elev[self.index(c, r)]
    }

    /// Bevy's frame (rows run south, so z flips).
    fn normal(&self, c: usize, r: usize) -> [f32; 3] {
        if let Some(n) = self.normal {
            return n[self.index(c, r)];
        }
        let (cl, cr) = (c.saturating_sub(1).max(self.c_off), (c + 1).min(self.c_off + self.cols - 1));
        let (ru, rd) = (r.saturating_sub(1).max(self.r_off), (r + 1).min(self.r_off + self.rows - 1));
        // world: x east, y north (row up); Bevy z = -y
        let dzdx = (self.z(cr, r) - self.z(cl, r)) / ((cr - cl).max(1) as f32 * self.posting);
        let dzdy = (self.z(c, ru) - self.z(c, rd)) / ((rd - ru).max(1) as f32 * self.posting);
        let (nx, ny, nz) = (-dzdx, 1.0, dzdy);
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        [nx / len, ny / len, nz / len]
    }
}

/// A point that must be a vertex: in base-grid units (column, row), its
/// height and its normal (Bevy's frame).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fixed {
    pub c: f32,
    pub r: f32,
    pub z: f32,
    pub n: [f32; 3],
}

#[derive(Clone, Copy)]
struct V {
    p: Point2<f64>,
    z: f32,
    n: [f32; 3],
}

impl HasPosition for V {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        self.p
    }
}

type Tin = DelaunayTriangulation<V, (), (), (), HierarchyHintGenerator<f64>>;

#[derive(PartialEq)]
struct Worst {
    err: f32,
    face: usize,
    version: u32,
    sample: (u32, u32),
}
impl Eq for Worst {}
impl PartialOrd for Worst {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Worst {
    fn cmp(&self, o: &Self) -> Ordering {
        self.err.total_cmp(&o.err)
    }
}

/// The geometry of one chunk at one level, in the chunk's frame.
pub(crate) struct ChunkGeometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// Samples `c0..=c0+ncx` by `r0..=r0+ncy` of the base grid; the edge every
/// `f`-th sample (the last one always the chunk's corner); `fixed` the road
/// kerbs strictly inside; refined to `tol` metres. `skirt` is the skirt's
/// depth, metres.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chunk(
    grid: &GridView,
    (c0, r0): (usize, usize),
    (ncx, ncy): (usize, usize),
    f: usize,
    fixed: &[Fixed],
    tol: f32,
    origin: [f32; 2],
    height_m: f32,
    skirt: f32,
) -> ChunkGeometry {
    let at = |c: usize, r: usize| V { p: Point2::new(c as f64, r as f64), z: grid.z(c, r), n: grid.normal(c, r) };
    // The edges: every sample's profile simplified to the tolerance, from the
    // edge's own samples only and always in the same direction, so the chunk
    // across the edge keeps exactly the same vertices. `f` caps the gap.
    let mut seed = Vec::with_capacity(4 * (ncx + ncy) / f.max(1) + fixed.len());
    let max_gap = (f * EDGE_GAP).max(1);
    for r in [r0, r0 + ncy] {
        let zs: Vec<f32> = (c0..=c0 + ncx).map(|c| grid.z(c, r)).collect();
        for k in edge_keep(&zs, tol, max_gap) {
            seed.push(at(c0 + k, r));
        }
    }
    for c in [c0, c0 + ncx] {
        let zs: Vec<f32> = (r0..=r0 + ncy).map(|r| grid.z(c, r)).collect();
        let keep = edge_keep(&zs, tol, max_gap);
        // the corners are already in
        for &k in &keep[1..keep.len() - 1] {
            seed.push(at(c, r0 + k));
        }
    }
    for k in fixed {
        seed.push(V { p: Point2::new(k.c as f64, k.r as f64), z: k.z, n: k.n });
    }
    let mut tin: Tin = Tin::bulk_load(seed).expect("terrain chunk triangulation");

    // interior samples, and whether each is already a vertex
    let (w, h) = (ncx - 1, ncy - 1);
    let mut taken = vec![false; w * h];
    let mut version: Vec<u32> = Vec::new();
    let mut heap = BinaryHeap::new();

    // the worst interior sample under one face
    let scan = |tin: &Tin, face: spade::handles::FixedFaceHandle<spade::handles::InnerTag>, taken: &[bool]| -> Option<(f32, (u32, u32))> {
        let fh = tin.face(face);
        let [a, b, c] = fh.vertices().map(|v| *v.data());
        let (ax, ay, bx, by, cx, cy) = (a.p.x, a.p.y, b.p.x, b.p.y, c.p.x, c.p.y);
        let det = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy);
        if det.abs() < 1e-12 {
            return None;
        }
        let lo_c = (ax.min(bx).min(cx).ceil() as usize).max(c0 + 1);
        let hi_c = (ax.max(bx).max(cx).floor() as usize).min(c0 + ncx - 1);
        let lo_r = (ay.min(by).min(cy).ceil() as usize).max(r0 + 1);
        let hi_r = (ay.max(by).max(cy).floor() as usize).min(r0 + ncy - 1);
        let mut best: Option<(f32, (u32, u32))> = None;
        for r in lo_r..=hi_r {
            let py = r as f64;
            for cc in lo_c..=hi_c {
                if taken[(r - r0 - 1) * w + (cc - c0 - 1)] {
                    continue;
                }
                let px = cc as f64;
                let l1 = ((by - cy) * (px - cx) + (cx - bx) * (py - cy)) / det;
                let l2 = ((cy - ay) * (px - cx) + (ax - cx) * (py - cy)) / det;
                let l3 = 1.0 - l1 - l2;
                if l1 < -1e-9 || l2 < -1e-9 || l3 < -1e-9 {
                    continue;
                }
                let z = (l1 * a.z as f64 + l2 * b.z as f64 + l3 * c.z as f64) as f32;
                let e = (grid.z(cc, r) - z).abs();
                if best.is_none_or(|(b, _)| e > b) {
                    best = Some((e, (cc as u32, r as u32)));
                }
            }
        }
        best
    };
    let push = |tin: &Tin, face: spade::handles::FixedFaceHandle<spade::handles::InnerTag>, taken: &[bool], version: &mut Vec<u32>, heap: &mut BinaryHeap<Worst>| {
        let i = face.index();
        if version.len() <= i {
            version.resize(i + 1, 0);
        }
        version[i] += 1;
        if let Some((err, sample)) = scan(tin, face, taken) {
            if err > tol {
                heap.push(Worst { err, face: i, version: version[i], sample });
            }
        }
    };
    let faces: Vec<_> = tin.fixed_inner_faces().collect();
    for face in faces {
        push(&tin, face, &taken, &mut version, &mut heap);
    }
    while let Some(Worst { face, version: v, sample: (c, r), .. }) = heap.pop() {
        if version.get(face) != Some(&v) {
            continue;
        }
        let (c, r) = (c as usize, r as usize);
        taken[(r - r0 - 1) * w + (c - c0 - 1)] = true;
        let Ok(vh) = tin.insert(at(c, r)) else { continue };
        // every face the insertion made or changed has the new vertex as a corner
        let star: Vec<_> = tin.vertex(vh).out_edges().filter_map(|e| e.face().as_inner().map(|f| f.fix())).collect();
        for face in star {
            push(&tin, face, &taken, &mut version, &mut heap);
        }
    }

    // to the chunk's frame (world x east, Bevy z south), as the grid mesh
    let to_local = |v: &V| [v.p.x as f32 * grid.posting - origin[0], v.z, v.p.y as f32 * grid.posting - height_m - origin[1]];
    // Triangles in Morton order of their centres, vertices numbered by first
    // use: the triangulation's own order is scattered over the chunk, which
    // costs the GPU its vertex cache.
    let mut faces: Vec<(u64, [usize; 3])> = tin
        .inner_faces()
        .map(|fh| {
            let vs = fh.vertices();
            let (x, y) = vs.iter().fold((0.0, 0.0), |(x, y), v| (x + v.position().x, y + v.position().y));
            let key = morton((((x / 3.0) - c0 as f64) * 64.0) as u32, (((y / 3.0) - r0 as f64) * 64.0) as u32);
            // spade winds counter-clockwise in (column, row); rows run south,
            // so that is clockwise seen from above: flip
            (key, [vs[0].fix().index(), vs[2].fix().index(), vs[1].fix().index()])
        })
        .collect();
    faces.sort_unstable_by_key(|f| f.0);
    let mut remap = vec![u32::MAX; tin.num_vertices()];
    let mut positions = Vec::with_capacity(tin.num_vertices());
    let mut normals = Vec::with_capacity(tin.num_vertices());
    let mut indices = Vec::with_capacity(faces.len() * 3);
    let verts: Vec<V> = tin.vertices().map(|v| *v.data()).collect();
    for (_, f) in &faces {
        for &v in f {
            if remap[v] == u32::MAX {
                remap[v] = positions.len() as u32;
                positions.push(to_local(&verts[v]));
                normals.push(verts[v].n);
            }
            indices.push(remap[v]);
        }
    }

    // skirts: the edge vertices in order along each side, dropped
    let (xmin, xmax, ymin, ymax) = (c0 as f64, (c0 + ncx) as f64, r0 as f64, (r0 + ncy) as f64);
    let mut sides: [Vec<(f64, u32)>; 4] = Default::default();
    for v in tin.vertices() {
        let (p, i) = (v.position(), remap[v.fix().index()]);
        if i == u32::MAX {
            continue;
        }
        if p.y == ymin { sides[0].push((p.x, i)); }
        if p.y == ymax { sides[1].push((p.x, i)); }
        if p.x == xmin { sides[2].push((p.y, i)); }
        if p.x == xmax { sides[3].push((p.y, i)); }
    }
    for mut side in sides {
        side.sort_by(|a, b| a.0.total_cmp(&b.0));
        let base = positions.len() as u32;
        for &(_, v) in &side {
            let mut p = positions[v as usize];
            p[1] -= skirt;
            let n = normals[v as usize];
            positions.push(p);
            normals.push(n);
        }
        for k in 0..side.len().saturating_sub(1) as u32 {
            let (a, b, c, d) = (side[k as usize].1, side[k as usize + 1].1, base + k, base + k + 1);
            indices.extend_from_slice(&[a, b, c, c, b, d, a, c, b, c, d, b]);
        }
    }
    ChunkGeometry { positions, normals, indices }
}

/// Interleave the bits of two 16-bit coordinates.
fn morton(x: u32, y: u32) -> u64 {
    fn spread(mut v: u64) -> u64 {
        v &= 0xffff_ffff;
        v = (v | (v << 16)) & 0x0000_ffff_0000_ffff;
        v = (v | (v << 8)) & 0x00ff_00ff_00ff_00ff;
        v = (v | (v << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
        v = (v | (v << 2)) & 0x3333_3333_3333_3333;
        (v | (v << 1)) & 0x5555_5555_5555_5555
    }
    spread(x as u64) | (spread(y as u64) << 1)
}

/// Largest gap between edge vertices, in multiples of the level's stride.
const EDGE_GAP: usize = 8;

/// Which samples of an edge profile to keep: Douglas-Peucker to `tol`, both
/// ends always, no gap wider than `max_gap` samples.
fn edge_keep(zs: &[f32], tol: f32, max_gap: usize) -> Vec<usize> {
    let n = zs.len();
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (za, zb) = (zs[a], zs[b]);
        let (mut worst, mut at) = (0.0f32, a);
        for i in a + 1..b {
            let line = za + (zb - za) * (i - a) as f32 / (b - a) as f32;
            let e = (zs[i] - line).abs();
            if e > worst {
                (worst, at) = (e, i);
            }
        }
        if worst > tol || b - a > max_gap {
            let m = if worst > tol { at } else { (a + b) / 2 };
            keep[m] = true;
            stack.push((a, m));
            stack.push((m, b));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}
