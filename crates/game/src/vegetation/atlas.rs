//! The foliage atlas every plant is painted from
//! (`scripts/build_vegetation_models.py`): leaf twigs with their alpha, the
//! canopy tiles of the distant crowns, strips of bark and a flat grey.

use bevy::prelude::*;

/// Mip levels of the atlas: down to 4 px for its 64 px canopy tiles, before
/// neighbouring tiles bleed into each other.
const ATLAS_MIPS: u32 = 5;

/// The foliage atlas, sRGB RGBA8, with its mips (a PNG has none), each level
/// keeping the share of each tile's texels that are leaf, or the leaf cards
/// would thin out to nothing with distance (alpha averages down).
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    /// Every level, the largest first.
    pub data: Vec<u8>,
}

/// RGBA8 sRGB pixels and size of a PNG.
fn decode(png: &[u8]) -> (Vec<u8>, u32, u32) {
    use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
    use bevy::asset::RenderAssetUsages;
    let mut image = Image::from_buffer(png, ImageType::Extension("png"), CompressedImageFormats::NONE, true, ImageSampler::Default, RenderAssetUsages::RENDER_WORLD)
        .expect("a valid foliage atlas");
    let (w, h) = (image.width(), image.height());
    (image.data.take().expect("atlas pixels"), w, h)
}

/// The authored foliage atlas alone, with its mips (the crowns' colours are
/// read from it, `canopy`).
pub fn atlas() -> Atlas {
    let (base, w, h) = decode(&crate::models::atlas_png());
    let (data, levels) = mip_chain(base, w as usize, h as usize, ATLAS_MIPS);
    Atlas { width: w, height: h, levels, data }
}

/// Where the authored atlas and the scanned plants' cards
/// (`models::scans`) sit in the atlas the plants are drawn with: the
/// authored one at the top left, the cards below it. Without the scans
/// the authored atlas alone, uvs unchanged.
#[derive(Clone, Copy)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    authored: [u32; 2],
    scans_y: u32,
    scans: [u32; 2],
}

/// Width and height of a PNG, from its header.
fn png_size(png: &[u8]) -> [u32; 2] {
    let be = |o: usize| u32::from_be_bytes(png[o..o + 4].try_into().unwrap());
    [be(16), be(20)]
}

pub fn layout() -> Layout {
    static LAYOUT: std::sync::OnceLock<Layout> = std::sync::OnceLock::new();
    *LAYOUT.get_or_init(|| {
        let authored = png_size(&crate::models::atlas_png());
        match crate::models::scans() {
            Some(s) => {
                let scans = [s.index.atlas.width, s.index.atlas.height];
                Layout { width: authored[0].max(scans[0]), height: authored[1] + scans[1], authored, scans_y: authored[1], scans }
            }
            None => Layout { width: authored[0], height: authored[1], authored, scans_y: authored[1], scans: [0, 0] },
        }
    })
}

impl Layout {
    /// A uv of the authored atlas in the plants' atlas.
    pub fn authored_uv(&self, uv: [f32; 2]) -> [f32; 2] {
        [uv[0] * self.authored[0] as f32 / self.width as f32, uv[1] * self.authored[1] as f32 / self.height as f32]
    }

    /// A uv of the scans' card atlas in the plants' atlas.
    pub fn scan_uv(&self, uv: [f32; 2]) -> [f32; 2] {
        [uv[0] * self.scans[0] as f32 / self.width as f32, (self.scans_y as f32 + uv[1] * self.scans[1] as f32) / self.height as f32]
    }
}

/// The atlas the plants are drawn with ([`Layout`]), with its mips.
pub fn plant_atlas() -> Atlas {
    let l = layout();
    let Some(scans) = crate::models::scans() else { return atlas() };
    let (authored, aw, ah) = decode(&crate::models::atlas_png());
    let (cards, cw, ch) = decode(&scans.atlas_png);
    let (w, h) = (l.width as usize, l.height as usize);
    let mut base = vec![0u8; w * h * 4];
    for (src, sw, sh, y0) in [(&authored, aw as usize, ah as usize, 0usize), (&cards, cw as usize, ch as usize, l.scans_y as usize)] {
        for y in 0..sh {
            base[((y0 + y) * w) * 4..((y0 + y) * w + sw) * 4].copy_from_slice(&src[y * sw * 4..(y + 1) * sw * 4]);
        }
    }
    let (data, levels) = mip_chain(base, w, h, ATLAS_MIPS);
    Atlas { width: l.width, height: l.height, levels, data }
}

/// `base` (RGBA8 sRGB, `w` x `h`) and its mips, concatenated. Colour is
/// averaged in linear light, weighted by alpha; each level's alpha is raised
/// per 64 px tile of the base so the share of texels at or above the cut
/// (0.5, `ALPHA_CUT` in `plant_common.wesl`) stays what it was.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mip level keeps the canopy tiles opaque and the leaf tiles'
    /// share of leaf texels near the base's.
    #[test]
    fn the_atlas_mips_keep_their_coverage() {
        let image = atlas();
        let (w, h) = (image.width as usize, image.height as usize);
        let data = &image.data;
        assert_eq!(image.levels, ATLAS_MIPS);
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
