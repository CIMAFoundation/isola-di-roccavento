// The sky dome: a large inverted sphere around the whole scenario, shaded
// purely from view direction, not from geometry. `crate::sky::update_sky`
// recomputes the four colours below from the sun's actual elevation, so this
// shader only has to blend them and paint the disc.
//
// What moves every frame is not in the material (2026-11-01: a material
// changed is a new GPU buffer on WebGL2): the sun's direction is the scene's
// directional light, the moon is opposite it, and the clouds drift from
// `sky.cloud.xy` at the velocity in `cloud_lit.a`, `cloud_shade.a` since the
// time in `cloud.w` (`globals.time`).

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::{view, lights, globals};

struct SkyUniform {
    sun_dir: vec4<f32>,      // xyz: direction to the sun. w: day gate, 0..1.
    sun_color: vec4<f32>,
    zenith_color: vec4<f32>,
    horizon_color: vec4<f32>,
    moon_dir: vec4<f32>,     // xyz: direction to the moon. w: how high it is, 0..1.
    moon_color: vec4<f32>,
    cloud: vec4<f32>,        // xy: downwind travel, metres (XZ), at time w. z: coverage.
    cloud_lit: vec4<f32>,    // a: drift velocity along x, m/s
    cloud_shade: vec4<f32>,  // a: drift velocity along z, m/s
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: SkyUniform;

// Direction to the sun: the sun light's, as the scene is lit this frame.
fn sun_direction() -> vec3<f32> {
    if (lights.n_directional_lights > 0u) {
        return normalize(lights.directional_lights[0].direction_to_light);
    }
    return normalize(sky.sun_dir.xyz);
}

fn cloud_drift() -> vec2<f32> {
    return sky.cloud.xy + vec2<f32>(sky.cloud_lit.a, sky.cloud_shade.a) * (globals.time - sky.cloud.w);
}

// Volumetric clouds: a slab between CLOUD_BOTTOM and CLOUD_TOP, ray-marched
// through a 3D noise density with a short march toward the sun for the
// self-shadowing. The slab is carried downwind by `sky.cloud.xy`.
const CLOUD_BOTTOM: f32 = 2300.0;
const CLOUD_TOP: f32 = 3800.0;
const CLOUD_SCALE: f32 = 2200.0;
const CLOUD_STEPS: i32 = 28;
const CLOUD_SIGMA: f32 = 0.0022;

fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn vnoise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x0 = mix(mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
                 mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let x1 = mix(mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
                 mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(x0, x1, u.z);
}

fn fbm3(p_in: vec3<f32>, octaves: i32) -> f32 {
    var p = p_in;
    var a = 0.5;
    var s = 0.0;
    for (var k = 0; k < octaves; k++) {
        s += a * vnoise3(p);
        p = p * 2.03 + vec3<f32>(17.1, 9.7, 5.3);
        a *= 0.5;
    }
    return s;
}

fn cloud_density(p: vec3<f32>, octaves: i32) -> f32 {
    let h = clamp((p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM), 0.0, 1.0);
    // flat bases, rounded tops
    let profile = smoothstep(0.0, 0.12, h) * smoothstep(1.0, 0.55, h);
    let drift = cloud_drift();
    let q = vec3<f32>(p.x - drift.x, p.y * 1.6, p.z - drift.y) / CLOUD_SCALE;
    let n = fbm3(q, octaves);
    let thr = mix(0.78, 0.38, sky.cloud.z);
    return smoothstep(thr, thr + 0.2, n) * profile;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let dir = normalize(in.world_position.xyz - view.world_position);

    // Below the horizon the dome still has to paint *something*: fade to a
    // darker version of the horizon colour rather than showing the zenith
    // gradient upside down.
    let up = clamp(dir.y, -1.0, 1.0);
    let sky_t = clamp(up, 0.0, 1.0);
    var col = mix(sky.horizon_color.rgb, sky.zenith_color.rgb, pow(sky_t, 0.45));
    let below_t = clamp(-up, 0.0, 1.0);
    col = mix(col, sky.horizon_color.rgb * 0.55, below_t * 0.8);

    let sun_dir = sun_direction();
    let sun_amt = max(dot(dir, sun_dir), 0.0);
    // A tight disc plus a broad, dim glow around it: bloom picks the disc up,
    // the glow reads even where bloom would otherwise clip it away.
    let disc = smoothstep(0.9994, 0.9999, sun_amt);
    let glow = pow(sun_amt, 200.0) * 0.6 + pow(sun_amt, 8.0) * 0.18;
    col += sky.sun_color.rgb * (disc * 8.0 + glow) * (0.15 + 0.85 * sky.sun_dir.w);

    // The moon: always full (see `crate::sky` for why), so a plain disc plus
    // a soft glow is enough — no terminator, no phase shading. Gated purely
    // by how high it is, unlike the sun's floor, so it is not a ghost in the
    // daytime sky when it sits opposite a sun that is still up.
    // exactly opposite the sun (`crate::sky`)
    let moon_dir = -sun_dir;
    let moon_amt = max(dot(dir, moon_dir), 0.0);
    let moon_disc = smoothstep(0.9985, 0.9996, moon_amt);
    let moon_glow = pow(moon_amt, 400.0) * 0.4 + pow(moon_amt, 6.0) * 0.05;
    col += sky.moon_color.rgb * (moon_disc * 3.0 + moon_glow) * sky.moon_dir.w;

    // Clouds
    if (dir.y > 0.03) {
        let o = view.world_position;
        let t0 = max((CLOUD_BOTTOM - o.y) / dir.y, 0.0);
        let t1 = min((CLOUD_TOP - o.y) / dir.y, t0 + 5500.0);
        if (t1 > t0) {
            let dt = (t1 - t0) / f32(CLOUD_STEPS);
            let jitter = hash3(vec3<f32>(in.position.xy, 1.0));
            let sun_d = sun_dir;
            var trans = 1.0;
            var light = vec3<f32>(0.0);
            for (var i = 0; i < CLOUD_STEPS; i++) {
                let p = o + dir * (t0 + (f32(i) + jitter) * dt);
                let d = cloud_density(p, 4);
                if (d > 0.01) {
                    var od = 0.0;
                    for (var j = 1; j <= 3; j++) {
                        od += cloud_density(p + sun_d * (f32(j) * 260.0), 2);
                    }
                    let beam = exp(-od * 260.0 * CLOUD_SIGMA * 1.3);
                    let c = sky.cloud_shade.rgb + sky.cloud_lit.rgb * beam * 0.85;
                    let ext = d * dt * CLOUD_SIGMA;
                    let step_t = exp(-ext);
                    light += trans * (1.0 - step_t) * c;
                    trans *= step_t;
                    if (trans < 0.03) { break; }
                }
            }
            let fade = smoothstep(0.03, 0.2, dir.y);
            let alpha = (1.0 - trans) * fade;
            col = mix(col, light / max(1.0 - trans, 1e-3), alpha);
        }
    }

    return vec4<f32>(col, 1.0);
}
