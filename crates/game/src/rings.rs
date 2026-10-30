//! Draped ring meshes: the one symbol the map overlays share.
//!
//! The rings are annuli draped on the 5 m render field rather than flat discs:
//! a disc at this scale is either buried in a hillside or floating off it, and
//! a filled shape hides what it marks. Wound counter-clockwise from above for
//! the reason the road ribbons are (finding 11).

use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use scenario::{Pos, Scenario};

/// Segments around a ring. Fixed rather than radius-dependent: even the
/// smallest ring is 60 m across on screen, where faceting would show.
const RING_SEGMENTS: usize = 96;

/// Lift above the ground.
///
/// Not a z-fighting margin: a ring on the ground is *under the canopy*, and the
/// plants are 5-15 m tall, so at 2 m it is drawn perfectly and seen never
/// (finding 13). Same layer as the household beacons and refuge markers.
pub(crate) const RING_LIFT_M: f32 = 20.0;

/// Ring thickness in metres, scaled with the radius.
fn ring_width(radius_m: f32) -> f32 {
    (radius_m * 0.06).clamp(4.0, 20.0)
}

/// An annulus in world space, draped on the render terrain.
pub(crate) fn ring_mesh(scn: &Scenario, centre: Pos, radius_m: f32) -> Mesh {
    annulus(scn, centre, radius_m, ring_width(radius_m))
}

fn annulus(scn: &Scenario, centre: Pos, radius_m: f32, width_m: f32) -> Mesh {
    let w = width_m * 0.5;
    let (inner, outer) = ((radius_m - w).max(1.0), radius_m + w);

    let mut positions = Vec::with_capacity((RING_SEGMENTS + 1) * 2);
    let mut normals = Vec::with_capacity((RING_SEGMENTS + 1) * 2);
    let mut uvs = Vec::with_capacity((RING_SEGMENTS + 1) * 2);
    let mut indices = Vec::with_capacity(RING_SEGMENTS * 6);

    for i in 0..=RING_SEGMENTS {
        let a = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        for r in [inner, outer] {
            let p = Pos { x: centre.x + c * r, y: centre.y + s * r };
            let h = scn.terrain.height_at(p) + RING_LIFT_M;
            positions.push([p.x, h, -p.y]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([if r == inner { 0.0 } else { 1.0 }, i as f32]);
        }
    }
    for i in 0..RING_SEGMENTS as u32 {
        let a = i * 2;
        indices.extend_from_slice(&[a, a + 2, a + 1, a + 1, a + 2, a + 3]);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A translucent vertical tube standing on a ring: straight, not draped. Its
/// foot is buried under the lowest ground inside the ring and its top is above
/// the highest, so the terrain cuts through it like a pipe through a table-top.
/// Full strength up to the crest, fading to nothing over `wall_height` above it
/// (vertex alpha), a sci-fi column of light.
pub(crate) fn wall_height(radius_m: f32) -> f32 {
    (radius_m * 0.7).clamp(110.0, 260.0)
}

/// Slow turn of a tube about its own axis: its slanted, banded fade sweeps
/// round it, so it reads as a moving beam and not as a still wall.
#[derive(Component, Clone, Copy)]
pub(crate) struct TubeSpin;

/// Where a tube's entity goes: its mesh is built about its own axis.
pub(crate) fn tube_transform(centre: Pos) -> Transform {
    Transform::from_xyz(centre.x, 0.0, -centre.y)
}

/// Turn every tube (`overlays::spin_tubes` runs this each frame).
pub(crate) fn spin(time: Res<Time>, mut q: Query<&mut Transform, With<TubeSpin>>) {
    let a = time.elapsed_secs() * 0.35;
    for mut tf in &mut q {
        tf.rotation = Quat::from_rotation_y(a);
    }
}

/// Built about its axis (x, z relative to `centre`; y absolute), to be placed
/// with [`tube_transform`]. The top edge of the fade is cut obliquely, and the
/// strength runs in diagonal bands, so the turn is visible.
pub(crate) fn wall_mesh(scn: &Scenario, centre: Pos, radius_m: f32) -> Mesh {
    const ROWS: usize = 8;
    let at = |i: usize| {
        let a = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        Pos { x: centre.x + c * radius_m, y: centre.y + s * radius_m }
    };
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for i in 0..RING_SEGMENTS {
        let h = scn.terrain.height_at(at(i));
        lo = lo.min(h);
        hi = hi.max(h);
    }
    let height = wall_height(radius_m);
    let mut positions = Vec::with_capacity((RING_SEGMENTS + 1) * (ROWS + 1));
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity((RING_SEGMENTS + 1) * (ROWS + 1));
    let mut indices = Vec::with_capacity(RING_SEGMENTS * 6 * ROWS);
    for i in 0..=RING_SEGMENTS {
        let a = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
        let p = at(i);
        // the fade ends higher on one side than the other: a slanted cut
        let top = hi + height * (0.55 + 0.45 * a.cos());
        for j in 0..=ROWS {
            let f = j as f32 / ROWS as f32;
            // below the crest, solid; above it, down to nothing at the slanted top
            let y = if j == 0 { lo - 40.0 } else { hi + (top - hi) * (j as f32 / ROWS as f32) };
            let fade = if j == 0 { 1.0 } else { (1.0 - f).powf(1.3) };
            // diagonal bands: the strength depends on the angle and on the height together
            let band = 0.8 + 0.2 * (a * 3.0 + f * 9.0).sin();
            positions.push([p.x - centre.x, y, -(p.y - centre.y)]);
            colors.push([1.0, 1.0, 1.0, (1.0 * fade * band).min(1.0)]);
        }
    }
    let stride = ROWS as u32 + 1;
    for i in 0..RING_SEGMENTS as u32 {
        for j in 0..ROWS as u32 {
            let a = i * stride + j;
            let b = a + stride;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    let n = positions.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A route drawn on the map: a ribbon `width_m` wide along `pts`, draped like
/// the rings. With `dash_m`, drawn and skipped in turns of that length, so a
/// proposed route reads apart from one being driven.
pub(crate) fn path_mesh(scn: &Scenario, pts: &[Pos], width_m: f32, dash_m: Option<f32>) -> Mesh {
    path_mesh_at(scn, pts, width_m, dash_m, RING_LIFT_M)
}

/// [`path_mesh`] at another lift: an under-stroke sits a little below its line.
///
/// One continuous strip, resampled every few metres, its sides offset along
/// the mitred normal of the two segments meeting at each sample: a wide line
/// on a wiggly contour folds inside itself rather than throwing spikes out.
pub(crate) fn path_mesh_at(scn: &Scenario, pts: &[Pos], width_m: f32, dash_m: Option<f32>, lift_m: f32) -> Mesh {
    const STEP_M: f32 = 5.0;
    /// A sharp corner's mitre is cut at this many half-widths.
    const MITRE_MAX: f32 = 2.0;
    // samples along the line and how far along each is
    let mut at: Vec<(Pos, f32)> = vec![];
    let mut along = 0.0f32;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.5 {
            continue;
        }
        let steps = (len / STEP_M).ceil() as usize;
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            at.push((Pos { x: a.x + dx * t, y: a.y + dy * t }, along + len * t));
        }
        along += len;
    }
    if let Some(&b) = pts.last() {
        if !at.is_empty() {
            at.push((b, along));
        }
    }
    let n = at.len();
    let unit = |a: Pos, b: Pos| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let l = (dx * dx + dy * dy).sqrt().max(1e-3);
        (dx / l, dy / l)
    };
    let half = width_m * 0.5;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 2);
    // u = metres along the line, v = 0 / 1 across (for materials that use it)
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (c, along_i) = at[i];
        let t_in = if i > 0 { unit(at[i - 1].0, c) } else { unit(c, at[(i + 1).min(n - 1)].0) };
        let t_out = if i + 1 < n { unit(c, at[i + 1].0) } else { t_in };
        let (n_in, n_out) = ((-t_in.1, t_in.0), (-t_out.1, t_out.0));
        let (mx, my) = (n_in.0 + n_out.0, n_in.1 + n_out.1);
        let ml = (mx * mx + my * my).sqrt();
        let (nx, ny) = if ml < 1e-3 { n_out } else { (mx / ml, my / ml) };
        // the mitre: longer at a corner so the sides stay parallel to the segments
        let cos = (nx * n_out.0 + ny * n_out.1).max(1.0 / MITRE_MAX);
        let (ox, oy) = (nx * half / cos, ny * half / cos);
        // each side at its own ground height: across a slope the ribbon lies
        // on it instead of cutting into the hill
        for side in [-1.0f32, 1.0] {
            let q = Pos { x: c.x + ox * side, y: c.y + oy * side };
            positions.push([q.x, scn.terrain.height_at(q) + lift_m, -q.y]);
            uvs.push([along_i, (side + 1.0) * 0.5]);
        }
    }
    // the ground's own bumps and steps would make the line zigzag in a low
    // view: average each side's heights over a few samples
    const SMOOTH: usize = 6;
    for side in 0..2 {
        let raw: Vec<f32> = (0..n).map(|i| positions[i * 2 + side][1]).collect();
        for i in 0..n {
            let (a, b) = (i.saturating_sub(SMOOTH), (i + SMOOTH).min(n - 1));
            positions[i * 2 + side][1] = raw[a..=b].iter().sum::<f32>() / (b - a + 1) as f32;
        }
    }
    let mut indices: Vec<u32> = vec![];
    for i in 0..n.saturating_sub(1) {
        let mid = (at[i].1 + at[i + 1].1) * 0.5;
        if dash_m.is_some_and(|d| (mid / d) as i64 % 2 != 0) {
            continue;
        }
        let s = i as u32 * 2;
        indices.extend_from_slice(&[s, s + 2, s + 1, s + 1, s + 2, s + 3]);
    }
    let n = positions.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
