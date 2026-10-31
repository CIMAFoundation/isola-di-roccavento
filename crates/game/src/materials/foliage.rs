//! The vegetation's material (`shaders/foliage.wgsl`) and its vertex stages,
//! which apply each plant's fire state from a small per-chunk image
//! (`shaders/veg_state.wgsl`, `vegetation.rs`), and the foliage atlas every
//! plant is painted from (`scripts/build_vegetation_models.py`).

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::shader::ShaderRef;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError, VertexFormat,
};

const SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f15");
const STATE_SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f11");
const VERTEX_SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f12");
const PREPASS_SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f13");

/// Which plant of the island a vertex belongs to, so the vertex stage finds
/// its texel in the state image: its global index, `-(index + 2)` in the
/// burnt-remains layer, -1 for none. A float, exact up to 2^24 plants: no
/// integer vertex attribute (suspected on the iPhone, 2026-10-31).
pub const ATTRIBUTE_PLANT: MeshVertexAttribute = MeshVertexAttribute::new("Plant", 0x91A7_4403, VertexFormat::Float32);
/// Vertex input location of [`ATTRIBUTE_PLANT`] (`shaders/veg_*.wgsl`).
const PLANT_LOCATION: u32 = 14;

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct FoliageMaterial {
    /// x: tactical desaturation (`tactical.rs`); y: the simulated clock, s;
    /// z: how long a plant flames, s (`vegetation::fire_clock`). One vec4:
    /// WebGL2 wants uniform bindings in multiples of 16 bytes.
    #[uniform(0)]
    pub fire: Vec4,
    /// One RGBA8 texel per plant of the island (`vegetation::Texel`: 24-bit
    /// ignition instant, 8 bits of flags), read with `textureLoad` in the
    /// vertex stage. The plants burn by writing a texel when one catches,
    /// never their meshes. One material, one image for every chunk.
    #[texture(1, sample_type = "float", filterable = false)]
    pub plant_state: Handle<Image>,
    /// The foliage atlas ([`foliage_atlas`]).
    #[texture(2)]
    #[sampler(3)]
    pub atlas: Handle<Image>,
}

impl FoliageMaterial {
    pub fn new(plant_state: Handle<Image>, atlas: Handle<Image>) -> Self {
        FoliageMaterial { fire: Vec4::ZERO, plant_state, atlas }
    }

    pub fn tactical(&self) -> f32 {
        self.fire.x
    }

    pub fn set_tactical(&mut self, amount: f32) {
        self.fire.x = amount;
    }
}

/// Mip levels of the atlas: down to 4 px for its 64 px canopy tiles, before
/// neighbouring tiles bleed into each other.
const ATLAS_MIPS: u32 = 5;

/// The foliage atlas as a texture: sRGB, mipmapped here (a PNG has no mips),
/// each level keeping the share of each tile's texels that are leaf, or the
/// leaf cards thin out to nothing with distance (alpha averages down).
pub fn foliage_atlas() -> Image {
    use bevy::image::{CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor, ImageType};
    use bevy::asset::RenderAssetUsages;
    let sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    let mut image = Image::from_buffer(
        &crate::models::atlas_png(),
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        sampler,
        RenderAssetUsages::RENDER_WORLD,
    )
    .expect("a valid foliage atlas");
    let (w, h) = (image.width() as usize, image.height() as usize);
    let base = image.data.take().expect("atlas pixels");
    let (data, levels) = mip_chain(base, w, h, ATLAS_MIPS);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image
}

/// `base` (RGBA8 sRGB, `w` x `h`) and its mips, concatenated. Colour is
/// averaged in linear light, weighted by alpha; each level's alpha is raised
/// per 64 px tile of the base so the share of texels at or above the cut
/// (0.5, `ALPHA_CUT` in `veg_state.wgsl`) stays what it was.
fn mip_chain(base: Vec<u8>, w: usize, h: usize, max_levels: u32) -> (Vec<u8>, u32) {
    const TILE: usize = 64;
    let to_lin: Vec<f32> = (0..256).map(|i| Color::srgb_u8(i as u8, 0, 0).to_linear().red).collect();
    let to_srgb = |v: f32| (Color::linear_rgb(v.clamp(0.0, 1.0), 0.0, 0.0).to_srgba().red * 255.0 + 0.5) as u8;
    let coverage = |px: &[u8], w: usize, x0: usize, y0: usize, n: usize, k: f32| {
        let mut hit = 0;
        for y in y0..y0 + n {
            for x in x0..x0 + n {
                if px[(y * w + x) * 4 + 3] as f32 * k >= 127.5 {
                    hit += 1;
                }
            }
        }
        hit as f32 / (n * n) as f32
    };
    let (tiles_x, tiles_y) = (w / TILE, h / TILE);
    let target: Vec<f32> = (0..tiles_x * tiles_y)
        .map(|t| coverage(&base, w, (t % tiles_x) * TILE, (t / tiles_x) * TILE, TILE, 1.0))
        .collect();
    let mut out = base.clone();
    let (mut prev, mut pw, mut ph) = (base, w, h);
    let mut levels = 1;
    while levels < max_levels && pw >= 2 && ph >= 2 {
        let (nw, nh) = (pw / 2, ph / 2);
        let mut next = vec![0u8; nw * nh * 4];
        let mut alpha = vec![0f32; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let (mut rgb, mut a) = ([0f32; 3], 0f32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let i = ((2 * y + dy) * pw + 2 * x + dx) * 4;
                    let al = prev[i + 3] as f32 / 255.0;
                    for c in 0..3 {
                        rgb[c] += to_lin[prev[i + c] as usize] * al.max(1e-3);
                    }
                    a += al.max(1e-3);
                }
                let o = (y * nw + x) * 4;
                for c in 0..3 {
                    next[o + c] = to_srgb(rgb[c] / a);
                }
                alpha[y * nw + x] = a / 4.0;
            }
        }
        // per tile: the alpha scale that keeps the base's coverage
        let n = TILE >> levels;
        for t in 0..tiles_x * tiles_y {
            let (x0, y0) = ((t % tiles_x) * n, (t / tiles_x) * n);
            let cov = |k: f32| {
                let mut hit = 0;
                for y in y0..y0 + n {
                    for x in x0..x0 + n {
                        if alpha[y * nw + x] * k >= 0.5 {
                            hit += 1;
                        }
                    }
                }
                hit as f32 / (n * n) as f32
            };
            let (mut lo, mut hi) = (0.25f32, 4.0f32);
            for _ in 0..12 {
                let mid = (lo + hi) * 0.5;
                if cov(mid) < target[t] { lo = mid } else { hi = mid }
            }
            // only ever raised: the smallest scale that reaches the coverage
            // would halve an opaque tile's alpha down to the cut
            let k = ((lo + hi) * 0.5).max(1.0);
            for y in y0..y0 + n {
                for x in x0..x0 + n {
                    next[(y * nw + x) * 4 + 3] = ((alpha[y * nw + x] * k).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        (pw, ph) = (nw, nh);
        levels += 1;
    }
    (out, levels)
}

impl Material for FoliageMaterial {
    /// Leaf cards are cut out of the atlas on the desktop. The browser draws
    /// only the distant proxies (opaque canopy tiles): opaque there, so its
    /// depth and shadow passes keep running without a fragment stage.
    fn alpha_mode(&self) -> AlphaMode {
        if cfg!(target_arch = "wasm32") { AlphaMode::Opaque } else { AlphaMode::Mask(0.5) }
    }

    fn vertex_shader() -> ShaderRef {
        VERTEX_SHADER_HANDLE.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_HANDLE.into()
    }

    /// Depth and shadow passes too: a plant that has collapsed casts nothing.
    fn prepass_vertex_shader() -> ShaderRef {
        PREPASS_SHADER_HANDLE.into()
    }

    /// The leaves' cut-out in the depth and shadow passes (`AlphaMode::Mask`).
    fn prepass_fragment_shader() -> ShaderRef {
        PREPASS_SHADER_HANDLE.into()
    }

    /// Foliage is thin: both faces drawn, in every pass, and light through the
    /// leaves (Bevy's diffuse transmission, set per pixel in `foliage.wgsl`).
    /// The plant index joins the mesh's one interleaved vertex buffer.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        // leaves let the sun through: lit from behind, a crown glows
        // (`?trasmissione=0` in the browser leaves it out, `?stato=0` the
        // plants' fire state, `?trama=0` the atlas (vertex colours only):
        // switches to find what kills an iPhone's page)
        if crate::quality::web_off("stato") {
            descriptor.vertex.shader_defs.push("NO_PLANT_STATE".into());
        }
        if let Some(fragment) = descriptor.fragment.as_mut() {
            if !crate::quality::web_off("trasmissione") {
                fragment.shader_defs.push("STANDARD_MATERIAL_DIFFUSE_TRANSMISSION".into());
            }
            if crate::quality::web_off("trama") {
                fragment.shader_defs.push("PLAIN_FOLIAGE".into());
            }
        }
        // the plant index: a vertex attribute in the merged chunk meshes, the
        // mesh tag of an instance (`vegetation::instanced`)
        if layout.0.contains(ATTRIBUTE_PLANT) {
            let plant = layout.0.get_layout(&[ATTRIBUTE_PLANT.at_shader_location(PLANT_LOCATION)])?;
            descriptor.vertex.buffers[0].attributes.extend(plant.attributes);
        } else {
            descriptor.vertex.shader_defs.push("PLANT_TAG".into());
        }
        Ok(())
    }
}

pub(super) struct FoliagePlugin;

impl Plugin for FoliagePlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, STATE_SHADER_HANDLE, "../shaders/veg_state.wesl");
        crate::load_wesl!(app, VERTEX_SHADER_HANDLE, "../shaders/veg_vertex.wesl");
        crate::load_wesl!(app, PREPASS_SHADER_HANDLE, "../shaders/veg_prepass.wesl");
        crate::load_wesl!(app, SHADER_HANDLE, "../shaders/foliage.wesl");
        app.add_plugins(MaterialPlugin::<FoliageMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mip level keeps the canopy tiles opaque and the leaf tiles'
    /// share of leaf texels near the base's.
    #[test]
    fn the_atlas_mips_keep_their_coverage() {
        let image = foliage_atlas();
        let (w, h) = (image.width() as usize, image.height() as usize);
        let data = image.data.as_ref().unwrap();
        assert_eq!(image.texture_descriptor.mip_level_count, ATLAS_MIPS);
        let grey = crate::models::atlas().grey;
        let (mut off, mut lw, mut lh) = (0, w, h);
        for level in 0..ATLAS_MIPS {
            let (x, y) = ((grey[0] * lw as f32) as usize, (grey[1] * lh as f32) as usize);
            let a = data[off + (y * lw + x) * 4 + 3];
            assert_eq!(a, 255, "level {level}: grey alpha {a}");
            off += lw * lh * 4;
            (lw, lh) = (lw / 2, lh / 2);
        }
        assert_eq!(off, data.len());
    }
}
