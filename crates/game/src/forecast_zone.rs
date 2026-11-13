//! The forecast's zones painted by a shader: see `shaders/forecast_zone.wgsl`.
//! The geometry is the isochrones of `forecast_view`, closed lines; a grid
//! draped over the island carries each vertex's signed distance to them, and
//! the shader fills what is inside and draws the line.

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use rocca::forecast::HORIZONS;
use scenario::{Pos, Scenario};

const ZONE_SHADER_HANDLE: Handle<Shader> = uuid_handle!("7a21c0de-55b3-4e90-8d16-3f4ab2c7e581");

/// Spacing of the draped grid, metres.
const GRID_M: f32 = 30.0;
/// Lift above the ground, as the other signs'.
const LIFT_M: f32 = crate::rings::RING_LIFT_M;
/// Opacity of the fill of +1 h and +3 h, and of the contour lines (none: they
/// made too much confusion, user 2026-10-17).
const FILL: [f32; HORIZONS] = [0.6, 0.5];
const LINE: f32 = 0.0;
/// Distances are held within this, metres: far from every line it is all the same.
const REACH_M: f32 = 300.0;
/// Gaussian smoothing of the arrival-time field, in fire cells.
const ETA_SIGMA_CELLS: f32 = 1.5;
/// The same pull toward the camera as `FlatMaterial`'s.
const PULL: Vec4 = Vec4::new(0.04, 40.0, 220.0, 0.0);

#[derive(Clone, Copy, ShaderType)]
pub struct ZoneUniform {
    near: Vec4,
    far: Vec4,
    line: Vec4,
    pull: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct ZoneMaterial {
    #[uniform(0)]
    pub uniform: ZoneUniform,
}

impl ZoneMaterial {
    pub fn saturation(&self) -> f32 {
        self.uniform.pull.w
    }

    /// Saturation gain of the tactical view (`tactical.rs`): 0 = as given.
    pub fn set_saturation(&mut self, gain: f32) {
        self.uniform.pull.w = gain;
    }

    /// `colours` as sRGB, `+1 h` first.
    pub fn new(colours: [[f32; 3]; HORIZONS]) -> Self {
        let lin = |c: [f32; 3], a: f32| {
            let l = Color::srgb(c[0], c[1], c[2]).to_linear();
            Vec4::new(l.red, l.green, l.blue, a)
        };
        ZoneMaterial {
            uniform: ZoneUniform {
                near: lin(colours[0], FILL[0]),
                far: lin(colours[1], FILL[1]),
                line: Vec4::new(LINE, 0.0, 0.0, 0.0),
                pull: PULL,
            },
        }
    }
}

impl Material for ZoneMaterial {
    // no depth prepass and no shadows (was `MaterialPlugin { prepass_enabled, shadows_enabled }` before Bevy 0.17)
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn vertex_shader() -> ShaderRef {
        ZONE_SHADER_HANDLE.into()
    }
    fn fragment_shader() -> ShaderRef {
        ZONE_SHADER_HANDLE.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        -2.0
    }
    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct ZonePlugin;

impl Plugin for ZonePlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, ZONE_SHADER_HANDLE, "shaders/forecast_zone.wesl");
        app.add_plugins(MaterialPlugin::<ZoneMaterial>::default());
    }
}

/// Signed distance from `p` to the closed `loops` (positive inside, even-odd
/// across the loops), held within `REACH_M`.
fn signed_distance(loops: &[Vec<Pos>], p: Pos) -> f32 {
    let mut inside = false;
    let mut d2 = REACH_M * REACH_M;
    for l in loops {
        let n = l.len();
        for i in 0..n {
            let (a, b) = (l[i], l[(i + 1) % n]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
            let (ex, ey) = (b.x - a.x, b.y - a.y);
            let t = (((p.x - a.x) * ex + (p.y - a.y) * ey) / (ex * ex + ey * ey).max(1e-6)).clamp(0.0, 1.0);
            let (dx, dy) = (a.x + t * ex - p.x, a.y + t * ey - p.y);
            d2 = d2.min(dx * dx + dy * dy);
        }
    }
    let d = d2.sqrt();
    if inside { d } else { -d }
}

/// A grid over the whole world, draped on the terrain, with the distances to
/// each horizon's lines in the first texture coordinates and the arrival time
/// (minutes after issue) in the second's x.
pub fn grid_mesh(scn: &Scenario, lines: &[Vec<Vec<Pos>>; HORIZONS], eta_s: &[f32]) -> Mesh {
    let (w, h) = (scn.world.width_m, scn.world.height_m);
    let (nx, ny) = ((w / GRID_M).ceil() as usize, (h / GRID_M).ceil() as usize);
    // where a line is near at all
    let boxes: [Option<(Pos, Pos)>; HORIZONS] = std::array::from_fn(|k| {
        let pts = lines[k].iter().flatten();
        let (mut lo, mut hi) = (Pos { x: f32::MAX, y: f32::MAX }, Pos { x: f32::MIN, y: f32::MIN });
        let mut any = false;
        for p in pts {
            any = true;
            lo = Pos { x: lo.x.min(p.x), y: lo.y.min(p.y) };
            hi = Pos { x: hi.x.max(p.x), y: hi.y.max(p.y) };
        }
        any.then_some((lo, hi))
    });
    // the forecast's arrival time at a point, minutes after issue: the
    // members' median per fire cell is blocky and noisy, so it is smoothed
    // first and then read with a bicubic filter, for a clean wave front
    let world = scn.world;
    let last = rocca::forecast::HORIZONS_S[HORIZONS - 1] as f32;
    let (rows, cols) = (world.fire_rows, world.fire_cols);
    let field = smooth_eta(eta_s, rows, cols, last);
    let cell = |r: i64, c: i64| {
        let (r, c) = (r.clamp(0, rows as i64 - 1), c.clamp(0, cols as i64 - 1));
        field[r as usize * cols + c as usize]
    };
    let eta_min = |p: Pos| {
        let (fc, fr) = (p.x / world.cellsize - 0.5, (world.height_m - p.y) / world.cellsize - 0.5);
        let (c0, r0) = (fc.floor(), fr.floor());
        let (wc, wr) = (catmull_rom(fc - c0), catmull_rom(fr - r0));
        let (c0, r0) = (c0 as i64, r0 as i64);
        let mut v = 0.0;
        for (j, wj) in wr.iter().enumerate() {
            for (i, wi) in wc.iter().enumerate() {
                v += wj * wi * cell(r0 + j as i64 - 1, c0 + i as i64 - 1);
            }
        }
        v.clamp(0.0, last) / 60.0
    };
    let mut etas: Vec<[f32; 2]> = Vec::new();
    let mut positions = Vec::with_capacity((nx + 1) * (ny + 1));
    let mut dists = Vec::with_capacity(positions.capacity());
    for j in 0..=ny {
        for i in 0..=nx {
            let p = Pos { x: (i as f32 * GRID_M).min(w), y: (j as f32 * GRID_M).min(h) };
            etas.push([eta_min(p), 0.0]);
            positions.push([p.x, scn.terrain.height_at(p) + LIFT_M, -p.y]);
            let d: [f32; HORIZONS] = std::array::from_fn(|k| match boxes[k] {
                Some((lo, hi)) if p.x > lo.x - REACH_M && p.x < hi.x + REACH_M && p.y > lo.y - REACH_M && p.y < hi.y + REACH_M => {
                    signed_distance(&lines[k], p)
                }
                _ => -REACH_M,
            });
            dists.push(d);
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mut indices = Vec::with_capacity(nx * ny * 6);
    let at = |i: usize, j: usize| (j * (nx + 1) + i) as u32;
    for j in 0..ny {
        for i in 0..nx {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            // a cell with no line near it is not drawn
            let near = [a, b, c, d].iter().any(|&v| dists[v as usize].iter().any(|&x| x > -REACH_M + 1.0) );
            if near {
                indices.extend([a, c, b, a, d, c]);
            }
        }
    }
    // drawn only (`forecast_view`): for the card alone
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, dists);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_1, etas);
    m.insert_indices(Indices::U32(indices));
    m
}

/// Catmull-Rom weights of the four samples around a point at fraction `t`.
fn catmull_rom(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// The arrival times (seconds) with what never burns held at `last`, then
/// blurred by a separable Gaussian of `ETA_SIGMA_CELLS`.
fn smooth_eta(eta_s: &[f32], rows: usize, cols: usize, last: f32) -> Vec<f32> {
    let mut f: Vec<f32> = (0..rows * cols).map(|i| eta_s.get(i).map_or(last, |t| t.min(last))).collect();
    let reach = (ETA_SIGMA_CELLS * 3.0).ceil() as i64;
    let k: Vec<f32> = (-reach..=reach).map(|d| (-0.5 * (d as f32 / ETA_SIGMA_CELLS).powi(2)).exp()).collect();
    let norm: f32 = k.iter().sum();
    let mut g = f.clone();
    for horizontal in [true, false] {
        for r in 0..rows as i64 {
            for c in 0..cols as i64 {
                let mut acc = 0.0;
                for (n, w) in k.iter().enumerate() {
                    let d = n as i64 - reach;
                    let (rr, cc) = if horizontal { (r, (c + d).clamp(0, cols as i64 - 1)) } else { ((r + d).clamp(0, rows as i64 - 1), c) };
                    acc += w * f[rr as usize * cols + cc as usize];
                }
                g[r as usize * cols + c as usize] = acc / norm;
            }
        }
        std::mem::swap(&mut f, &mut g);
    }
    f
}
