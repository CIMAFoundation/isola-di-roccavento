//! Procedural textures for the fire, and the realistic ground's photo layers
//! ([`ground_detail`]).
//!
//! Flat quads are what make particle fire look like cardboard: the silhouette
//! is a rectangle and the edges are hard lines. Everything here exists to give
//! the fire soft, irregular edges without shipping any image assets — the
//! textures are generated at startup from value noise, so the whole scenario
//! stays reproducible from code and data alone.
//!
//! All three are alpha masks; colour comes from the mesh's vertex colours, so
//! one texture serves flames of any temperature.

use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const SIZE: u32 = 64;

/// Value noise on a lattice, smoothed — enough structure to break up an edge,
/// cheap enough to generate a handful of 64² textures at startup.
struct Noise(u64);

impl Noise {
    fn at(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        // Smoothstep the interpolant so the lattice does not show as a grid.
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let c = |ix: f32, iy: f32| {
            let mut h = self.0
                ^ (ix as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (iy as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            h ^= h >> 29;
            h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            h ^= h >> 32;
            (h >> 40) as f32 / (1u32 << 24) as f32
        };
        let top = c(x0, y0) * (1.0 - sx) + c(x0 + 1.0, y0) * sx;
        let bot = c(x0, y0 + 1.0) * (1.0 - sx) + c(x0 + 1.0, y0 + 1.0) * sx;
        top * (1.0 - sy) + bot * sy
    }

    /// Two octaves is plenty at 64²; more just adds shimmer.
    fn fbm(&self, x: f32, y: f32) -> f32 {
        self.at(x, y) * 0.65 + self.at(x * 2.3, y * 2.3) * 0.35
    }
}

fn image(pixels: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// A soft irregular puff: smoke, and (scaled down) embers.
pub fn puff() -> Image {
    let noise = Noise(0x9C4D_2E77);
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        let v = y as f32 / (SIZE - 1) as f32 * 2.0 - 1.0;
        for x in 0..SIZE {
            let u = x as f32 / (SIZE - 1) as f32 * 2.0 - 1.0;
            let r = (u * u + v * v).sqrt();
            // Radial falloff, with the radius itself perturbed by noise so the
            // puff is a cloud rather than a disc.
            let lumpy = r * (0.75 + 0.5 * noise.fbm(u * 2.5 + 3.0, v * 2.5 + 7.0));
            let a = (1.0 - lumpy).clamp(0.0, 1.0).powf(1.8);
            let shade = (0.6 + 0.4 * noise.fbm(u * 4.0, v * 4.0)) * 255.0;
            pixels.extend_from_slice(&[shade as u8, shade as u8, shade as u8, (a * 255.0) as u8]);
        }
    }
    image(pixels)
}

/// A tight glowing dot for embers: same idea, much harder falloff.
pub fn spark() -> Image {
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        let v = y as f32 / (SIZE - 1) as f32 * 2.0 - 1.0;
        for x in 0..SIZE {
            let u = x as f32 / (SIZE - 1) as f32 * 2.0 - 1.0;
            let r = (u * u + v * v).sqrt();
            let a = (1.0 - r).clamp(0.0, 1.0).powf(3.5);
            pixels.extend_from_slice(&[
                255,
                (140.0 + 115.0 * a) as u8,
                (40.0 * a) as u8,
                (a * 255.0) as u8,
            ]);
        }
    }
    image(pixels)
}

/// Side of the flames' noise volume (`noise_volume`): tileable, so the
/// period is the side.
const VOLUME: u32 = 48;

/// Tileable 3D value noise on a lattice of `n` cells per period.
fn lattice3(seed: u64, n: u32, x: f32, y: f32, z: f32) -> f32 {
    let (x0, y0, z0) = (x.floor(), y.floor(), z.floor());
    let (fx, fy, fz) = (x - x0, y - y0, z - z0);
    let s = |f: f32| f * f * (3.0 - 2.0 * f);
    let (sx, sy, sz) = (s(fx), s(fy), s(fz));
    let c = |ix: f32, iy: f32, iz: f32| {
        let w = |v: f32| (v.rem_euclid(n as f32)) as u64;
        let mut h = seed ^ w(ix).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ w(iy).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ w(iz).wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        (h >> 40) as f32 / (1u32 << 24) as f32
    };
    let l = |iy: f32, iz: f32| c(x0, iy, iz) * (1.0 - sx) + c(x0 + 1.0, iy, iz) * sx;
    let p = |iz: f32| l(y0, iz) * (1.0 - sy) + l(y0 + 1.0, iz) * sy;
    p(z0) * (1.0 - sz) + p(z0 + 1.0) * sz
}

/// The flames' noise (`shaders/fire_volume.wesl`), a tileable 3D texture:
/// r three octaves of value noise (the body), g and b two octaves each
/// (the curl), a spare. One trilinear fetch replaces a dozen hashes per
/// step of the march.
pub fn noise_volume() -> Image {
    let n = VOLUME;
    let mut pixels = Vec::with_capacity((n * n * n * 4) as usize);
    let fbm = |seed: u64, octaves: u32, x: f32, y: f32, z: f32| {
        let (mut sum, mut amp, mut cells) = (0.0, 1.0, 4u32);
        let mut norm = 0.0;
        for o in 0..octaves {
            let k = cells as f32 / n as f32;
            sum += amp * lattice3(seed + o as u64 * 7919, cells, x * k, y * k, z * k);
            norm += amp;
            // the fine octaves add grain, not shape: weaker than the usual half
            amp *= 0.42;
            cells *= 2;
        }
        sum / norm
    };
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let (fx, fy, fz) = (x as f32, y as f32, z as f32);
                let r = fbm(0x51A3_7C11, 3, fx, fy, fz);
                let g = fbm(0x9C4D_2E77, 2, fx, fy, fz);
                let b = fbm(0x3F0A_B55D, 2, fx, fy, fz);
                pixels.extend_from_slice(&[(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255]);
            }
        }
    }
    // Mips, box-averaged and wrapping like the noise (2026-11-05): read
    // from afar or with long strides, a billow finer than the stride must
    // be averaged; sampled at full detail it sparkles as the camera moves
    // (the smoke's «vibrare»). 48, 24, 12, 6, 3.
    let mut data = pixels.clone();
    let (mut level, mut size, mut levels) = (pixels, n, 1u32);
    while size > 3 {
        let half = size / 2;
        let mut next = Vec::with_capacity((half * half * half * 4) as usize);
        for z in 0..half {
            for y in 0..half {
                for x in 0..half {
                    let mut sum = [0u32; 4];
                    for (dx, dy, dz) in (0..8).map(|i| (i & 1, (i >> 1) & 1, (i >> 2) & 1)) {
                        let (sx, sy, sz) = ((2 * x + dx) % size, (2 * y + dy) % size, (2 * z + dz) % size);
                        let o = (((sz * size + sy) * size + sx) * 4) as usize;
                        for c in 0..4 {
                            sum[c] += level[o + c] as u32;
                        }
                    }
                    next.extend(sum.map(|v| ((v + 4) / 8) as u8));
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        size = half;
        levels += 1;
    }
    let mut image = Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: n },
        TextureDimension::D3,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        address_mode_w: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The terrain's heights for the shaders that stand on the ground
/// (`shaders/fire_volume.wesl`): `Rg8Unorm`, the height above the lowest
/// sample in 16 bits (r high, g low) over `scale` metres per code; returned
/// with that offset and scale. Row 0 at the north edge, as `Terrain::elev`.
pub fn heights(t: &scenario::Terrain) -> (Image, f32, f32) {
    // the data's own range: the graded roads may dip under the DEM's
    let lo = t.elev.iter().copied().fold(f32::INFINITY, f32::min).min(0.0);
    let hi = t.elev.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let scale = (hi - lo).max(1.0) / 65535.0;
    let mut pixels = Vec::with_capacity(t.elev.len() * 2);
    for h in &t.elev {
        let code = (((h - lo) / scale).round() as i64).clamp(0, 65535) as u16;
        pixels.extend_from_slice(&[(code >> 8) as u8, (code & 0xFF) as u8]);
    }
    let image = Image::new(
        Extent3d { width: t.cols as u32, height: t.rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rg8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    (image, lo, scale)
}

/// The realistic ground's layers (2026-11-06, with the scanned plants,
/// `KIOSK_SCANS=1`), in the order of the array and of `terrain.wesl`'s
/// `L_*` constants. Photos from Poly Haven, CC0 (`assets/ground/CREDITS.txt`),
/// high-passed into a ratio around one by `tools/terrain/ground_detail.py`.
pub const GROUND_LAYERS: [&str; 10] = ["grass", "soil", "needles", "leaves", "rock", "sand", "track", "burnt", "asphalt", "paving"];

/// The ground's detail layers and their mean colours (linear), read once.
pub struct GroundDetail {
    /// Every layer's pixels, `side`², RGBA8 (linear), with their mips, layer
    /// after layer.
    data: Vec<u8>,
    side: u32,
    levels: u32,
    /// Mean colour of each layer's photo, linear, in `GROUND_LAYERS`' order.
    pub means: [[f32; 3]; GROUND_LAYERS.len()],
    /// The near grass's tuft cards (`grass_cards.png`), RGBA8 with mips.
    cards: Vec<u8>,
    cards_size: (u32, u32),
    cards_levels: u32,
}

/// The realistic ground, when the scanned plants are on (the same switch:
/// `models::scans`, `?scans=1` in the WebGPU build) and its files are there.
pub fn ground_detail() -> Option<&'static GroundDetail> {
    static DETAIL: std::sync::OnceLock<Option<GroundDetail>> = std::sync::OnceLock::new();
    DETAIL
        .get_or_init(|| {
            crate::models::scans()?;
            load_ground_detail().map_err(|e| warn!("realistic ground: {e}")).ok()
        })
        .as_ref()
}

/// A file of `assets/ground`: of the source tree on the desktop, of the
/// page's data in the browser (where the layers come at half their side:
/// every size is read from the files).
fn ground_file(name: &str) -> Result<Vec<u8>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ground").join(name));
    #[cfg(target_arch = "wasm32")]
    let bytes = datafs::take(format!("assets/ground/{name}"));
    bytes.map_err(|e| format!("{name}: {e}"))
}

/// A PNG of `assets/ground` as RGBA8 pixels (linear or sRGB as stored) and
/// its side.
fn ground_png(name: &str) -> Result<(Vec<u8>, u32, u32), String> {
    use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
    let png = ground_file(name)?;
    let image = Image::from_buffer(&png, ImageType::Extension("png"), CompressedImageFormats::NONE, false, ImageSampler::Default, RenderAssetUsages::RENDER_WORLD).map_err(|e| format!("{name}: {e}"))?;
    let image = match image.texture_descriptor.format {
        TextureFormat::Rgba8Unorm => image,
        f => image.convert(TextureFormat::Rgba8Unorm).ok_or(format!("{name}: {f:?}, not convertible to RGBA8"))?,
    };
    let (w, h) = (image.width(), image.height());
    Ok((image.data.ok_or(format!("{name}: no pixels"))?, w, h))
}

/// Box-filtered mips of a `w` x `h` RGBA8 image (wrapping: the photos
/// tile), appended after it; returns the levels.
fn push_mips(data: &mut Vec<u8>, base: Vec<u8>, w: usize, h: usize) -> u32 {
    let (mut level, mut lw, mut lh, mut count) = (base, w, h, 1u32);
    loop {
        data.extend_from_slice(&level);
        if lw == 1 && lh == 1 {
            return count;
        }
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = Vec::with_capacity(nw * nh * 4);
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let at = |xx: usize, yy: usize| level[((yy % lh) * lw + (xx % lw)) * 4 + c] as u32;
                    let (x0, y0) = (2 * x, 2 * y);
                    let s = at(x0, y0) + at(x0 + 1, y0) + at(x0, y0 + 1) + at(x0 + 1, y0 + 1);
                    next.push(((s + 2) / 4) as u8);
                }
            }
        }
        (level, lw, lh) = (next, nw, nh);
        count += 1;
    }
}

fn load_ground_detail() -> Result<GroundDetail, String> {
    let index: serde_json::Value = serde_json::from_slice(&ground_file("ground.json")?).map_err(|e| e.to_string())?;
    let layers = index["layers"].as_array().ok_or("ground.json: no layers")?;
    let mut means = [[0.5f32; 3]; GROUND_LAYERS.len()];
    let (mut data, mut side, mut levels) = (Vec::new(), 0u32, 0u32);
    for (k, name) in GROUND_LAYERS.iter().enumerate() {
        let entry = layers.iter().find(|l| l["name"] == *name).ok_or(format!("ground.json: no layer {name}"))?;
        for (c, v) in entry["mean"].as_array().ok_or("mean")?.iter().enumerate().take(3) {
            means[k][c] = v.as_f64().unwrap_or(0.5) as f32;
        }
        let (base, w, h) = ground_png(&format!("{name}.png"))?;
        if w != h || !w.is_power_of_two() || (side != 0 && w != side) {
            return Err(format!("{name}.png: {w} x {h}, the layers must be square, of one power-of-two side"));
        }
        side = w;
        // the mips of a ratio around one fade to one, the macro colour
        levels = push_mips(&mut data, base, w as usize, h as usize);
    }
    // the near grass's cards (`near_cover`), with mips whose alpha keeps
    // the share of texels a cut-out at one half draws, so far tufts do not
    // thin out into nothing
    let (cards_px, cw, ch) = ground_png("grass_cards.png")?;
    let mut cards = Vec::new();
    let cards_levels = push_mips(&mut cards, cards_px, cw as usize, ch as usize);
    keep_alpha_coverage(&mut cards, cw as usize, ch as usize, cards_levels);
    info!("realistic ground: {} layers of {side}² and grass cards of {cw} x {ch} from assets/ground", GROUND_LAYERS.len());
    Ok(GroundDetail { data, side, levels, means, cards, cards_size: (cw, ch), cards_levels })
}

/// Scales each mip's alpha so that as many texels pass 0.5 as in the full
/// level (the mips of a cut-out otherwise lose their thin blades).
fn keep_alpha_coverage(data: &mut [u8], w: usize, h: usize, levels: u32) {
    let share = |px: &[u8], k: f32| px.chunks(4).filter(|p| p[3] as f32 * k >= 127.5).count() as f32 / (px.len() / 4) as f32;
    let target = share(&data[..w * h * 4], 1.0);
    let (mut off, mut lw, mut lh) = (w * h * 4, (w / 2).max(1), (h / 2).max(1));
    for _ in 1..levels {
        let len = lw * lh * 4;
        let px = &mut data[off..off + len];
        // bisection on the factor
        let (mut lo, mut hi) = (0.5f32, 8.0f32);
        for _ in 0..16 {
            let mid = 0.5 * (lo + hi);
            if share(px, mid) < target { lo = mid } else { hi = mid }
        }
        for p in px.chunks_mut(4) {
            p[3] = (p[3] as f32 * hi).min(255.0) as u8;
        }
        off += len;
        (lw, lh) = ((lw / 2).max(1), (lh / 2).max(1));
    }
}

/// The near grass's tuft cards (`near_cover`), sRGB with alpha and mips; a
/// 1 x 1 stand-in that no shader reads when the realistic ground is off.
pub fn grass_cards_image() -> Image {
    use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    let (data, (w, h), levels) = match ground_detail() {
        Some(d) => (d.cards.clone(), d.cards_size, d.cards_levels),
        None => (vec![0u8; 4], (1, 1), 1),
    };
    let mut image = Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, Vec::new(), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The ground's detail as a 2D array texture for the terrain
/// (`materials::TerrainMaterial`): the layers when the realistic ground is
/// on, else a 1 x 1 stand-in of two layers that no shader reads.
pub fn ground_detail_image() -> Image {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
    let (data, side, layers, levels) = match ground_detail() {
        Some(d) => (d.data.clone(), d.side, GROUND_LAYERS.len() as u32, d.levels),
        None => (vec![102u8; 8], 1, 2, 1),
    };
    let mut image = Image::new(Extent3d { width: side, height: side, depth_or_array_layers: layers }, TextureDimension::D2, Vec::new(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        // the ground is seen at a grazing angle: without it the far photos
        // blur into the macro colour too soon
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

/// One layer of the realistic ground as a texture of its own, with its mips,
/// repeating: for the meshes laid over the ground (`fields.rs`). The ratio is
/// stored * [`GROUND_ENCODE`]: a material multiplies it back.
pub fn ground_layer_image(name: &str) -> Option<Image> {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    let d = ground_detail()?;
    let k = GROUND_LAYERS.iter().position(|l| *l == name)?;
    let per_layer = d.data.len() / GROUND_LAYERS.len();
    let mut image = Image::new(Extent3d { width: d.side, height: d.side, depth_or_array_layers: 1 }, TextureDimension::D2, Vec::new(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(d.data[k * per_layer..(k + 1) * per_layer].to_vec());
    image.texture_descriptor.mip_level_count = d.levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    Some(image)
}

/// How the layers store the ratio (`tools/terrain/ground_detail.py`).
pub const GROUND_ENCODE: f32 = 0.33;

/// The flame flipbook (2026-11-05, `tools/fire/flipbook.py`): two filmed
/// -looking flame sequences of 64 frames, CC0 from Unity Labs, one sheet of
/// 16 x 8 frames. Red is how hot, alpha how much flame; the shader colours
/// it (`flame.wesl`). Linear, with box-filtered mips: a tongue far off is a
/// few pixels of a 128 x 256 frame, and without them it shimmers.
///
/// `skip`: without its first levels. The sheet is 2048 a side, 21 MB with
/// its mips; where the volumes draw the front's flames only the burning
/// houses play it, small on the screen (`fire_view::setup`).
pub fn flame_flipbook(skip: u32) -> Image {
    use bevy::image::{CompressedImageFormats, ImageFilterMode, ImageSampler, ImageSamplerDescriptor, ImageType};
    #[cfg(not(target_arch = "wasm32"))]
    let png: std::borrow::Cow<'static, [u8]> = include_bytes!("../../../assets/models/flame_flipbook.png").as_slice().into();
    #[cfg(target_arch = "wasm32")]
    let png: std::borrow::Cow<'static, [u8]> = datafs::read("assets/models/flame_flipbook.png")
        .expect("assets/models/flame_flipbook.png is not in the page's data")
        .into();
    let image = Image::from_buffer(&png, ImageType::Extension("png"), CompressedImageFormats::NONE, false, ImageSampler::Default, RenderAssetUsages::RENDER_WORLD)
        .expect("a valid flame flipbook");
    let image = image.convert(TextureFormat::Rgba8Unorm).unwrap_or(image);
    let (w, h) = (image.width() as usize, image.height() as usize);
    let mut data = image.data.clone().expect("flipbook pixels");
    // mips down to 8 x 16 a frame: past that, frames bleed into each other
    let (mut lw, mut lh, mut offset, mut levels) = (w, h, 0usize, 1u32);
    while levels < 5 {
        let (nw, nh) = (lw / 2, lh / 2);
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let at = |xx: usize, yy: usize| data[offset + (yy * lw + xx) * 4 + c] as u32;
                    let s = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                    data.push(((s + 2) / 4) as u8);
                }
            }
        }
        offset += lw * lh * 4;
        (lw, lh) = (nw, nh);
        levels += 1;
    }
    // the levels skipped: their bytes come first
    let skip = skip.min(levels - 1);
    let (mut w, mut h) = (w, h);
    let mut first = 0;
    for _ in 0..skip {
        first += w * h * 4;
        (w, h) = (w / 2, h / 2);
    }
    data.drain(..first);
    let mut image = Image::new(Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, TextureDimension::D2, Vec::new(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels - skip;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

