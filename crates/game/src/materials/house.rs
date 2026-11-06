//! The houses of the towns in the realistic style (`models::scans`,
//! `KIOSK_SCANS=1`; 2026-11-06, third pass): the building chunks of
//! `buildings` (houses and the town kit drawn into them), in photographed
//! plaster, clay and slate roofs, rubble stone and brick (Poly Haven, CC0,
//! `assets/buildings/`), with the weather on them: damp rising from the
//! foot, dirt run down from the eaves, plaster flaked off to the brick.
//!
//! A material of its own, beside [`super::masonry`] (the landmarks', the
//! rocca's and the rocks'), so the two can change apart. It reads the same
//! vertex conventions: the colour is the tint (so the fire's recolouring in
//! `buildings::damage` reads through), the alpha what the vertex is made of
//! (`surface_alpha`: the structures' 1–11, `masonry::Part` 12–17 and
//! [`HousePart`] 20–24). The first uv is per house: `x` the height of the
//! ground at its foot, `y` a number of its own in 0..1 (how worn it is, which
//! plaster and which clay), `buildings::Builder::house_span`.
//!
//! Without the scans nothing here is created.

use bevy::asset::{uuid_handle, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use bevy::shader::ShaderRef;

const SHADER_HANDLE: Handle<Shader> = uuid_handle!("0f6b2a1e-7c34-4d58-b9e2-3a5c7d9e1f24");

/// What a vertex of a house is, beyond `masonry::Part` (in the colour's
/// alpha, `surface_alpha`).
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum HousePart {
    /// The walls' lime plaster, weathered: damp at the foot, flaked to the
    /// brick in patches (how much: the house's own number).
    Plaster = 20,
    /// Slate roof (the Genoese «ciappe»), tinted by the vertex colour.
    Slate = 21,
    /// Rubble stone left bare: a rustic house's walls.
    Rustic = 22,
    /// Paint on the plaster (trompe-l'œil frames, quoins and bands, painted
    /// shutters): smooth, no flaking.
    Paint = 23,
    /// Inside a window's or a door's opening: plaster in the shade, no
    /// weather.
    Reveal = 24,
}

/// The photographs, as layers of two array textures, in this order (the
/// shader's `L_*`).
const LAYERS: [&str; 8] = ["plaster", "roof", "stone", "slate", "rustic", "brick", "plaster2", "roof2"];

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct HouseFinish {
    /// sRGB colour (ambient occlusion baked in), roughness in alpha.
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub color: Handle<Image>,
    /// OpenGL tangent-space normals.
    #[texture(102, dimension = "2d_array")]
    pub normal: Handle<Image>,
}

pub type HouseMaterial = ExtendedMaterial<StandardMaterial, HouseFinish>;

impl MaterialExtension for HouseFinish {
    fn fragment_shader() -> ShaderRef {
        SHADER_HANDLE.into()
    }
}

pub(super) struct HousePlugin;

impl Plugin for HousePlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, SHADER_HANDLE, "../shaders/house.wesl");
        app.add_plugins(MaterialPlugin::<HouseMaterial>::default());
    }
}

/// The house material over `base`, if the realistic look is on and the
/// photographs load; `None` otherwise (the buildings keep their usual look).
pub fn material(base: StandardMaterial, images: &mut Assets<Image>) -> Option<HouseMaterial> {
    crate::models::scans()?;
    let color = array(&LAYERS.map(|n| format!("{n}_color.png")), true)?;
    let normal = array(&LAYERS.map(|n| format!("{n}_normal.png")), false)?;
    Some(HouseMaterial { base, extension: HouseFinish { color: images.add(color), normal: images.add(normal) } })
}

/// A file of `assets/buildings/`: of the source tree on the desktop, of the
/// page's data in the browser (the WebGPU build with `?scans=1`, at half the
/// side: every size is read from the files).
fn file(name: &str) -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/buildings").join(name));
    #[cfg(target_arch = "wasm32")]
    let bytes = datafs::take(format!("assets/buildings/{name}"));
    bytes.map_err(|e| warn!("houses: assets/buildings/{name}: {e}")).ok()
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
            .map_err(|e| warn!("houses: {f}: {e}"))
            .ok()?;
        let want = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
        if img.texture_descriptor.format != want {
            warn!("houses: {f} is {:?}, not {want:?}", img.texture_descriptor.format);
            return None;
        }
        layers.push(img);
    }
    let (w, h) = (layers[0].width() as usize, layers[0].height() as usize);
    if layers.iter().any(|l| l.width() as usize != w || l.height() as usize != h) {
        warn!("houses: the textures are not all {w} x {h}");
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
