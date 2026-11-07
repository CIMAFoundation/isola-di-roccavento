// The terrain (`materials/terrain.rs`): ground colour and normal per terrain
// sample, the canopy painted on it, the fire's scar and glow (`fire_ground`),
// the road network and the meadow drawn from distance-field masks,
// the limestone's beds on the rock faces. A material of its own rather than an
// extension of StandardMaterial, whose textures it would never read: on WebGL2
// every texture of the layout takes one of the 16 units, read or not.
// The light is Bevy's PBR all the same (`apply_pbr_lighting`), fog included,
// with the sun's shadow faded out at the end of its reach (`faded_shadow`).

import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::mesh_bindings::mesh;
import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::{pbr_functions, pbr_types};
import game::shaders::noise::{hash21, value_noise};
import game::shaders::scenery::{limestone, rock_bed, rock_bump, tactical_grey};
import game::shaders::faded_shadow::apply_pbr_lighting_faded;
import game::shaders::fire_ground::fire_ground;

struct Terrain {
    // world metres to mask uv (xy); z > 0.5: meadow on; w: where the blades
    // end, metres from the eye (0 = no blades)
    mask_rect: vec4<f32>,
    // world metres to the ground textures' uv: x, y = 1 / (posting * size),
    // z = half a texel in u, w = the north edge plus half a texel in v
    ground_rect: vec4<f32>,
    // x: tactical desaturation, 0 (full colour) to 1 (`tactical.rs`);
    // y, z: the canopy's fade from shade to crowns, metres from the eye;
    // w: the forest's occlusion, 0 (off) to 1 (`forest_cover`)
    tactical: vec4<f32>,
    // mean crown colour (linear) of conifers, broadleaves, macchia; in the
    // conifers' w the samples' posting (m)
    canopy_conifer: vec4<f32>,
    canopy_broadleaf: vec4<f32>,
    canopy_macchia: vec4<f32>,
    // x: the drawing clock less the front map's base (s); y > 0.5: the
    // front map is read (`fire_ahead`)
    fire_front: vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> terrain: Terrain;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var road_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var road_samp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var grass_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var grass_samp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var ground_col_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var ground_col_samp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var ground_nrm_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var ground_nrm_samp: sampler;
// The realistic ground (2026-11-06, with the scanned plants: `REAL_GROUND`,
// `textures::ground_detail`): CC0 photos, one layer each, high-passed into a
// ratio around one (stored * DETAIL_ENCODE) that multiplies the macro colour
// of the ground texture. The far mips average to one, so from afar the
// ground is the macro colour, which is the photos' mean.
@if(REAL_GROUND)
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var detail_tex: texture_2d_array<f32>;
@if(REAL_GROUND)
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var detail_samp: sampler;
// The front drawn ahead of the game's (`fire_ahead::FRONT_MAP`): a line a
// fire cell, read texel by texel.
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var front_tex: texture_2d<f32>;

// Dry karst, not wet slate: a low reflectance keeps the sun's specular lobe
// from painting a bright sheet across every ridge.
const ROUGHNESS: f32 = 1.0;
const REFLECTANCE: f32 = 0.02;

// Road mask encoding and ribbon geometry: keep in step with `ground_mask.rs`.
const RANGE_M: f32 = 12.0;
const KERB_M: f32 = 1.3;
const ASPHALT: vec3<f32> = vec3<f32>(0.214, 0.214, 0.223);
const ASPHALT_KERB: vec3<f32> = vec3<f32>(0.045, 0.045, 0.050);
const PAVING: vec3<f32> = vec3<f32>(0.74, 0.67, 0.55);
const TRACK: vec3<f32> = vec3<f32>(0.296, 0.234, 0.141);
// the realistic ground's: old asphalt, darker than the painted one
const ASPHALT_REAL: vec3<f32> = vec3<f32>(0.115, 0.112, 0.118);
// and the stone of its paved streets and yards, weathered grey
const PAVING_REAL: vec3<f32> = vec3<f32>(0.15, 0.142, 0.125);
const TRACK_KERB: vec3<f32> = vec3<f32>(0.119, 0.091, 0.056);

// One layer of tussocks on a jittered grid of `size` metres. Returns
// (coverage, tone, tip lightness, sun-side shading). `occupancy` is the share
// of cells that hold a tuft; radii are in cell units.
fn tuft_layer(wp: vec2<f32>, size: f32, occupancy: f32, r_min: f32, r_max: f32, seed: f32) -> vec4<f32> {
    let q = wp / size;
    let base_cell = floor(q);
    // Tuft centres sit in [0.15, 0.85] of their cell and radii are at most 0.5,
    // so only the own cell and the neighbour on the nearer side can reach a
    // pixel: 2 x 2 cells, not 3 x 3.
    let f = q - base_cell;
    let step_x = select(-1, 1, f.x > 0.5);
    let step_y = select(-1, 1, f.y > 0.5);
    var best = vec4<f32>(0.0, 0.5, 0.0, 0.0);
    for (var iy = 0; iy < 2; iy++) {
        for (var ix = 0; ix < 2; ix++) {
            let dx = ix * step_x;
            let dy = iy * step_y;
            let c = base_cell + vec2<f32>(f32(dx), f32(dy)) + seed;
            let h1 = hash21(c);
            if h1 > occupancy { continue; }
            let h2 = hash21(c + 17.3);
            let h3 = hash21(c + 31.7);
            let centre = c - seed + vec2<f32>(0.15 + 0.7 * h2, 0.15 + 0.7 * h3);
            let r = r_min + (r_max - r_min) * hash21(c + 5.1);
            let v = (q - centre) / r;
            let d = length(v);
            let ang = atan2(v.y, v.x);
            // Blades: radial streaks that lengthen and shorten around the tuft.
            let blades = 0.5 + 0.5 * sin(ang * (9.0 + 7.0 * h2) + h3 * 6.28 + d * 1.5);
            let reach = 0.55 + 0.45 * blades;
            let a = 1.0 - smoothstep(reach * 0.80, reach, d);
            if a > best.x {
                let side = dot(v / max(d, 0.001), vec2<f32>(-0.55, 0.83)) * (1.0 - d * 0.5);
                best = vec4<f32>(a, h1 / max(occupancy, 0.001) * 0.5 + 0.5 * hash21(c + 9.9), smoothstep(0.30, 0.95, d / reach), side);
            }
        }
    }
    return best;
}

// Layers of the detail array (`textures::GROUND_LAYERS`) and the metres a
// photo covers (`tools/terrain/ground_detail.py`).
const DETAIL_ENCODE: f32 = 0.33;
const L_GRASS: i32 = 0;
const L_SOIL: i32 = 1;
const L_NEEDLES: i32 = 2;
const L_LEAVES: i32 = 3;
const L_ROCK: i32 = 4;
const L_SAND: i32 = 5;
const L_TRACK: i32 = 6;
const L_BURNT: i32 = 7;
const L_ASPHALT: i32 = 8;
// the towns' setts (Poly Haven «Cobblestone Floor 03»), and their tile
const L_PAVING: i32 = 9;
const SIZE_PAVING: f32 = 3.5;

// One photo layer at `uv` (in tiles), with the gradients of the uv: read
// with explicit gradients, so it may be called in any branch.
@if(REAL_GROUND)
fn photo_at(layer: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec3<f32> {
    return textureSampleGrad(detail_tex, detail_samp, uv, layer, dx, dy).rgb / DETAIL_ENCODE;
}

// A photo of `size` metres laid on the plane coordinates `wp` (m, gradients
// `dx`, `dy`) at two scales: as it is, and turned and 6 times larger, so
// neither repeat shows; their product has a mean of one.
@if(REAL_GROUND)
fn photo(layer: i32, size: f32, wp: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec3<f32> {
    let r1 = photo_at(layer, wp / size, dx / size, dy / size);
    let rot = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);
    let s2 = size * 6.0;
    let r2 = photo_at(layer, rot * wp / s2 + vec2<f32>(0.37, 0.71), rot * dx / s2, rot * dy / s2);
    return mix(vec3<f32>(1.0), r1, 0.9) * mix(vec3<f32>(1.0), r2, 0.6);
}

// What the photo layers make of the macro colour `rgb` at `p`: the meadow's
// dry grass, and elsewhere what the canopy's kind says lies on the ground
// (`kind`: 0 pine needles, 0.5 leaf litter, 1 the macchia's stony soil),
// sand on the low flat shore. Blended by height (each photo's brightness),
// so stones and tufts poke through at the edges instead of a cross-fade.
// `paint` is how much of the pixel is the crowns painted from afar, which
// keep their own colour.
@if(REAL_GROUND)
fn real_ground(rgb: vec3<f32>, p: vec3<f32>, meadow: f32, kind: f32, nrm: vec3<f32>, sd: f32, paint: f32, dx: vec2<f32>, dy: vec2<f32>) -> vec3<f32> {
    let wp = p.xz;
    let grass = smoothstep(0.35, 0.65, meadow + (value_noise(wp * 0.35) - 0.5) * 0.45);
    // bare patches in the meadow, as in the painted one
    let clump = smoothstep(0.20, 0.70, value_noise(wp * 0.045 + 7.0) * 0.7 + value_noise(wp * 0.17) * 0.3);
    let k = clamp(kind, 0.0, 1.0);
    let needles = clamp(1.0 - 2.0 * k, 0.0, 1.0);
    let soil = clamp(2.0 * k - 1.0, 0.0, 1.0);
    let leaves = 1.0 - needles - soil;
    let open = 1.0 - grass;
    let bare = grass * (1.0 - clump) * 0.45;
    let sand = (1.0 - smoothstep(3.0, 8.0, p.y)) * smoothstep(0.86, 0.94, nrm.y) * (1.0 - smoothstep(60.0, 110.0, -sd));
    var w = array<f32, 5>(
        (grass - bare) * (1.0 - sand),
        (open * soil + bare) * (1.0 - sand),
        open * needles * (1.0 - sand),
        open * leaves * (1.0 - sand),
        sand,
    );
    var r = array<vec3<f32>, 5>(vec3<f32>(1.0), vec3<f32>(1.0), vec3<f32>(1.0), vec3<f32>(1.0), vec3<f32>(1.0));
    if w[0] > 0.01 { r[0] = photo(L_GRASS, 2.0, wp, dx, dy); }
    if w[1] > 0.01 { r[1] = photo(L_SOIL, 4.0, wp, dx, dy); }
    if w[2] > 0.01 { r[2] = photo(L_NEEDLES, 1.5, wp, dx, dy); }
    if w[3] > 0.01 { r[3] = photo(L_LEAVES, 2.0, wp, dx, dy); }
    if w[4] > 0.01 { r[4] = photo(L_SAND, 15.0, wp, dx, dy); }
    // height blend: the higher photo wins where the weights are close
    var top = -1.0;
    for (var i = 0; i < 5; i++) {
        top = max(top, w[i] + 0.4 * dot(r[i], vec3<f32>(0.2126, 0.7152, 0.0722)));
    }
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < 5; i++) {
        let b = max(w[i] + 0.4 * dot(r[i], vec3<f32>(0.2126, 0.7152, 0.0722)) - top + 0.2, 0.0) * step(0.01, w[i]);
        sum += r[i] * b;
        total += b;
    }
    let ratio = select(vec3<f32>(1.0), sum / max(total, 1.0e-4), total > 1.0e-4) * ground_patches(wp);
    return rgb * mix(ratio, vec3<f32>(1.0), paint);
}

// The rock faces' photo, triplanar: the projections on the two vertical
// planes and the horizontal one, weighted by the normal, so a face is never
// stretched. `g` holds the gradients of p.x, p.y, p.z along the screen's x
// (xyz of the first) and y (of the second).
@if(REAL_GROUND)
fn rock_detail(p: vec3<f32>, nrm: vec3<f32>, gx: vec3<f32>, gy: vec3<f32>) -> vec3<f32> {
    var w = pow(abs(nrm), vec3<f32>(4.0));
    w = w / (w.x + w.y + w.z);
    var r = vec3<f32>(0.0);
    if w.x > 0.02 { r += w.x * photo(L_ROCK, 20.0, p.zy, gx.zy, gy.zy); }
    if w.y > 0.02 { r += w.y * photo(L_ROCK, 20.0, p.xz, gx.xz, gy.xz); }
    if w.z > 0.02 { r += w.z * photo(L_ROCK, 20.0, p.xy, gx.xy, gy.xy); }
    let wsum = select(w.x, 0.0, w.x <= 0.02) + select(w.y, 0.0, w.y <= 0.02) + select(w.z, 0.0, w.z <= 0.02);
    return r / max(wsum, 1.0e-3);
}

// Patches a few tens of metres across, lighter and darker, drier and greener:
// what keeps a slope from reading as one flat colour where the photos have
// faded to their mean.
@if(REAL_GROUND)
fn ground_patches(wp: vec2<f32>) -> vec3<f32> {
    let a = value_noise(wp * 0.041 + 13.0) - 0.5;
    let b = value_noise(wp * 0.013 + 71.0) - 0.5;
    let c = value_noise(wp * 0.11 + 5.0) - 0.5;
    let light = 1.0 + 0.22 * a + 0.16 * b + 0.10 * c;
    return light * mix(vec3<f32>(1.0), vec3<f32>(0.94, 1.02, 1.04), 0.5 + b);
}

// The road network, paving, waterline and
// meadow, drawn over the ground colour from the distance-field mask.
fn terrain_albedo(base: vec4<f32>, p: vec3<f32>, mask_s: vec4<f32>, meadow: f32, nrm: vec3<f32>, grads: vec4<f32>, gx: vec3<f32>, gy: vec3<f32>) -> vec4<f32> {
    let mask_d = (mask_s.rg * 2.0 - 1.0) * RANGE_M;
    let metres_per_px = max(length(dpdx(p.xz)), length(dpdy(p.xz)));
    // A road never gets thinner than about a pixel and a half, so the
    // network stays legible from high up; the kerb fades into the asphalt
    // as it would become sub-pixel.
    let grow_t = max(0.0, 0.8 * metres_per_px - 1.6);
    let grow_d = max(0.0, 0.8 * metres_per_px - 4.8);
    let aa = max(0.5 * metres_per_px, 0.25);
    let kerb_fade = clamp(1.0 - (metres_per_px - 3.0) / 8.0, 0.0, 1.0);
    var rgb = base.rgb;

    // Meadow: dry tussocks over the ground colour, in two sizes and in
    // clumps with bare patches between. Far away they average into a
    // grain of the same mean tone.
    // (the realistic ground has the photo's grass instead)
    @if(!REAL_GROUND)
    if terrain.mask_rect.z > 0.5 && meadow > 0.01 {
        let wp = p.xz;
        let patch_dry = value_noise(wp * 0.02 + 40.0);
        let cover = smoothstep(0.35, 0.65, meadow + (value_noise(wp * 0.35) - 0.5) * 0.45);
        let clump = smoothstep(0.20, 0.70, value_noise(wp * 0.045 + 7.0) * 0.7 + value_noise(wp * 0.17) * 0.3);
        let fine = 1.0 - smoothstep(1.2, 4.0, metres_per_px);
        let blades_k = 1.0;
        var big = vec4<f32>(0.0, 0.5, 0.0, 0.0);
        var small = vec4<f32>(0.0, 0.5, 0.0, 0.0);
        if fine > 0.02 {
            big = tuft_layer(wp, 5.0, (0.25 + 0.75 * cover * (0.35 + 0.65 * clump)) * blades_k, 0.28, 0.50, 0.0);
            // The small tufts only show up close.
            if fine > 0.5 {
                small = tuft_layer(wp, 2.2, 0.55 * cover * clump * blades_k, 0.22, 0.40, 40.0);
            }
        }
        // Dark brown-olive tussocks, lit on the sun side and lighter at the tips.
        let body_k = vec3<f32>(0.50, 0.46, 0.36) * mix(1.0, 0.9, patch_dry);
        var near = rgb * 0.97;
        for (var i = 0; i < 2; i++) {
            let t = select(big, small, i == 1);
            let lit = 1.0 + 0.22 * t.w;
            let col = mix(rgb * body_k * (0.78 + 0.5 * t.y) * lit, rgb * vec3<f32>(0.92, 0.88, 0.70), t.z * 0.55);
            near = mix(near, col, t.x);
        }
        let far = rgb * (1.0 - 0.14 * cover * (0.5 + 0.5 * clump));
        rgb = mix(rgb, mix(far, near, fine), cover);

        // Where the real blades (`near_cover.rs`) are not drawn, beyond the
        // distance from the eye in terrain.mask_rect.w (0 = no blades anywhere, grain
        // everywhere), the meadow keeps some contrast: a light and dark grain
        // with a wavelength of a few pixels, so it holds still as the camera
        // moves, in two octaves, tinted dry and green. It fades in over the
        // stretch where the blades fade out and away again on the overview,
        // where it would only be noise.
        let d_eye = distance(p, view.world_position.xyz);
        let past_blades = select(1.0, smoothstep(0.6 * terrain.mask_rect.w, terrain.mask_rect.w, d_eye), terrain.mask_rect.w > 0.5);
        let grain_k = cover * past_blades * (1.0 - smoothstep(20.0, 60.0, metres_per_px));
        if grain_k > 0.01 {
            let wl = max(3.0 * metres_per_px, 1.0);
            let g = (value_noise(wp / wl) - 0.5) * 0.8 + (value_noise(wp / (wl * 3.1) + 9.0) - 0.5) * 1.0;
            let dry = smoothstep(-0.15, 0.25, g);
            let tint = mix(vec3<f32>(0.84, 0.97, 0.80), vec3<f32>(1.08, 1.02, 0.84), dry);
            rgb = rgb * mix(vec3<f32>(1.0), tint * (1.0 + 0.30 * g), grain_k);
        }
    }

    // Waterline: signed distance, positive in the sea. The seabed under
    // the (partly transparent) water goes from sand through turquoise to
    // deep blue; the dry side gets a damp edge. Vertex colour only knows
    // the coast to 5 m, the mask knows it to the pixel.
    let sd = mask_s.a * 255.0 - 127.5;
    let wet = smoothstep(-0.5, 1.5, sd);
    if wet > 0.0 {
        let ripple = 0.5 * value_noise(p.xz * 0.9) + 0.5 * value_noise(p.xz * 0.23 + 4.0);
        let sandy = vec3<f32>(0.62, 0.57, 0.40) * (0.88 + 0.22 * ripple);
        let lagoon = vec3<f32>(0.10, 0.30, 0.32);
        let deep = vec3<f32>(0.012, 0.034, 0.075);
        var bed = mix(sandy, lagoon, smoothstep(2.0, 18.0, sd));
        bed = mix(bed, deep, smoothstep(14.0, 55.0, sd));
        rgb = mix(rgb, bed, wet);
    } else {
        rgb = rgb * mix(1.0, 0.80, smoothstep(-3.5, 0.0, sd));
    }


    // Rock faces: bedded limestone. Wavy bedding seams, ashlar-like joints
    // and tonal beds drawn in the pixel (the 5 m vertex grid cannot hold
    // them), a wet dark spray band at the foot and a bleached salt line just
    // above it. Everything fades to the vertex colour once a pixel spans
    // more than ~3 m, so the far cliffs never shimmer.
    var steep = (1.0 - smoothstep(0.72, 0.90, nrm.y));
    @if(REAL_GROUND) {
        // bare rock only where it is too steep for soil to hold: the photos'
        // litter and stony soil cover the moderate slopes
        steep = 1.0 - smoothstep(0.62, 0.84, nrm.y);
    }
    if steep > 0.01 && wet < 0.5 {
        let from_sea = clamp(-sd, 0.0, 127.0);
        let rock_k = steep * mix(0.45, 1.0, 1.0 - smoothstep(30.0, 120.0, from_sea)) * (1.0 - wet);
        let fine = 1.0 - smoothstep(1.0, 3.5, metres_per_px);
        let rb = rock_bed(p, nrm);
        let along = rb.x;
        let yy = rb.y;
        let u = rb.z;
        let bed = value_noise(vec2<f32>(along * 0.10, yy * 0.8));
        let seam = smoothstep(0.86, 1.0, abs(fract(u) - 0.5) * 2.0);
        let joint_off = hash21(vec2<f32>(floor(u), 3.0));
        let joint = smoothstep(0.93, 1.0, abs(fract(along * 0.45 + joint_off) - 0.5) * 2.0)
            * (1.0 - smoothstep(0.0, 1.0, abs(fract(u) - 0.5) * 2.0));
        let tone = 0.80 + 0.34 * bed + 0.10 * (value_noise(vec2<f32>(along * 0.7, yy * 0.25)) - 0.5);
        var rock = mix(rgb, limestone(p) * (0.94 + 0.12 * value_noise(p.xz * 0.5)), 0.85) * tone * (1.0 - fine * (0.42 * seam + 0.24 * joint));
        // spray: dark and wet at the foot, salt-bleached above it
        let edge_n = value_noise(p.xz * 0.35) - 0.5;
        let spray = 1.0 - smoothstep(0.6, 2.8, p.y + edge_n * 1.6);
        let salt = smoothstep(2.0, 3.2, p.y + edge_n * 1.2) * (1.0 - smoothstep(3.6, 5.4, p.y + edge_n * 1.2));
        @if(REAL_GROUND) {
            // the photo's own grey for the painted limestone's paler one
            rock = rock * 0.45 * rock_detail(p, nrm, gx, gy);
        }
        rock = mix(rock, rock * vec3<f32>(0.46, 0.47, 0.50), spray * 0.8);
        rock = mix(rock, vec3<f32>(0.92, 0.91, 0.86), salt * 0.30);
        rgb = mix(rgb, rock, rock_k);
    }

    // Paved ground: town squares and yards, as running-bond stones. The
    // edge is ragged with noise; the stone pattern fades to its mean
    // colour as it shrinks below a pixel, so it never shimmers.
    let wp = p.xz;
    let ragged = (0.6 * value_noise(wp * 0.7) + 0.4 * value_noise(wp * 0.15 + 9.1) - 0.5) * 0.30;
    let paved = smoothstep(0.42, 0.58, mask_s.b + ragged);
    // Slabs 1.6 x 0.8 m in running bond, each its own tone, with a seam
    // that never gets thinner than a pixel.
    let q0 = vec2<f32>(wp.x / 1.6, wp.y / 0.8);
    let row = floor(q0.y);
    let q = vec2<f32>(q0.x + 0.5 * (row - 2.0 * floor(row * 0.5)), q0.y);
    let cell = floor(q);
    let f = fract(q);
    let seam_w = max(0.04, 0.7 * metres_per_px / 0.8);
    let edge_d = min(min(f.x * 2.0, (1.0 - f.x) * 2.0), min(f.y, 1.0 - f.y));
    let seam = 1.0 - smoothstep(seam_w * 0.5, seam_w * 1.5, edge_d);
    let detail = 1.0 - smoothstep(0.7, 1.8, metres_per_px);
    let tone = hash21(cell);
    let stone = mix(vec3<f32>(0.90, 0.92, 0.95), vec3<f32>(1.08, 1.02, 0.92), tone) * (0.90 + 0.16 * hash21(cell + 7.7));
    let patch_tone = 0.90 + 0.16 * value_noise(wp * 0.05) + 0.06 * value_noise(wp * 0.4);
    var paving = PAVING * patch_tone * mix(vec3<f32>(0.97), stone * (1.0 - 0.22 * seam), detail);
    @if(REAL_GROUND) {
        // worn grey setts, photographed: one scale only, so the courses
        // stay straight; their mip fades to the mean far away
        if paved > 0.01 {
            let k = 1.0 / SIZE_PAVING;
            let setts = photo_at(L_PAVING, wp * k, grads.xy * k, grads.zw * k);
            paving = PAVING_REAL * patch_tone * setts;
        }
    }
    rgb = mix(rgb, paving, paved);
    let t_kerb = 1.0 - smoothstep(grow_t + KERB_M - aa, grow_t + KERB_M + aa, mask_d.y);
    let t_surf = 1.0 - smoothstep(grow_t - aa, grow_t + aa, mask_d.y);
    let d_kerb = 1.0 - smoothstep(grow_d + KERB_M - aa, grow_d + KERB_M + aa, mask_d.x);
    let d_surf = 1.0 - smoothstep(grow_d - aa, grow_d + aa, mask_d.x);
    var track = TRACK;
    var gravel = TRACK * 0.62;
    var tarmac = ASPHALT * mix(0.97, 0.86 + 0.28 * value_noise(wp * 2.3) * value_noise(wp * 0.4 + 3.0) * 1.6, detail);
    @if(REAL_GROUND) {
        if t_kerb > 0.01 {
            track = track * photo(L_TRACK, 2.0, wp, grads.xy, grads.zw);
        }
        if d_kerb > 0.01 {
            tarmac = ASPHALT_REAL * photo(L_ASPHALT, 30.0, wp, grads.xy, grads.zw);
            gravel = TRACK * 0.62 * photo(L_TRACK, 2.0, wp * 1.3, grads.xy * 1.3, grads.zw * 1.3);
        }
    }
    rgb = mix(rgb, mix(track, TRACK_KERB, kerb_fade), t_kerb);
    rgb = mix(rgb, track, t_surf);
    var asphalt = ASPHALT;
    var kerb = ASPHALT_KERB;
    @if(REAL_GROUND) {
        // no kerbs on these roads: a gravel shoulder
        asphalt = ASPHALT_REAL;
        kerb = gravel;
    }
    rgb = mix(rgb, mix(asphalt, kerb, kerb_fade), d_kerb);
    rgb = mix(rgb, tarmac, d_surf);
    return vec4<f32>(rgb, base.a);
}

// The canopy painted on the ground (`vegetation::canopy`): every crown of
// the full-density forest as a disc, `cover` of the sample. Far from the eye
// it is the crowns' colour, broken up a little so it does not read as paint;
// near it, where the real plants stand, only their shade and litter.
// The forest around a point, as the sky sees it (2026-11-04): the canopy's
// cover averaged over ~25 m (x) and ~100 m (y), read from the mips of the
// ground colour's alpha. Zero when the occlusion is off (the texture then has
// no mips). A closed stand hides the sky from the ground under it and from
// its own lower crowns; a clearing or an edge keeps it.
const FOREST_MID_M: f32 = 25.0;
const FOREST_WIDE_M: f32 = 100.0;
fn forest_cover(guv: vec2<f32>) -> vec2<f32> {
    if terrain.tactical.w <= 0.0 {
        return vec2<f32>(0.0);
    }
    let lp = log2(max(terrain.canopy_conifer.w, 0.5));
    let mid = textureSampleLevel(ground_col_tex, ground_col_samp, guv, max(log2(FOREST_MID_M) - lp, 0.0)).a;
    let wide = textureSampleLevel(ground_col_tex, ground_col_samp, guv, max(log2(FOREST_WIDE_M) - lp, 0.0)).a;
    return vec2<f32>(mid, wide) * terrain.tactical.w;
}

// The ground's ambient under the forest, and the crowns' (painted far away,
// they stand above the stand and see most of the sky).
fn forest_ao(forest: vec2<f32>, cover: f32, far: f32) -> f32 {
    let ground = 1.0 - 0.38 * forest.x - 0.30 * forest.y;
    let crowns = 1.0 - 0.15 * forest.y;
    return mix(ground, crowns, cover * far);
}

fn canopy_far(p: vec3<f32>) -> f32 {
    return smoothstep(terrain.tactical.y, terrain.tactical.z, distance(p, view.world_position.xyz));
}

fn with_canopy(rgb_in: vec3<f32>, p: vec3<f32>, cover_in: f32, kind: f32, light: f32, forest: vec2<f32>, guv: vec2<f32>) -> vec3<f32> {
    if cover_in < 0.004 {
        return rgb_in;
    }
    var cover = cover_in;
    @if(REAL_GROUND) {
        // The crowns painted from afar only where the wood is closed: a lone
        // olive's disc round a real tree drawn far off read as a green halo
        // (2026-11-06). The stand around: the cover 12 m to each side.
        if canopy_far(p) > 0.01 {
            let d = 12.0 * terrain.ground_rect.xy;
            let around = 0.25 * (textureSampleLevel(ground_col_tex, ground_col_samp, guv + vec2<f32>(d.x, 0.0), 0.0).a
                + textureSampleLevel(ground_col_tex, ground_col_samp, guv - vec2<f32>(d.x, 0.0), 0.0).a
                + textureSampleLevel(ground_col_tex, ground_col_samp, guv + vec2<f32>(0.0, d.y), 0.0).a
                + textureSampleLevel(ground_col_tex, ground_col_samp, guv - vec2<f32>(0.0, d.y), 0.0).a);
            cover = cover * mix(1.0, smoothstep(0.2, 0.55, around), canopy_far(p));
        }
    }
    // far away the gaps between the painted crowns are the shade deep in
    // the stand, and the crowns of a big wood are darker than those at its
    // edge or alone (they see less sky)
    let far0 = canopy_far(p);
    let rgb = rgb_in * mix(1.0, 1.0 - 0.45 * forest.x - 0.20 * forest.y, far0);
    let crown_k = 1.0 + 0.10 * (1.0 - forest.y) * step(0.001, terrain.tactical.w) - 0.22 * forest.y;
    let far = far0;
    let crown = crown_k * mix(
        mix(terrain.canopy_conifer.rgb, terrain.canopy_broadleaf.rgb, clamp(kind * 2.0, 0.0, 1.0)),
        terrain.canopy_macchia.rgb,
        clamp(kind * 2.0 - 1.0, 0.0, 1.0),
    ) * clamp(light, 0.5, 1.6);
    let mottle = 0.78 + 0.44 * (0.6 * value_noise(p.xz * 0.22) + 0.4 * value_noise(p.xz * 0.07 + 3.0));
    var shade = rgb * vec3<f32>(0.80, 0.84, 0.78);
    @if(REAL_GROUND) {
        // shade, not a green wash
        shade = rgb * 0.80;
    }
    let near_rgb = mix(rgb, shade, cover);
    let far_rgb = mix(rgb, crown * mottle, cover);
    return mix(near_rgb, far_rgb, far);
}

// The fire map's g and b (`fire_view::update_fire_map`) from the front
// map instead (`fire_ahead`): each corner's share burnt on this frame's
// clock, and its arrival, then bilinear as the filter would. g how burnt, b
// the arrival against now weighted by g; r and a stay the fire map's.
fn front_fire(uv: vec2<f32>, fire: vec4<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(front_tex));
    let at = uv * vec2<f32>(dims) - 0.5;
    let c0 = vec2<i32>(floor(at));
    let f = at - floor(at);
    let clock = terrain.fire_front.x;
    var g = 0.0;
    var b = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let wk = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        let l = textureLoad(front_tex, clamp(c0 + o, vec2<i32>(0), dims - 1), 0);
        let v = l.g + l.b * clock;
        let s = select(clamp(v, l.a, 1.0), clamp(v, 0.0, l.a), l.b >= 0.0);
        g += wk * s;
        b += wk * s * min(l.r - clock, 0.0);
    }
    return vec4<f32>(fire.r, g, b, fire.a);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let p = in.world_position.xyz;
    // sampled outside any branch: derivatives need uniform control flow
    let mask_uv = vec2<f32>(p.x, -p.z) * terrain.mask_rect.xy;
    let mask_s = textureSample(road_tex, road_samp, mask_uv);
    let meadow = textureSample(grass_tex, grass_samp, mask_uv).r;
    // The fire map shares the meadow's texture (one texel per fire cell),
    // read at a point warped by ~7 m as `field::FireField::warp` does.
    let wn = vec2<f32>(p.x, -p.z);
    let fire_warp = vec2<f32>(value_noise(wn * 0.045 + 11.0), value_noise(wn * 0.045 + vec2<f32>(3.0, 29.0))) - 0.5;
    let fire_uv = (wn + fire_warp * 14.0) * terrain.mask_rect.xy;
    var fire_s = textureSample(grass_tex, grass_samp, fire_uv);
    if terrain.fire_front.y > 0.5 {
        fire_s = front_fire(fire_uv, fire_s);
    }
    let mpp = max(length(dpdx(p.xz)), length(dpdy(p.xz)));
    // colour and normal per sample (texel centres on the samples), not per
    // vertex: the mesh may be coarser than the data
    let gr = terrain.ground_rect;
    let guv = vec2<f32>(p.x * gr.x + gr.z, p.z * gr.y + gr.w);
    let g_col4 = textureSampleLevel(ground_col_tex, ground_col_samp, guv, 0.0);
    let g_nrm4 = textureSampleLevel(ground_nrm_tex, ground_nrm_samp, guv, 0.0);
    let forest = forest_cover(guv);
    let g_col = with_canopy(g_col4.rgb, p, g_col4.a, g_nrm4.z, g_nrm4.w, forest, guv);
    // the normal's y is always up: rebuilt from x and z
    var g_nrm = normalize(vec3<f32>(g_nrm4.x, sqrt(max(1.0 - g_nrm4.x * g_nrm4.x - g_nrm4.y * g_nrm4.y, 0.0)), g_nrm4.y));
    if !is_front {
        g_nrm = -g_nrm;
    }

    var pbr = pbr_types::pbr_input_new();
    pbr.flags = mesh[in.instance_index].flags;
    pbr.is_orthographic = view.clip_from_view[3].w == 1.0;
    pbr.V = pbr_functions::calculate_view(in.world_position, pbr.is_orthographic);
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    // the fire's scar into the ground, before the light (`fire_ground`)
    let fire = fire_ground(p, fire_s, (mask_s.rg * 2.0 - 1.0) * RANGE_M, mpp);
    pbr.world_normal = g_nrm;
    pbr.N = normalize(g_nrm + rock_bump(p, g_nrm) + fire.bump);
    // the plane's and the rock face's coordinates with their gradients, for
    // the photo layers (taken here, in uniform control flow)
    let grads = vec4<f32>(dpdx(p.xz), dpdy(p.xz));
    let gx = dpdx(p);
    let gy = dpdy(p);
    var ground = g_col;
    @if(REAL_GROUND) {
        let sd = mask_s.a * 255.0 - 127.5;
        ground = real_ground(g_col, p, meadow, g_nrm4.z, g_nrm, sd, g_col4.a * canopy_far(p), grads.xy, grads.zw);
    }
    let albedo = terrain_albedo(vec4<f32>(ground, 1.0), p, mask_s, meadow, g_nrm, grads, gx, gy);
    var scar = fire.scar.rgb;
    @if(REAL_GROUND) {
        if fire.scar.a > 0.01 {
            scar = scar * photo(L_BURNT, 1.0, p.xz, grads.xy, grads.zw);
        }
    }
    pbr.material.base_color = vec4<f32>(mix(albedo.rgb, scar, fire.scar.a), albedo.a);
    pbr.material.perceptual_roughness = ROUGHNESS;
    pbr.material.metallic = 0.0;
    pbr.material.reflectance = vec3<f32>(REFLECTANCE);
    pbr.material.flags |= pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr.diffuse_occlusion = vec3<f32>(forest_ao(forest, g_col4.a, canopy_far(p)));

    var out: FragmentOutput;
    out.color = apply_pbr_lighting_faded(pbr);
    // the burning band: light of its own, over the lit ground, under the fog
    out.color = vec4<f32>(mix(out.color.rgb, fire.glow.rgb, fire.glow.a), out.color.a);
    out.color = pbr_functions::main_pass_post_lighting_processing(pbr, out.color);
    out.color = tactical_grey(out.color, terrain.tactical.x);
    return out;
}
