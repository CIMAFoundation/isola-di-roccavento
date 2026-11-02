// Near-field ground cover: grass tufts drawn around the camera.
//
// Nothing here is stored per plant. The mesh is a fixed square lattice of
// cells (`near_cover.rs`); each vertex carries only which cell and which blade
// it belongs to, and this vertex shader decides where that blade stands in the
// world, whether it exists (cover map, roads, shore, slope), how tall it is,
// how it sways and whether the fire has been through it. The lattice follows
// the camera by whole cells with the cell index wrapped, so a blade keeps its
// world position as the camera moves: it is hashed from the world cell.
//
// Vertex attributes (mesh built in `near_cover.rs`):
//   position = (cell ix, cell iz, blade index k)
//   normal   = (side -1..1, height fraction 0..1, unused)
//   uv       = (side * 0.5 + 0.5, height fraction)
//   color    = unused (present so the pipeline defines VERTEX_COLORS)

import bevy_pbr::render::mesh_view_bindings::{view, globals};
import bevy_pbr::render::forward_io::{Vertex, VertexOutput};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing};
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
import game::shaders::noise::{hash21, value_noise};

struct Params {
    // lattice centre (world x, world z), draw radius, cell size
    focus: vec4<f32>,
    // lattice side in cells, world height of the terrain texture, terrain posting, tactical amount
    lattice: vec4<f32>,
    // x = simulated seconds, y = scale of this ring, z, w = this ring's band of distance from the eye (z = 0: nothing nearer)
    fire: vec4<f32>,
    // 1 / world width, 1 / world height, spare, spare (road mask coordinates)
    rect: vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var height_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var cover_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var cover_samp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var road_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var road_samp: sampler;

const RANGE_M: f32 = 12.0;
const HASH_K: u32 = 747796405u;

fn pcg(v: u32) -> u32 {
    let s = v * HASH_K + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// Three decorrelated uniform numbers in [0, 1) from a lattice cell and a salt.
fn rnd3(c: vec2<i32>, salt: u32) -> vec3<f32> {
    let h0 = pcg((u32(c.x) * 1597334677u) ^ pcg(u32(c.y) + salt * 3266489917u));
    let h1 = pcg(h0 + 374761393u);
    let h2 = pcg(h1 + 668265263u);
    return vec3<f32>(f32(h0), f32(h1), f32(h2)) * (1.0 / 4294967296.0);
}

fn terrain_h(ix: i32, iz: i32, dims: vec2<i32>) -> f32 {
    return textureLoad(height_tex, clamp(vec2<i32>(ix, iz), vec2<i32>(0), dims - 1), 0).r;
}

// Terrain height at a world position (bilinear over the 5 m render grid, the
// same grid the terrain mesh is built from).
fn ground_h(x: f32, z: f32) -> f32 {
    let dims = vec2<i32>(textureDimensions(height_tex));
    let fx = x / params.lattice.z;
    let fz = (params.lattice.y + z) / params.lattice.z;
    let x0 = i32(floor(fx));
    let z0 = i32(floor(fz));
    let tx = fx - floor(fx);
    let tz = fz - floor(fz);
    let a = mix(terrain_h(x0, z0, dims), terrain_h(x0 + 1, z0, dims), tx);
    let b = mix(terrain_h(x0, z0 + 1, dims), terrain_h(x0 + 1, z0 + 1, dims), tx);
    return mix(a, b, tz);
}

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let cell_i = vec2<i32>(i32(v.position.x), i32(v.position.y));
    let k = u32(v.position.z);
    let side = v.normal.x;
    let t = v.normal.y;
    let n = i32(params.lattice.x);
    let cell = params.focus.w;

    // Which world cell this lattice slot shows: the lattice is centred on the
    // focus, and the slot index wraps so a world cell always lands on the same
    // slot.
    let origin = vec2<i32>(floor(params.focus.xy / cell)) - n / 2;
    let rel = ((cell_i - origin) % n + vec2<i32>(n)) % n;
    let wc = origin + rel;
    let r = rnd3(wc, 1u);
    let rb = rnd3(wc, 11u + k * 7u);

    let tuft = (vec2<f32>(wc) + r.xy) * cell;
    let scale = params.fire.y;

    // Cover under this tuft: R tuft density, B palette, A fire.
    // the 20 m cover cells are sampled at a noise-warped point, so the edge of
    // a meadow or of a burn is ragged rather than square
    let warp = (vec2<f32>(value_noise(tuft * 0.07), value_noise(tuft * 0.07 + 31.0)) - 0.5) * 14.0;
    let cover_uv = (tuft + warp) * params.rect.xy;
    let cv = textureSampleLevel(cover_tex, cover_samp, cover_uv, 0.0);
    let road = textureSampleLevel(road_tex, road_samp, cover_uv, 0.0);
    let road_d = min((road.r * 2.0 - 1.0) * RANGE_M - 5.0 - 1.5, (road.g * 2.0 - 1.0) * RANGE_M - 1.6 - 1.0);
    let shore = road.a * 255.0 - 127.5;
    let sea_ok = select(0.0, 1.0, shore < -6.0);
    let road_ok = smoothstep(0.0, 1.2, road_d) * (1.0 - smoothstep(0.25, 0.45, road.b));

    // Density as a share of cells that grow anything.
    let h0 = ground_h(tuft.x, -tuft.y);
    let d_eye = distance(vec3<f32>(tuft.x, h0, -tuft.y), view.world_position.xyz);
    let patch_n = value_noise(tuft * 0.09 + 3.0);
    let fine_n = value_noise(tuft * 0.6);
    var dens = cv.r;
    dens = dens * (0.8 + 0.5 * patch_n) * (0.85 + 0.3 * fine_n);
    let keep = f32(r.z < dens) * sea_ok * road_ok;
    // This ring's band of distance from the eye. Over the last stretch below
    // its outer edge (0.6..1 of it) a ring dissolves into the next. Each blade
    // dissolves at its own distance (`fade_at`), over a short stretch, with a
    // per-pixel threshold in the fragment shader: at any moment only a few
    // blades are part-drawn, the rest whole, and no blade changes shape as the
    // camera moves (blades that shrank as their ring gave way looked as if
    // they sprouted out of the ground).
    let lo = params.fire.z;
    let hi = params.fire.w;
    let fade_out = 1.0 - smoothstep(0.6 * hi, hi, d_eye);
    let fade_in = select(smoothstep(0.6 * lo, lo, d_eye), 1.0, lo <= 0.0);
    let fade_at = rnd3(wc, 5u + k * 13u).x * 0.85;
    let dissolve = clamp((fade_out * fade_in - fade_at) / 0.15, 0.0, 1.0);
    let thin = f32(dissolve > 0.001);

    // Fire: 0 untouched, ~0.5 burning, 1 burnt, 0.78 cleared to bare ground.
    let burnt = smoothstep(0.9, 1.0, cv.a);
    let cleared = f32(cv.a > 0.72 && cv.a < 0.84);
    let scorch = smoothstep(0.3, 0.6, cv.a) * (1.0 - burnt);
    let alive = keep * thin * (1.0 - cleared);

    // Ground and slope.
    let hx = ground_h(tuft.x + 5.0, -tuft.y) - h0;
    let hz = ground_h(tuft.x, -tuft.y - 5.0) - h0;
    let slope = length(vec2<f32>(hx, hz)) * 0.2;
    let flat_ok = 1.0 - smoothstep(2.6, 4.0, slope);

    let sway_ph = hash21(tuft) * 6.2831;
    let gust = value_noise(tuft * 0.05 + vec2<f32>(globals.time * 0.35, 0.0));
    var wpos: vec3<f32>;
    var shade = 1.0;
    var tone = rb.x;
    var nrm = vec3<f32>(0.0, 1.0, 0.0);

    {
        // --- grass blade -------------------------------------------------
        // A tuft is a handful of blades fanning out from one point.
        let ang = rb.x * 6.2831 + f32(k) * 1.2566;
        let lean = 0.25 + 0.55 * rb.y;
        let h_tuft = mix(0.40, 0.95, patch_n) * (0.6 + 0.8 * r.z) * 1.3 * min(pow(scale, 0.6), 2.4);
        var h = h_tuft * (0.55 + 0.6 * rb.z);
        // blades of burnt ground are a few centimetres of stubble
        h = mix(h, 0.05, burnt) * mix(1.0, 0.7, scorch);
        // wide enough to be a pixel from where the camera is
        let w = 0.06 * scale * (1.0 - 0.7 * t);
        let dir = vec2<f32>(cos(ang), sin(ang));
        let tang = vec2<f32>(-dir.y, dir.x);
        // the blades fan out over the cell, which grows with the ring
        let root = tuft + dir * (0.04 + 0.32 * rb.y) * scale;
        // bend: the tip leans out and sways in the gust
        let bend = t * t * lean * h;
        let wind = vec2<f32>(0.7, 0.4) * sin(globals.time * 1.7 + sway_ph + t * 1.3) * (0.05 + 0.12 * gust) * t * t * h * 3.0 / sqrt(scale);
        let xz = root + dir * bend + tang * side * w + wind;
        let y = ground_h(xz.x, -xz.y) + t * h * (1.0 - 0.18 * t * lean) - 0.03;
        wpos = vec3<f32>(xz.x, y, -xz.y);
        // normal: blades are thin, so light them like the ground they stand
        // on with a little of their own tilt
        let own = normalize(vec3<f32>(dir.x * lean, 1.0, -dir.y * lean));
        nrm = normalize(mix(vec3<f32>(0.0, 1.0, 0.0), own, 0.35));
        shade = mix(0.40, 1.0, t) * (0.85 + 0.3 * rb.z);
        tone = rb.x;
    }

    // dead / absent: collapse the blade to a point below the ground
    let present = alive * flat_ok;
    if present < 0.5 {
        wpos = vec3<f32>(wpos.x, -1000.0, wpos.z);
    }

    out.world_position = vec4<f32>(wpos, 1.0);
    out.position = position_world_to_clip(wpos);
    out.world_normal = nrm;
    out.uv = v.uv;
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = v.instance_index;

    // Colour. Palette from the fuel group (cover.b: 0 meadow, 1/3 broadleaf,
    // 2/3 macchia, 1 pine), tuned in `near_cover.rs`; dry/green by patch.
    let pal = cv.b;
    var green = vec3<f32>(0.26, 0.31, 0.10);
    var dry = vec3<f32>(0.44, 0.35, 0.15);
    if pal > 0.17 && pal < 0.5 {
        green = vec3<f32>(0.10, 0.28, 0.05);
        dry = vec3<f32>(0.30, 0.33, 0.09);
    } else if pal >= 0.5 && pal < 0.83 {
        green = vec3<f32>(0.12, 0.26, 0.05);
        dry = vec3<f32>(0.28, 0.30, 0.09);
    } else if pal >= 0.83 {
        green = vec3<f32>(0.07, 0.17, 0.09);
        dry = vec3<f32>(0.24, 0.17, 0.08);
    }
    var col = mix(green, dry, clamp(0.35 + 0.75 * patch_n - 0.35 * tone + 0.2 * (fine_n - 0.5), 0.0, 1.0));
    col = col * shade;
    // scorch browns, burnt blackens
    col = mix(col, col * vec3<f32>(0.55, 0.34, 0.16), scorch);
    col = mix(col, vec3<f32>(0.035, 0.03, 0.028), burnt);
    out.color = vec4<f32>(col, dissolve);
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    // a blade dissolving between two rings (see the vertex stage):
    // interleaved gradient noise, a threshold per pixel even at every scale
    let ign = fract(52.9829189 * fract(dot(in.position.xy, vec2<f32>(0.06711056, 0.00583715))));
    if ign >= in.color.a {
        discard;
    }
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // base colour from the vertex stage; the material's base colour is white
    pbr_input.material.base_color = vec4<f32>(in.color.rgb, 1.0);
    pbr_input.N = normalize(in.world_normal);
    var out = apply_pbr_lighting(pbr_input);
    out = main_pass_post_lighting_processing(pbr_input, out);
    // tactical view: toward grey, like the terrain and foliage
    let amount = params.lattice.w;
    let luma = dot(out.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    out = vec4<f32>(max(mix(out.rgb, vec3<f32>(luma) * 0.9, amount), vec3<f32>(0.0)), out.a);
    return out;
}
