//! The plants alight, as the CPU sees them: for the flames that stand on a
//! burning plant (`fire_view::plant_torches`, 2026-11-04). Read from the same
//! instants of ignition the shaders burn the plants by (`GroupFire`), so the
//! flames climb a plant exactly as its crown chars.

use bevy::prelude::*;

use std::collections::HashMap;
use std::sync::Mutex;

use super::meshes::{PlantMesh, Tier, VERTEX_BYTES};
use super::{Vegetation, INSTANCE_BYTES, NEVER, PLANT_FLAMING_S};

/// Where a variety's model tops out and where its foliage starts, as shares
/// of the plant's height: the shader climbs the model's own vertices
/// (`place`), so the flames must stop where the model does. Measured once
/// per mesh of a scene.
static SHAPES: Mutex<Option<(u64, HashMap<u32, (f32, f32)>)>> = Mutex::new(None);

fn shape(mesh: &PlantMesh) -> (f32, f32) {
    let n = mesh.vertices.len() / VERTEX_BYTES as usize;
    let (mut all, mut leaves) = (Vec::with_capacity(n), Vec::new());
    for i in 0..n {
        let o = i * VERTEX_BYTES as usize;
        let y = f32::from_le_bytes(mesh.vertices[o + 4..o + 8].try_into().unwrap());
        all.push(y);
        // the kind in the normal's w (`Kind`): leaves above 0.5
        if (mesh.vertices[o + 15] as i8 as f32 / 127.0) > 0.5 {
            leaves.push(y);
        }
    }
    // the top without the odd card that sticks out
    all.sort_by(f32::total_cmp);
    let top = all.get(all.len() * 97 / 100).copied().unwrap_or(0.0);
    if leaves.is_empty() || top <= 0.0 {
        return (top.max(0.1), 0.35 * top.max(0.1));
    }
    leaves.sort_by(f32::total_cmp);
    (top, leaves[leaves.len() / 8])
}

/// A plant on fire: its foot (Bevy frame), crown width and height (m),
/// seconds since it caught, and a number of its own (0..1).
#[derive(Clone, Copy)]
pub(crate) struct Torch {
    pub foot: Vec3,
    pub width: f32,
    /// the top of the model, and the foot of its foliage (m above the foot)
    pub height: f32,
    pub crown_lo: f32,
    pub age: f32,
    pub seed: f32,
}

impl Vegetation {
    /// The plants alight now within `radius` of `eye`, appended to `out`:
    /// those that caught less than [`PLANT_FLAMING_S`] ago. Only tiles near
    /// the eye and only plants the fire has come to are looked at.
    pub(crate) fn torches(&self, eye: Vec3, radius: f32, out: &mut Vec<Torch>) {
        let now = self.uniform[0];
        let r2 = radius * radius;
        let f32_at = |b: &[u8], k: usize| f32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap());
        let mut shapes = SHAPES.lock().unwrap();
        if shapes.as_ref().is_none_or(|s| s.0 != self.scene.id) {
            *shapes = Some((self.scene.id, HashMap::new()));
        }
        let shapes = &mut shapes.as_mut().unwrap().1;
        for tile in &self.scene.tiles {
            let near = eye.clamp(tile.lo, tile.hi);
            if near.distance_squared(eye) > r2 {
                continue;
            }
            for g in tile.groups.clone() {
                let fire = &self.fire[g];
                if fire.caught.is_empty() {
                    continue;
                }
                let group = &self.scene.groups[g];
                // the nearest model the group has
                let Some(m) = group.tiers[Tier::Full as usize].or_else(|| group.tiers.iter().flatten().next().copied()) else {
                    continue;
                };
                let (top, crown) = *shapes.entry(m).or_insert_with(|| shape(&self.scene.meshes[m as usize]));
                for &i in fire.caught.iter() {
                    let at = fire.state[i as usize][0];
                    let age = now - at;
                    if at >= NEVER * 0.5 || !(0.0..PLANT_FLAMING_S).contains(&age) {
                        continue;
                    }
                    let b = &group.instances[i as usize * INSTANCE_BYTES as usize..(i as usize + 1) * INSTANCE_BYTES as usize];
                    let foot = Vec3::new(f32_at(b, 0), f32_at(b, 1), f32_at(b, 2));
                    if foot.distance_squared(eye) > r2 {
                        continue;
                    }
                    let h = f32_at(b, 5);
                    out.push(Torch { foot, width: f32_at(b, 4), height: h * top, crown_lo: h * crown, age, seed: b[29] as f32 / 255.0 });
                }
            }
        }
    }
}
