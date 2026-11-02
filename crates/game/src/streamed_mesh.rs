//! Geometry rebuilt every frame (the fire's flames, smoke and sparks) drawn
//! through a Bevy `Mesh` that is never replaced: its vertices are written in
//! place, in the buffer the mesh allocator gave it.
//!
//! Why (2026-11-01, an iPhone page at 1.2 GB): a `Mesh` inserted anew every
//! frame is prepared again, and on WebGL2 (no base vertex) that is a new GL
//! vertex buffer each frame plus a partial write into the shared index slab,
//! several MB that the browser's Metal backend may copy whole to avoid a
//! stall. Measured in WebKit: 15 GL buffers created and deleted per frame, the
//! three fire meshes 178 KiB of them and three writes into a 5–12 MB slab.
//!
//! A streamed mesh holds room for `capacity` items of one [`Shape`] (a flame
//! tongue, a billboard) whose indices never change. Each frame the producer
//! packs the items' vertices in Bevy's interleaved order (position, normal,
//! uv, colour) and [`Streamed::write`] queues them; in the render world
//! [`upload`] copies them over the mesh's vertices with one `write_buffer`.
//! Unused room is zeros: triangles of no area, which draw nothing. Only when
//! the items outgrow the room is the mesh built again, twice as large.
//!
//! [`write_range`] does the same for part of any mesh that keeps its CPU copy
//! (the buildings' colours as the fire reaches them): the copy is changed
//! untracked, so Bevy does not prepare the mesh again, and the changed
//! vertices are written over the old ones.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::RenderQueue;
use bevy::render::{MainWorld, Render, RenderApp, RenderSystems};

pub struct StreamedMeshPlugin;

impl Plugin for StreamedMeshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StreamedFrame>();
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        render
            .init_resource::<StreamedFrame>()
            .add_systems(ExtractSchedule, extract)
            .add_systems(Render, upload.in_set(RenderSystems::PrepareResources));
    }
}

/// The items one streamed mesh is made of: `verts` vertices each, joined by
/// `indices` (relative to the item's first vertex).
#[derive(Clone, Copy)]
pub struct Shape {
    pub verts: u32,
    pub indices: &'static [u32],
}

/// Vertices written this frame, by mesh, with their offset in bytes from the
/// mesh's first vertex (main world, moved to the render world at extraction).
#[derive(Resource, Default)]
pub struct StreamedFrame(Vec<(AssetId<Mesh>, u64, Vec<u8>)>);

/// Queue the vertices `range` of `mesh` as they are in its CPU copy, packed as
/// Bevy packs them (every attribute, in the order of their ids).
pub fn write_range(frame: &mut StreamedFrame, id: AssetId<Mesh>, mesh: &Mesh, range: std::ops::Range<usize>) {
    let Ok(attributes) = mesh.try_attributes() else { return };
    let parts: Vec<(&[u8], usize)> = attributes.map(|(a, v)| (v.get_bytes(), a.format.size() as usize)).collect();
    let stride: usize = parts.iter().map(|p| p.1).sum();
    let mut out = Vec::with_capacity(range.len() * stride);
    for i in range.clone() {
        for (bytes, size) in &parts {
            out.extend_from_slice(&bytes[i * size..(i + 1) * size]);
        }
    }
    if !out.is_empty() {
        frame.0.push((id, (range.start * stride) as u64, out));
    }
}

pub struct Streamed {
    pub handle: Handle<Mesh>,
    shape: Shape,
    normals: bool,
    /// Items the mesh has room for.
    capacity: u32,
    /// Bytes written last frame: what this frame has to cover, zeros past
    /// its own items, so last frame's items do not linger.
    written: usize,
}

/// Bytes of one vertex: position, the normal if any, uv, colour.
pub fn stride(normals: bool) -> usize {
    12 + if normals { 12 } else { 0 } + 8 + 16
}

/// Append one vertex in Bevy's interleaved order (attribute ids 0, 1, 2, 5).
pub fn push_vertex(out: &mut Vec<u8>, p: [f32; 3], n: Option<[f32; 3]>, uv: [f32; 2], c: [f32; 4]) {
    match n {
        Some(n) => put(out, [p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1], c[0], c[1], c[2], c[3]]),
        None => put(out, [p[0], p[1], p[2], uv[0], uv[1], c[0], c[1], c[2], c[3]]),
    }
}

/// One copy per vertex: extended float by float the packing was a fifth of
/// the flames' time in the browser (profile of 2026-11-01). Native byte
/// order, as Bevy uploads its own vertices.
fn put<const N: usize>(out: &mut Vec<u8>, v: [f32; N]) {
    out.extend_from_slice(bytemuck::cast_slice(&v));
}

impl Streamed {
    pub fn new(meshes: &mut Assets<Mesh>, shape: Shape, normals: bool) -> Streamed {
        let capacity = 64;
        Streamed { handle: meshes.add(room(shape, normals, capacity)), shape, normals, capacity, written: 0 }
    }

    /// This frame's items, packed with [`push_vertex`] (`items` of them).
    pub fn write(&mut self, meshes: &mut Assets<Mesh>, frame: &mut StreamedFrame, mut bytes: Vec<u8>, items: u32) {
        debug_assert_eq!(bytes.len(), (items * self.shape.verts) as usize * stride(self.normals));
        if items > self.capacity {
            self.capacity = items.next_power_of_two();
            let _ = meshes.insert(&self.handle, room(self.shape, self.normals, self.capacity));
            self.written = 0;
        }
        let used = bytes.len();
        if used < self.written {
            bytes.resize(self.written, 0);
        }
        self.written = used;
        if !bytes.is_empty() {
            frame.0.push((self.handle.id(), 0, bytes));
        }
    }
}

/// A mesh with room for `capacity` items, all zeros, and their indices.
fn room(shape: Shape, normals: bool, capacity: u32) -> Mesh {
    let n = (capacity * shape.verts) as usize;
    // only the GPU copy is used: the vertices are written there
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; n]);
    if normals {
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32; 3]; n]);
    }
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; n]);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; n]);
    let indices = (0..capacity).flat_map(|i| shape.indices.iter().map(move |k| i * shape.verts + k)).collect();
    m.insert_indices(Indices::U32(indices));
    m
}

fn extract(mut main: ResMut<MainWorld>, mut frame: ResMut<StreamedFrame>) {
    let mut taken = std::mem::take(&mut main.resource_mut::<StreamedFrame>().0);
    frame.0.append(&mut taken);
}

/// Over the vertices of each streamed mesh, after the allocator placed it
/// (`PrepareAssets`). A mesh not placed yet keeps its zeros this frame.
fn upload(mut frame: ResMut<StreamedFrame>, allocator: Res<MeshAllocator>, meshes: Res<RenderAssets<RenderMesh>>, queue: Res<RenderQueue>) {
    for (id, offset, bytes) in frame.0.drain(..) {
        let (Some(slice), Some(mesh)) = (allocator.mesh_vertex_slice(&id), meshes.get(id)) else { continue };
        let stride = mesh.layout.0.layout().array_stride;
        let room = ((slice.range.end - slice.range.start) as u64 * stride).saturating_sub(offset);
        let n = (bytes.len() as u64).min(room) & !3;
        if n > 0 && offset % 4 == 0 {
            queue.write_buffer(slice.buffer, slice.range.start as u64 * stride + offset, &bytes[..n as usize]);
        }
    }
}
