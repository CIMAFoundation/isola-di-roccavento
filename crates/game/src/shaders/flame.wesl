// Fragment shader for flame billboards.
//
// The CPU side (fire_view::update_flames) already decides *where* a tongue
// stands, how tall it is and how hot: this shader is only responsible for
// making one quad look like fire instead of a lit sprite. It domain-warps
// the texture lookup with a small animated noise field and adds a fast
// per-instance flicker, both driven by `globals.time` and the billboard's
// own world position (so tongues a metre apart do not flicker in lockstep).

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::globals;
import game::shaders::noise::{hash21, value_noise};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var base_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var base_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> mode: vec4<f32>;

// Blackbody-ish ramp: deep red tip -> orange -> yellow -> white-hot core.
fn flame_ramp(h: f32) -> vec3<f32> {
    // 2026-11-04: deeper orange, a smaller white core: in daylight a
    // wildfire's flames read orange-yellow, and the white sheets washed out
    // against the realistic plants
    let c0 = vec3<f32>(0.85, 0.20, 0.025);
    let c1 = vec3<f32>(2.3, 0.48, 0.04);
    let c2 = vec3<f32>(3.2, 1.35, 0.16);
    let c3 = vec3<f32>(3.8, 2.5, 0.9);
    let a = clamp(h * 3.0, 0.0, 1.0);
    let b = clamp(h * 3.0 - 1.0, 0.0, 1.0);
    let c = clamp(h * 3.0 - 2.0, 0.0, 1.0);
    return mix(mix(mix(c0, c1, a), c2, b), c3, c);
}

// Glowing ember temperature, h = 1 just thrown ... 0 spent: white-yellow,
// orange, dull red, then a dark cinder.
fn ember_ramp(h: f32) -> vec3<f32> {
    let spent = vec3<f32>(0.22, 0.025, 0.0);
    let red = vec3<f32>(1.5, 0.20, 0.02);
    let orange = vec3<f32>(3.0, 1.05, 0.12);
    let white = vec3<f32>(4.2, 2.9, 1.3);
    let a = smoothstep(0.0, 0.30, h);
    let b = smoothstep(0.30, 0.65, h);
    let c = smoothstep(0.65, 1.0, h);
    return mix(mix(mix(spent, red, a), orange, b), white, c);
}

// An ember or a landing flash (`fire_view::update_flames`): an oval quad
// (uv 0..1) with the round shape drawn per pixel. Vertex colour:
// r = share of its flight, g = phase, b = gain (negative: a landing flash).
fn ember(in: VertexOutput, t: f32) -> vec4<f32> {
    let k = clamp(in.color.r, 0.0, 1.0);
    let ph = in.color.g;
    let flash = in.color.b < 0.0;
    let gain = abs(in.color.b);
    let x = in.uv.x * 2.0 - 1.0;
    let y = in.uv.y * 2.0 - 1.0;

    if flash {
        let f2 = (1.0 - clamp(x * x + y * y, 0.0, 1.0));
        let f4 = f2 * f2;
        return vec4<f32>(vec3<f32>(4.0, 2.2, 0.9) * gain * (f4 * f4 + 0.25 * f4), 0.0);
    }

    // A round glowing dot, drawn per pixel: a bright core, a soft body and a
    // halo, all exactly zero before the quad's edge so no square shows.
    let r = clamp(length(vec2<f32>(x, y)), 0.0, 1.0);
    let body = 1.0 - smoothstep(0.45, 0.95, r);
    let core = 1.0 - smoothstep(0.0, 0.40, r);
    let shape = 0.9 * body + 1.2 * core * core + 0.25 * (1.0 - r) * (1.0 - r);

    // It cools over its flight and flickers as it tumbles.
    let heat = 1.0 - pow(k, 0.7);
    let flick = 0.6 + 0.4 * sin(t * (13.0 + ph * 23.0) + ph * 50.0);
    let life = smoothstep(1.0, 0.78, k);
    let col = ember_ramp(heat) * shape * gain * flick * life;
    // Alpha 0: Bevy's Add is premultiplied (src + dst * (1 - alpha)), so any
    // alpha would darken what is behind the dot into a dark rim.
    return vec4<f32>(col, 0.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = globals.time;
    if mode.x > 1.5 {
        return ember(in, t);
    }
    if mode.x < 0.5 {
        let s0 = in.world_position.x * 0.13 + in.world_position.z * 0.071;
        let tx = textureSample(base_texture, base_sampler, in.uv);
        let fl = 0.80 + 0.35 * value_noise(vec2<f32>(t * 11.0 + s0 * 7.0, s0 * 3.0));
        var c0 = in.color * tx * fl;
        c0.a = tx.a * in.color.a;
        return c0;
    }
    // 2026-11-05 (the user: «più realistico», a flipbook): every tongue
    // plays a filmed flame (`textures::flame_flipbook`, 2 sequences of 64
    // frames, CC0 Unity Labs). uv.x carries the tongue's number besides its
    // place across (`QuadBuilder::tongue`: 2 id + 0.5 + u), so neighbours
    // play different sequences, from different frames, at different rates,
    // some mirrored: the front is a wall of flames that do not repeat.
    let id = floor(in.uv.x * 0.5);
    var u = clamp(in.uv.x - 2.0 * id - 0.5, 0.0, 1.0);
    let v = clamp(in.uv.y, 0.0, 1.0);
    let foot = v;
    let up = 1.0 - v;
    let seq = id % 2.0;
    let start = floor(id * 0.5) % 64.0;
    if id >= 128.0 {
        u = 1.0 - u;
    }
    let h = hash21(vec2<f32>(id, 7.0));
    // big flames move slower than a campfire's: about the film's own 24 fps
    // stretched to 14-22
    let f = t * (14.0 + 8.0 * h) + start;
    let w = fract(f);
    let f0 = floor(f) % 64.0;
    let f1 = (f0 + 1.0) % 64.0;
    let cell = vec2<f32>(1.0 / 16.0, 1.0 / 8.0);
    let uv0 = (vec2<f32>(f0 % 16.0, floor(f0 / 16.0) + 4.0 * seq) + vec2<f32>(u, v)) * cell;
    let uv1 = (vec2<f32>(f1 % 16.0, floor(f1 / 16.0) + 4.0 * seq) + vec2<f32>(u, v)) * cell;
    let tx = mix(textureSample(base_texture, base_sampler, uv0), textureSample(base_texture, base_sampler, uv1), w);
    // nothing at the quad's edges, whatever the frame
    let edge = smoothstep(0.0, 0.04, u) * smoothstep(1.0, 0.96, u) * smoothstep(0.0, 0.03, v);

    // in.color carries the CPU's brightness (cell intensity): a gain on
    // the ramp, not the colour itself
    let gain = clamp(in.color.r * 0.30, 0.55, 1.6);
    // alpha carries the foot's fade: 1 on the ground's tongues; on a burning
    // plant's (`fire_view::plant_torches`) 0 at the foot
    let fade = smoothstep(0.0, 0.3, in.color.a);
    // the film's light is the heat: a yellow core at the foot, orange
    // licks, deep red where it thins into the air; the thin tops are soot
    // (the film's core is near white: kept orange-yellow, as a wildfire's
    // flames read in daylight; the white core only in the hottest)
    // the licks keep orange: only the core goes yellow (from pow 1.7, which
    // left the film's licks dark and the flame a ball of light at its foot)
    let heat = clamp(0.08 + 0.55 * tx.r * (0.7 + 0.25 * gain), 0.0, 1.0);
    let cover = clamp(tx.a * 1.3, 0.0, 1.0) * edge * fade;
    let soot = smoothstep(0.55, 0.95, up) * (1.0 - smoothstep(0.0, 0.25, tx.r));
    // dimmer than the noise sheets were: the film has more flame per
    // pixel, and over a daylit slope the bloom washed it to cream
    let fire = flame_ramp(heat) * gain * 0.5;
    let dark = vec3<f32>(0.10, 0.075, 0.06);
    // Bevy's Add is premultiplied (src + dst * (1 - alpha)): the colour
    // carries its own coverage, and the body hides what is behind it
    let col = mix(fire, dark, soot) * cover;
    return vec4<f32>(col, cover * mix(0.7, 0.95, soot));
}
