//! Photographed masonry for the buildings, when the plants are photogrammetry
//! scans (`models::scans`, `KIOSK_SCANS=1`; 2026-11-06). Lime plaster, clay
//! roof tiles and rubble stone from Poly Haven (CC0, `assets/buildings/`):
//! with scanned plants, the flat colours and procedural patterns of
//! [`super::StructureMaterial`] read as a cartoon next to them.
//!
//! A material of its own, not a finish of `StructureMaterial`: its two
//! texture arrays would add to the bind group of every structure, also where
//! the scans are off (and on WebGL2, where texture units are few). Without
//! the scans nothing here is created, and the buildings are drawn exactly as
//! before.
//!
//! What a vertex is made of comes from the vertex colour's alpha, as for the
//! structures (`surface_alpha`), with the ids of [`Part`]. The vertex colour
//! stays the tint, so the fire's recolouring (`buildings::damage`) works
//! unchanged: a burnt house is its plaster and tiles gone black.

use bevy::asset::{uuid_handle, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    TextureViewDescriptor, TextureViewDimension,
};
use bevy::shader::ShaderRef;

const SHADER_HANDLE: Handle<Shader> = uuid_handle!("5be0c8a4-1d3f-4e62-9a7b-0c2f4d6e8a13");

/// What a vertex of a building is, in its colour's alpha (`surface_alpha`).
/// Above the structures' ids (1–11, which keep their meaning: 4 glass,
/// 11 louvred shutters).
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Part {
    /// Lime plaster over the vertex colour.
    Plaster = 12,
    /// Clay roof tiles, tinted a little by the vertex colour.
    Tiles = 13,
    /// Rubble stone: plinths, towers, the rocca.
    Stone = 14,
    /// A kit model (church, town hall): plaster on the walls, tiles on the
    /// slopes, as the face's normal says.
    Building = 15,
    /// Window and door frames, painted wood.
    Frame = 16,
    /// Bare limestone (the boulders at the tower's foot).
    Rock = 17,
}

/// The structures' surface ids the masonry shader also draws (`apply_surface`
/// in `structure.wesl`): for the kit's props and windows.
pub mod surface {
    pub const PAINT: u8 = 1;
    pub const RUBBER: u8 = 3;
    pub const GLASS: u8 = 4;
    pub const WOOD: u8 = 10;
}

/// Which texture a material leans on where a vertex does not say: the
/// town (props keep their flat colour), a landmark (all stone), bare rock
/// (the limestone the terrain's cliffs are drawn with, Poly Haven «Marble
/// Cliff 05»): the coast's boulders, or the crag under the rocca, whose bake
/// is darker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Masonry {
    #[default]
    Town,
    Landmark,
    Rock,
    Crag,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
#[bind_group_data(Masonry)]
pub struct MasonryFinish {
    /// Layers plaster, tiles, stone, rock: sRGB colour (ambient occlusion baked
    /// in), roughness in alpha.
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub color: Handle<Image>,
    /// The same layers' OpenGL tangent-space normals.
    #[texture(102, dimension = "2d_array")]
    pub normal: Handle<Image>,
    pub masonry: Masonry,
}

impl From<&MasonryFinish> for Masonry {
    fn from(f: &MasonryFinish) -> Self {
        f.masonry
    }
}

pub type MasonryMaterial = ExtendedMaterial<StandardMaterial, MasonryFinish>;

impl MaterialExtension for MasonryFinish {
    fn fragment_shader() -> ShaderRef {
        SHADER_HANDLE.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            match key.bind_group_data {
                Masonry::Town => {}
                Masonry::Landmark => fragment.shader_defs.push("MASONRY_LANDMARK".into()),
                Masonry::Rock => fragment.shader_defs.push("MASONRY_ROCK".into()),
                Masonry::Crag => {
                    fragment.shader_defs.push("MASONRY_ROCK".into());
                    fragment.shader_defs.push("MASONRY_CRAG".into());
                }
            }
        }
        Ok(())
    }
}

pub(super) struct MasonryPlugin;

impl Plugin for MasonryPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, SHADER_HANDLE, "../shaders/masonry.wesl");
        app.add_plugins(MaterialPlugin::<MasonryMaterial>::default());
    }
}

/// The photographed textures, made once and shared by the buildings and the
/// landmarks.
#[derive(Clone)]
pub struct MasonryTextures {
    color: Handle<Image>,
    normal: Handle<Image>,
}

/// The textures, if the realistic look is on (`models::scans`) and the files
/// load; `None` otherwise, and the buildings keep their usual look.
pub fn textures(images: &mut Assets<Image>) -> Option<MasonryTextures> {
    static MADE: std::sync::Mutex<Option<Option<MasonryTextures>>> = std::sync::Mutex::new(None);
    crate::models::scans()?;
    let mut made = MADE.lock().ok()?;
    if let Some(t) = made.as_ref() {
        return t.clone();
    }
    let names = ["plaster", "roof", "stone", "rock"];
    let t = (|| {
        let color = array(&names.map(|n| format!("{n}_color.png")), true)?;
        let normal = array(&names.map(|n| format!("{n}_normal.png")), false)?;
        Some(MasonryTextures { color: images.add(color), normal: images.add(normal) })
    })();
    *made = Some(t.clone());
    t
}

/// A masonry material over `base` (the vertex colours do the rest).
pub fn material(base: StandardMaterial, t: &MasonryTextures, masonry: Masonry) -> MasonryMaterial {
    MasonryMaterial {
        base,
        extension: MasonryFinish { color: t.color.clone(), normal: t.normal.clone(), masonry },
    }
}

/// A file of `assets/buildings/`: of the source tree on the desktop, of the
/// page's data in the browser (the WebGPU build with `?scans=1`, at half the
/// side: every size is read from the files).
fn file(name: &str) -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/buildings").join(name));
    #[cfg(target_arch = "wasm32")]
    let bytes = datafs::take(format!("assets/buildings/{name}"));
    bytes.map_err(|e| warn!("masonry: assets/buildings/{name}: {e}")).ok()
}

/// Normals averaged across the facets of a rock (corners sharper than
/// `crease_deg` stay hard): a baked boulder is a few dozen flat faces, and
/// photographed rock on flat-lit facets still reads as a cut gem. Only with
/// the photographed masonry; the mesh keeps its vertices, so whatever reads
/// it back (Mario's collisions) sees the same shape.
/// `only`: the vertices to smooth (all if `None`).
pub fn smooth_normals(mesh: &mut Mesh, crease_deg: f32, only: Option<&[bool]>) {
    use bevy::mesh::VertexAttributeValues;
    use std::collections::HashMap;
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return };
    let Some(VertexAttributeValues::Float32x3(nor)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { return };
    let key = |p: &[f32; 3]| [(p[0] * 50.0).round() as i64, (p[1] * 50.0).round() as i64, (p[2] * 50.0).round() as i64];
    let mut at: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (i, p) in pos.iter().enumerate() {
        if only.map_or(true, |o| o[i]) {
            at.entry(key(p)).or_default().push(i);
        }
    }
    let cos = crease_deg.to_radians().cos();
    let mut out = nor.clone();
    for group in at.values() {
        for &i in group {
            let ni = Vec3::from(nor[i]);
            let sum: Vec3 = group.iter().map(|&j| Vec3::from(nor[j])).filter(|nj| nj.dot(ni) >= cos).sum();
            out[i] = sum.try_normalize().unwrap_or(ni).to_array();
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, out);
}

/// PNGs of one size as the layers of an array texture, with box-filtered
/// mips (a wall a kilometre off is a few texels, and without them it
/// shimmers).
fn array(files: &[String], srgb: bool) -> Option<Image> {
    use bevy::image::{CompressedImageFormats, ImageType};
    let mut layers = Vec::new();
    for f in files {
        let bytes = file(f)?;
        let img = Image::from_buffer(&bytes, ImageType::Extension("png"), CompressedImageFormats::NONE, srgb, ImageSampler::Default, RenderAssetUsages::RENDER_WORLD)
            .map_err(|e| warn!("masonry: {f}: {e}"))
            .ok()?;
        // PNGs decode to 8-bit RGBA (RGB is widened), sRGB or not as asked
        let want = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
        if img.texture_descriptor.format != want {
            warn!("masonry: {f} is {:?}, not {want:?}", img.texture_descriptor.format);
            return None;
        }
        layers.push(img);
    }
    let (w, h) = (layers[0].width() as usize, layers[0].height() as usize);
    if layers.iter().any(|l| l.width() as usize != w || l.height() as usize != h) {
        warn!("masonry: the textures are not all {w} x {h}");
        return None;
    }
    let levels = (w.min(h) as f32).log2() as u32 + 1;
    // layer-major: every mip of layer 0, then of layer 1, ...
    let mut data = Vec::new();
    for l in &layers {
        let mut level = l.data.clone()?;
        let (mut lw, mut lh) = (w, h);
        data.extend_from_slice(&level);
        for _ in 1..levels {
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            let mut next = Vec::with_capacity(nw * nh * 4);
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..4 {
                        let at = |xx: usize, yy: usize| level[((yy.min(lh - 1)) * lw + xx.min(lw - 1)) * 4 + c] as u32;
                        let s = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                        next.push(((s + 2) / 4) as u8);
                    }
                }
            }
            data.extend_from_slice(&next);
            (level, lw, lh) = (next, nw, nh);
        }
    }
    let format = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    let size = Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: layers.len() as u32 };
    let mut image = Image::new(size, TextureDimension::D2, vec![0; w * h * 4 * layers.len()], format, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 4,
        ..default()
    });
    Some(image)
}
