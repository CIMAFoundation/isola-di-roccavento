//! WebGL2 only: two of Bevy's habits that cost the browser a GPU buffer, or a
//! copy of one, every frame (2026-11-01, an iPhone page charged 1.2 GB, most
//! of it graphics memory; counted in WebKit with the GL calls wrapped).
//!
//! - **Index data in buffers of its own.** Without base vertex every mesh
//!   already has its own vertex buffer, but the indices share slabs of several
//!   MB: a mesh changed (the fire's ground layer, a water jet, a burning block
//!   of houses) is a partial write into a slab the GPU is still reading, which
//!   the browser's Metal backend may answer by copying the whole slab.
//!   `large_threshold` 0 gives each mesh's indices a buffer of their own: a
//!   mesh changed is a new small buffer and the old one freed. Such a buffer
//!   is made at the data's size, which wgpu wants a multiple of 4 bytes: an
//!   odd number of 16-bit indices gets one degenerate index or triangle more
//!   ([`even_u16_indices`]).
//! - **The light clusters, once.** `prepare_clusters_for_cpu_clustering` makes
//!   two new 16 KiB uniform buffers per camera every frame. The island has no
//!   point, spot or clustered light, so their content never changes from
//!   zeros: the system runs again only for a camera without them yet, or if
//!   a clustered light appears.
//! - **No metadata buffer per mesh.** A mesh uploaded without its CPU copy
//!   (`RENDER_WORLD`, most of the scene) gets its bounds as `final_aabb`, and
//!   on WebGL2 Bevy gives every mesh with bounds a uniform buffer of its own
//!   for them; `prepare_mesh_bind_groups` then makes three bind groups per such
//!   mesh every frame (2026-11-01: ~700 per frame, 8 % of the browser's frame
//!   on the title). The bounds there only decompress compressed vertices, and
//!   none of ours are: without them a mesh uses Bevy's shared fallback
//!   ([`no_metadata_without_compression`]). The main world keeps its bounds.

use bevy::ecs::schedule::ScheduleCleanupPolicy;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::light::cluster::GlobalClusterSettings;
use bevy::pbr::{prepare_clusters_for_cpu_clustering, ExtractedClusterableObjects, GlobalClusterableObjectMeta, ViewClusterBindings};
use bevy::prelude::*;
use bevy::render::mesh::allocator::{allocate_and_free_meshes, MeshAllocatorSettings};
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::ExtractedAssets;
use bevy::render::{Render, RenderApp, RenderSystems};

pub struct WebGpuPlugin;

impl Plugin for WebGpuPlugin {
    fn build(&self, app: &mut App) {
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        let mut settings = MeshAllocatorSettings::default();
        settings.large_threshold = 0;
        render.insert_resource(settings);
        render.add_systems(Render, no_metadata_without_compression.in_set(RenderSystems::PrepareAssets).before(allocate_and_free_meshes));
        // after `AssetEventSystems` (PostUpdate), before the extraction
        app.add_systems(Last, even_u16_indices);
    }

    // after every plugin has added its systems
    fn cleanup(&self, app: &mut App) {
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        let removed = render.world_mut().try_schedule_scope(Render, |world, schedule| {
            schedule.remove_systems_in_set(prepare_clusters_for_cpu_clustering, world, ScheduleCleanupPolicy::RemoveSystemsOnly)
        });
        match removed {
            Ok(Ok(n)) if n > 0 => {
                render.add_systems(
                    Render,
                    prepare_clusters_for_cpu_clustering
                        .in_set(RenderSystems::PrepareResources)
                        .run_if(|settings: Res<GlobalClusterSettings>| settings.gpu_clustering.is_none())
                        .run_if(clusters_wanted),
                );
            }
            other => warn!("web_gpu: Bevy's light clusters left as they are ({other:?})"),
        }
    }
}

/// A camera has no cluster buffers yet, or something is clustered.
fn clusters_wanted(
    bare: Query<(), (With<ExtractedClusterableObjects>, Without<ViewClusterBindings>)>,
    meta: Res<GlobalClusterableObjectMeta>,
) -> bool {
    !bare.is_empty() || !meta.entity_to_index.is_empty()
}

/// The meshes extracted this frame drop the bounds that only decompression
/// reads, unless their vertices are compressed: no metadata buffer of their own.
fn no_metadata_without_compression(mut extracted: ResMut<ExtractedAssets<RenderMesh>>) {
    for (_, mesh) in &mut extracted.extracted {
        // compressed positions are Snorm16x4; uv ranges are set only by compressing them
        let plain = matches!(mesh.try_attribute(Mesh::ATTRIBUTE_POSITION), Ok(VertexAttributeValues::Float32x3(_)));
        if plain && mesh.final_uv_ranges.iter().all(Option::is_none) {
            mesh.final_aabb = None;
        }
    }
}

/// A mesh with an odd number of 16-bit indices gets one more, which draws
/// nothing: a triangle of the first vertex three times in a triangle list,
/// the last index again in a strip, a line list or among points. Untracked:
/// the mesh is read by the extraction later this frame, as it is.
fn even_u16_indices(mut events: MessageReader<AssetEvent<Mesh>>, mut meshes: ResMut<Assets<Mesh>>) {
    for e in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::Modified { id }) = e else { continue };
        let Some(mesh) = meshes.get_mut_untracked(*id) else { continue };
        let list = mesh.primitive_topology() == PrimitiveTopology::TriangleList;
        if let Ok(Indices::U16(i)) = mesh.try_indices_mut() {
            if i.len() % 2 == 1 {
                match (list, i.last().copied()) {
                    (true, _) => i.extend_from_slice(&[0, 0, 0]),
                    (false, Some(last)) => i.push(last),
                    (false, None) => {}
                }
            }
        }
    }
}
