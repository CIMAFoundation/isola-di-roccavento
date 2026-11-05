//! Procedural textures for the fire.
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

/// The flame flipbook (2026-11-05, `tools/fire/flipbook.py`): two filmed
/// -looking flame sequences of 64 frames, CC0 from Unity Labs, one sheet of
/// 16 x 8 frames. Red is how hot, alpha how much flame; the shader colours
/// it (`flame.wesl`). Linear, with box-filtered mips: a tongue far off is a
/// few pixels of a 128 x 256 frame, and without them it shimmers.
pub fn flame_flipbook() -> Image {
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
    let mut image = Image::new(Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, TextureDimension::D2, Vec::new(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
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

