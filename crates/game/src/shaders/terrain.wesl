// The terrain (`materials/terrain.rs`): ground colour and normal per terrain
// sample, the road network and the meadow drawn from distance-field masks,
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

struct Terrain {
    // world metres to mask uv (xy); z > 0.5: meadow on; w: where the blades
    // end, metres from the eye (0 = no blades)
    mask_rect: vec4<f32>,
    // world metres to the ground textures' uv: x, y = 1 / (posting * size),
    // z = half a texel in u, w = the north edge plus half a texel in v
    ground_rect: vec4<f32>,
    // x: tactical desaturation, 0 (full colour) to 1 (`tactical.rs`)
    tactical: vec4<f32>,
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

// The road network, paving, waterline and
// meadow, drawn over the ground colour from the distance-field mask.
fn terrain_albedo(base: vec4<f32>, p: vec3<f32>, mask_s: vec4<f32>, meadow: f32, nrm: vec3<f32>) -> vec4<f32> {
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
    let steep = (1.0 - smoothstep(0.72, 0.90, nrm.y));
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
    let paving = PAVING * patch_tone * mix(vec3<f32>(0.97), stone * (1.0 - 0.22 * seam), detail);
    rgb = mix(rgb, paving, paved);
    let t_kerb = 1.0 - smoothstep(grow_t + KERB_M - aa, grow_t + KERB_M + aa, mask_d.y);
    let t_surf = 1.0 - smoothstep(grow_t - aa, grow_t + aa, mask_d.y);
    let d_kerb = 1.0 - smoothstep(grow_d + KERB_M - aa, grow_d + KERB_M + aa, mask_d.x);
    let d_surf = 1.0 - smoothstep(grow_d - aa, grow_d + aa, mask_d.x);
    rgb = mix(rgb, mix(TRACK, TRACK_KERB, kerb_fade), t_kerb);
    rgb = mix(rgb, TRACK, t_surf);
    rgb = mix(rgb, mix(ASPHALT, ASPHALT_KERB, kerb_fade), d_kerb);
    let tarmac = ASPHALT * mix(0.97, 0.86 + 0.28 * value_noise(wp * 2.3) * value_noise(wp * 0.4 + 3.0) * 1.6, detail);
    rgb = mix(rgb, tarmac, d_surf);
    return vec4<f32>(rgb, base.a);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let p = in.world_position.xyz;
    // sampled outside any branch: derivatives need uniform control flow
    let mask_uv = vec2<f32>(p.x, -p.z) * terrain.mask_rect.xy;
    let mask_s = textureSample(road_tex, road_samp, mask_uv);
    let meadow = textureSample(grass_tex, grass_samp, mask_uv).r;
    // colour and normal per sample (texel centres on the samples), not per
    // vertex: the mesh may be coarser than the data
    let gr = terrain.ground_rect;
    let guv = vec2<f32>(p.x * gr.x + gr.z, p.z * gr.y + gr.w);
    let g_col = textureSampleLevel(ground_col_tex, ground_col_samp, guv, 0.0).rgb;
    var g_nrm = normalize(textureSampleLevel(ground_nrm_tex, ground_nrm_samp, guv, 0.0).xyz);
    if !is_front {
        g_nrm = -g_nrm;
    }

    var pbr = pbr_types::pbr_input_new();
    pbr.flags = mesh[in.instance_index].flags;
    pbr.is_orthographic = view.clip_from_view[3].w == 1.0;
    pbr.V = pbr_functions::calculate_view(in.world_position, pbr.is_orthographic);
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    pbr.world_normal = g_nrm;
    pbr.N = normalize(g_nrm + rock_bump(p, g_nrm));
    pbr.material.base_color = terrain_albedo(vec4<f32>(g_col, 1.0), p, mask_s, meadow, g_nrm);
    pbr.material.perceptual_roughness = ROUGHNESS;
    pbr.material.metallic = 0.0;
    pbr.material.reflectance = vec3<f32>(REFLECTANCE);
    pbr.material.flags |= pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting_faded(pbr);
    out.color = pbr_functions::main_pass_post_lighting_processing(pbr, out.color);
    out.color = tactical_grey(out.color, terrain.tactical.x);
    return out;
}
