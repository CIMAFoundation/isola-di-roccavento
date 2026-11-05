// Flames and smoke, ray-marched at a fraction of the screen's resolution
// (`fire_volumetric.rs`, which explains the field). One ray per pixel of
// the small target: through the plume's box, to the scene's depth; out
// come the light the volume sends to the eye (premultiplied), how much of
// the scene it hides, and the depth the ray stopped at (for the
// composite's bilateral upsample) and the volume's distance (for the
// resolve, `fire_resolve.wesl`, which blends in the frames before).

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_pbr::render::mesh_view_bindings::{view, globals, lights};
import game::shaders::scene_light::apply_scene_fog;

struct Params {
    map: vec4<f32>,
    lo: vec4<f32>,
    hi: vec4<f32>,
    wind: vec4<f32>,
    smoke: vec4<f32>,
    flame: vec4<f32>,
    look: vec4<f32>,
    look2: vec4<f32>,
    size: vec4<f32>,
    origin: vec4<f32>,
    frame: vec4<f32>,
    drift: array<vec4<f32>, 4>,
    shear: array<vec4<f32>, 2>,
    prev: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    eye: vec4<f32>,
    more: vec4<f32>,
}

@group(1) @binding(0) var<uniform> p: Params;
@if(MULTISAMPLED)
@group(1) @binding(1) var depth_tex: texture_depth_multisampled_2d;
@else
@group(1) @binding(1) var depth_tex: texture_depth_2d;
@group(1) @binding(2) var plume_tex: texture_2d<f32>;
@group(1) @binding(3) var plume_samp: sampler;
@group(1) @binding(4) var noise_tex: texture_3d<f32>;
@group(1) @binding(5) var noise_samp: sampler;

struct Out {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
    // how far the volume's weighted middle is (the scene's, where none):
    // the resolve reprojects the frames before from there; then the
    // flames' share of the opacity, how far they are, and how high up
    // their tongues (0 foot, 1 tip): the composite carves them finer
    @location(2) mid: vec4<f32>,
}

// The plume map at a point of the ground (Bevy x, z): r smoke given off,
// g how alight, b flame height (m), a ground height (m).
fn plume(xz: vec2<f32>, lod: f32) -> vec4<f32> {
    let uv = vec2<f32>(xz.x, -xz.y) * p.map.xy;
    return textureSampleLevel(plume_tex, plume_samp, uv, min(lod, p.map.w));
}

// Emission of the flame by its temperature (as `fire_volume.wesl`).
fn flame_ramp(h: f32) -> vec3<f32> {
    let c0 = vec3<f32>(0.30, 0.05, 0.01);
    let c1 = vec3<f32>(1.4, 0.18, 0.02);
    let c2 = vec3<f32>(2.8, 0.75, 0.06);
    let c3 = vec3<f32>(3.6, 1.7, 0.25);
    let c4 = vec3<f32>(4.2, 3.0, 1.1);
    let a = smoothstep(0.0, 0.3, h);
    let b = smoothstep(0.3, 0.55, h);
    let c = smoothstep(0.55, 0.8, h);
    let d = smoothstep(0.8, 1.0, h);
    return mix(mix(mix(mix(c0, c1, a), c2, b), c3, c), c4, d);
}

// The noise, at the mip its footprint asks for (`textures::noise_volume`
// has mips): a billow finer than a sample stands for is averaged, not
// sampled (sampled, it sparkles as the camera moves).
fn noise(q: vec3<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(noise_tex, noise_samp, q, lod);
}

// A rotation of the noise's space (about 37° round (1, 2, 1.5)).
const NOISE_TURN = mat3x3<f32>(
    vec3<f32>(0.8303, 0.4210, -0.3654),
    vec3<f32>(-0.3058, 0.8885, 0.3420),
    vec3<f32>(0.4686, -0.1824, 0.8644),
);

// How far smoke `age` seconds old has drifted with the wind of its time
// (`fire_volumetric::WindMemory`): the table holds ages 1/8 … 8/8 of the
// plume's, from no drift at age 0.
fn drift_at(age: f32) -> vec2<f32> {
    let full = p.wind.w / p.wind.z;
    let x = clamp(age / full, 0.0, 1.0) * 8.0;
    let k = min(i32(floor(x)), 7);
    let f = x - f32(k);
    var a = vec2<f32>(0.0);
    if k > 0 {
        let v = p.drift[(k - 1) / 2];
        a = select(v.zw, v.xy, (k - 1) % 2 == 0);
    }
    let v = p.drift[k / 2];
    let b = select(v.zw, v.xy, k % 2 == 0);
    return mix(a, b, f);
}

// How many cells wider smoke `age` seconds old has spread for the winds
// it met (`fire_volumetric::WindMemory`): ages 1/8 … 8/8 of the plume's.
fn shear_at(age: f32) -> f32 {
    let x = clamp(age / (p.wind.w / p.wind.z), 0.0, 1.0) * 8.0;
    let k = min(i32(floor(x)), 7);
    let f = x - f32(k);
    var a = 0.0;
    if k > 0 {
        a = p.shear[(k - 1) / 4][(k - 1) % 4];
    }
    return mix(a, p.shear[k / 4][k % 4], f);
}

// The smoke at `pos`, `h` metres over the ground below it: density (x;
// below 0 where nothing smokes, so the march may stride) and the fire's
// glow from below (y). The parcel left the ground `h / rise` seconds ago
// and drifted since; the map is read where it left, at a mip as wide as
// the plume has grown.
fn smoke_at(pos: vec3<f32>, h: f32, detail: bool, lod: f32) -> vec2<f32> {
    let top = p.wind.w;
    if h < -8.0 || h > top {
        return vec2<f32>(-1.0, 0.0);
    }
    let hh = max(h, 0.0);
    let k = hh / top;
    let t = globals.time;
    var src = pos.xz - drift_at(hh / p.wind.z);
    if detail {
        // the column sways as it rises
        src -= (noise(vec3<f32>(pos.x, hh * 1.5, pos.z) / 600.0 + vec3<f32>(0.0, -t * 0.02, 0.0), 0.0).gb - 0.5) * (40.0 + 160.0 * k);
    }
    let base = 1.0 + hh / p.smoke.z;
    // the winds the parcel met spread it further, and thin it: the mip
    // averages it over the wider ground, only the plume's own growth is
    // made up below, and the widening thins it once more (seen slanting
    // through a wide layer, a spread that only averaged read as a haze as
    // thick as the column)
    let spread = base + shear_at(hh / p.wind.z);
    let s = plume(src, log2(spread) + 0.5).r;
    if s < 0.002 {
        return vec2<f32>(-1.0, 0.0);
    }
    // the mip averages the column over its width: a line of fire would
    // fade as the square of it, the plume spreads mostly across the wind;
    // thin over the flames (the column lifts off the fire, the front shows
    // under it), thickest some tens of metres up, gone at the top
    let lift = 0.06 + 0.94 * smoothstep(p.look.x * 0.07, p.look.x, hh);
    var dens = min(s * pow(base, 0.85), 1.6) * (base / spread) * lift * (1.0 - smoothstep(top * 0.5, top, hh));
    if !detail {
        return vec2<f32>(dens * 0.6, 0.0);
    }
    // billows, carried by the wind and rising with the plume; a slow warp
    // first, so they roll instead of sliding
    let adv = vec3<f32>(p.wind.x, p.wind.z * 0.7, p.wind.y) * t;
    // turned off the axes: the noise repeats every few hundred metres,
    // and a repeat along the vertical reads as horizontal layers
    let q = NOISE_TURN * ((pos - adv) / p.smoke.y);
    let warp = (noise(q * 0.37 + vec3<f32>(0.0, -t * 0.01, 0.0), max(lod - 1.4, 0.0)).gb - 0.5) * 1.2;
    let qw = q + vec3<f32>(warp.x, 0.0, warp.y);
    var n = 0.64 * noise(qw, lod).r + 0.36 * noise(qw * 2.7 + vec3<f32>(0.0, -t * 0.03, 0.0), lod + 1.4).g;
    n = 0.87 * n + 0.13 * noise(qw * 7.1, lod + 2.8).b;
    // dense at the source, torn into shreds toward the top: a narrow
    // window, so the billows have edges
    let edge = p.look.y + 0.16 * k - 0.14 * (1.0 - k);
    dens *= smoothstep(edge, edge + 0.12, n) * (0.6 + 0.8 * n);
    // lit from below by the alight ground under it, widening with height
    var glow = 0.0;
    if hh < 300.0 {
        glow = plume(pos.xz, 1.0 + hh / 40.0).g * exp(-hh / 70.0);
    }
    return vec2<f32>(dens, glow);
}

// The march pixel's place on the screen, as full-resolution pixels of
// the main pass.
fn ndc_of(full: vec2<f32>) -> vec2<f32> {
    let f = full / p.size.zw;
    return vec2<f32>(f.x * 2.0 - 1.0, 1.0 - f.y * 2.0);
}

// A per-pixel jitter of the first step, moved on each frame when the
// history averages it (golden ratio), still otherwise.
fn jitter(frag: vec2<f32>) -> f32 {
    let ign = fract(52.9829189 * fract(0.06711056 * frag.x + 0.00583715 * frag.y));
    return fract(ign + p.frame.x * 0.6180339);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> Out {
    var out: Out;
    // the scene's depth under this pixel, from the full-resolution buffer
    let full = in.position.xy * p.origin.z;
    let at = clamp(full, vec2<f32>(0.0), p.size.zw - 1.0);
    let depth = textureLoad(depth_tex, vec2<i32>(p.origin.xy + at), 0);
    let ndc = ndc_of(full);
    let eye = view.world_position;
    let near = view.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(near.xyz / near.w - eye);
    var t_scene = 1.0e7;
    out.depth = 1.0e7;
    var hit = eye + dir * 1.0e6;
    if depth > 0.0 {
        let h4 = view.world_from_clip * vec4<f32>(ndc, depth, 1.0);
        hit = h4.xyz / h4.w;
        t_scene = distance(hit, eye);
        out.depth = p.origin.w / depth;
    }
    var col = vec3<f32>(0.0);
    var trans = 1.0;
    var fog_at = vec3<f32>(0.0);
    var fog_w = 0.0;
    var fl_w = 0.0;
    var fl_s = 0.0;
    var fl_yr = 0.0;

    // the plume's box
    let safe = select(dir, vec3<f32>(1e-6), abs(dir) < vec3<f32>(1e-6));
    let inv = 1.0 / safe;
    let a = (p.lo.xyz - eye) * inv;
    let b = (p.hi.xyz - eye) * inv;
    let tmin = min(a, b);
    let tmax = max(a, b);
    let t0 = max(max(tmin.x, tmin.y), max(tmin.z, 0.0));
    let t1 = min(min(min(tmax.x, tmax.y), tmax.z), t_scene);
    if t1 > t0 {
        var sun_dir = vec3<f32>(0.0, 1.0, 0.0);
        var sun_col = vec3<f32>(0.0);
        if lights.n_directional_lights > 0u {
            let light = lights.directional_lights[0];
            sun_dir = light.direction_to_light;
            sun_col = light.color.rgb * view.exposure * 0.3183;
        }
        let ambient = lights.ambient_color.rgb * view.exposure;
        // forward scattering: the plume glows at its rim against the sun
        let mu = dot(dir, sun_dir);
        let phase = 0.7 + 0.9 * pow(max(mu, 0.0), 6.0);
        let brown = vec3<f32>(0.17, 0.14, 0.11);
        let grey = vec3<f32>(0.52, 0.52, 0.53);
        let glow_col = vec3<f32>(1.4, 0.48, 0.10);
        let sigma = p.smoke.x;
        let zone = p.hi.w;
        let t = globals.time;
        let fperiod = p.look.w;
        let len = t1 - t0;
        let coarse0 = max(len / (p.lo.w * 0.7), 4.0);
        var s = t0;
        var first = true;
        let max_steps = i32(p.lo.w);
        for (var i = 0; i < max_steps; i++) {
            if s >= t1 || trans < 0.02 {
                break;
            }
            let pos = eye + dir * s;
            let here = plume(pos.xz, 0.0);
            let h = pos.y - here.a;
            // steps: coarse through the smoke, fine through the flames,
            // never jumping over the flames' zone on the way down
            let coarse = clamp(max(coarse0, s * 0.02), 4.0, 90.0);
            // the flames grow toward the toy scale with distance, as the
            // tongues and the houses do: a true flame from the play camera
            // is a few pixels (`VolumeSettings::flame_toy`)
            let grow = 1.0 + (p.more.x - 1.0) * smoothstep(p.flame.z, p.flame.w, s);
            let fine = clamp(s * 0.005, 0.8, 6.0) * (0.5 + 0.5 * grow);
            let flames_on = 1.0;
            var ds = coarse;
            var in_flames = false;
            if zone > 0.0 && flames_on > 0.0 && h < zone * grow {
                // the tallest flame round here: the cell's own, or three
                // times the mean over 80 m (the mip averages the band with
                // the ground beside it)
                let cap = max(here.b, plume(pos.xz, 2.0).b * 3.0) * grow * p.look2.y + 2.0;
                if h < cap && plume(pos.xz, 2.0).g > 0.0005 {
                    in_flames = true;
                    // fine steps only over a cell alight (and its edge),
                    // half a cell elsewhere in the band's reach
                    ds = select(max(fine, p.map.z * 0.5), fine, plume(pos.xz, 0.0).g > 0.005);
                } else if dir.y < 0.0 {
                    ds = clamp((h - cap) / -dir.y, fine, coarse);
                }
            } else if zone > 0.0 && flames_on > 0.0 && dir.y < 0.0 {
                ds = min(coarse, max((h - zone * grow) / -dir.y, fine));
            }
            if first {
                // a jittered start: no bands
                first = false;
                s += ds * jitter(in.position.xy);
                continue;
            }
            // metres a march pixel spans here
            let foot = s * p.frame.y;

            // the flames
            // the band read through a wobble of a cell or so: no squares
            var fire_here = here;
            if in_flames {
                let wob = noise(vec3<f32>(pos.xz / 90.0, 0.71), 0.0).rg - 0.5;
                fire_here = plume(pos.xz + wob * p.map.z * 2.2, 0.0);
            }
            let show = u32(p.frame.z);
            if in_flames && fire_here.g > 0.02 && (show & 1u) != 0u {
                let here = fire_here;
                let clumps = noise(vec3<f32>(pos.xz * 0.028, 0.37), 0.0).b;
                let clump = smoothstep(0.3, 0.75, clumps);
                let band = here.g * (0.25 + 0.75 * clump);
                let hf = max(here.b * grow * p.look2.y * (0.45 + 0.35 * clump), max(0.5, foot * 3.0));
                // under the coarse ground: the real ground is drawn, the
                // depth stops the ray there
                let yr = max(h, 0.0) / hf;
                if yr < 1.0 && h > -8.0 {
                    // a tongue as tall as the flame and a third as wide
                    // (`lingue` scales the width), never under a few march
                    // pixels: a fixed size cut the flame into slabs where
                    // it was lower than the pattern
                    let wide = max(hf * 0.32 * fperiod / 14.0, foot * 3.0);
                    // the noise has 48 texels a period
                    let flod = log2(max(foot * 48.0 / wide, 1.0));
                    // rising a flame's height in about a second
                    let tq = t * 1.1;
                    let q = vec3<f32>(pos.x / wide, yr * 0.9 - tq, pos.z / wide);
                    let w = noise(q * 0.37 + vec3<f32>(0.0, -tq * 0.3, 0.0), max(flod - 1.4, 0.0)).gb - 0.5;
                    var n = noise(q + vec3<f32>(w.x, 0.0, w.y) * 0.9, flod).r;
                    // a finer flicker, where it is more than a pixel
                    let near_w = 1.0 - smoothstep(0.12, 0.35, foot / wide);
                    if near_w > 0.0 {
                        let fine_n = noise(q * vec3<f32>(2.9, 2.2, 2.9) + vec3<f32>(w.y, -tq, w.x) * 1.4, flod + 1.5).g;
                        n = mix(n, 0.62 * n + 0.38 * fine_n * 1.15, near_w);
                    }
                    let thresh = 0.33 + 0.26 * yr + 0.20 * (1.0 - band);
                    let dens = smoothstep(thresh, thresh + p.look2.x, n) * (1.0 - yr * yr * yr) * flames_on;
                    if dens > 0.002 {
                        let temp = clamp((1.0 - 0.75 * yr) * (0.25 + 0.55 * dens) * p.look2.z, 0.0, 1.0);
                        let absorbed = 1.0 - exp(-dens * ds * p.flame.y);
                        col += flame_ramp(temp) * p.flame.x * absorbed * trans;
                        fog_at += pos * absorbed * trans;
                        fog_w += absorbed * trans;
                        fl_w += absorbed * trans;
                        fl_s += s * absorbed * trans;
                        fl_yr += yr * absorbed * trans;
                        trans *= 1.0 - absorbed;
                    }
                }
            }

            // the smoke
            // the billows averaged over what one sample stands for: the
            // pixel's width and part of the stride (the jitter and the
            // history average the rest); sampled finer, the smoke sparkles
            // as the camera moves
            let slod = log2(max(max(foot, ds * 0.35) * 48.0 / p.smoke.y, 1.0));
            let dn = smoke_at(pos, h, true, slod);
            if (show & 2u) == 0u {
                // no smoke drawn: only its strides
            } else if dn.x < 0.0 && !in_flames {
                // empty: a longer stride
                ds *= 1.8;
            } else if dn.x > 0.002 {
                let k = clamp(h / p.wind.w, 0.0, 1.0);
                // two taps toward the sun: the plume shades itself
                let d1 = max(smoke_at(pos + sun_dir * 35.0, h + sun_dir.y * 35.0, false, 0.0).x, 0.0);
                let d2 = max(smoke_at(pos + sun_dir * 110.0, h + sun_dir.y * 110.0, false, 0.0).x, 0.0);
                let shade = exp(-sigma * (d1 * 80.0 + d2 * 130.0) * p.look.z);
                let albedo = mix(brown, grey, smoothstep(0.0, 0.5, k));
                let lit = albedo * (ambient * (0.45 + 0.4 * k) * (0.5 + 0.5 * shade) + sun_col * shade * phase * 0.9);
                let fire_lit = glow_col * dn.y * 0.9;
                // thinned round the eye
                let clear = p.frame.w;
                let near_k = select(1.0, smoothstep(clear * 0.25, clear, s), clear > 1.0);
                let absorbed = 1.0 - exp(-dn.x * near_k * ds * sigma);
                col += (lit + fire_lit) * p.smoke.w * absorbed * trans;
                fog_at += pos * absorbed * trans;
                fog_w += absorbed * trans;
                trans *= 1.0 - absorbed;
            }
            s += ds;
        }
    }
    let alpha = 1.0 - trans;
    var now = vec4<f32>(0.0);
    var mid = hit;
    if alpha >= 0.002 {
        // the scene's fog, at the volume's weighted middle
        mid = fog_at / max(fog_w, 1e-4);
        let fogged = apply_scene_fog(vec4<f32>(col / alpha, alpha), mid);
        now = vec4<f32>(fogged.rgb * alpha, alpha);
    }

    out.color = now;
    let fl_d = select(0.0, fl_s / max(fl_w, 1e-4), fl_w > 1e-4);
    out.mid = vec4<f32>(min(distance(mid, eye), 1.0e6), fl_w, fl_d, select(0.0, fl_yr / max(fl_w, 1e-4), fl_w > 1e-4));
    return out;
}
