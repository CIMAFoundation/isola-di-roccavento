// The fire on the ground, drawn by the terrain's own pixel shader
// (`terrain.wesl`), so the scar can never be pierced by the terrain or float
// over it. What the fire did comes from the fire map (`fire_view::
// update_fire_map`, one texel per fire cell, linearly filtered): g how burnt,
// b the arrival time against the moment the map was written, weighted by g,
// a how much was cut by the crews. Sampled at a point shifted by a smooth
// warp, which breaks up the 20 m raster (`field::FireField::warp`).

import bevy_pbr::render::mesh_view_bindings::globals;
import game::shaders::noise::{hash21, value_noise};

// How long ground stays visibly incandescent after the front passes. Much
// shorter than the 20-minute burn-out: past this it is a scar with smoke over
// it, not a glowing surface, or the glow reads as orange fog on the hillside.
// 2026-11-04: from 1100 s, which painted a wide orange band; now the bright
// line is the front's own, embers glint in the char behind it.
const GLOWING_S: f32 = 600.0;

// Fractal Brownian motion, domain-warped: the warp is what turns blobby
// noise into the tendrilled, branching texture of a real fire front.
fn fbm(p_in: vec2<f32>) -> f32 {
    var p = p_in;
    var a = 0.5;
    var s = 0.0;
    for (var i = 0; i < 5; i++) {
        s += a * value_noise(p);
        p = p * 2.07 + vec2<f32>(17.3, 9.1);
        a *= 0.5;
    }
    return s / 0.96875;
}

// The same, without the octaves finer than two pixels (`mpp` metres per
// pixel; `freq` the first octave's, cycles per metre): they would only
// shimmer, and the terrain pays for every hidden fragment as well. A
// dropped octave counts at its mean, so the tone does not shift.
fn fbm_lod(p_in: vec2<f32>, freq: f32, mpp: f32) -> f32 {
    var p = p_in;
    var a = 0.5;
    var s = 0.0;
    var f = freq;
    for (var i = 0; i < 5; i++) {
        if f * 2.0 * mpp < 1.0 {
            s += a * value_noise(p);
        } else {
            s += a * 0.5;
        }
        p = p * 2.07 + vec2<f32>(17.3, 9.1);
        a *= 0.5;
        f *= 2.07;
    }
    return s / 0.96875;
}

fn fire_ramp(h: f32) -> vec3<f32> {
    // ember red -> orange -> yellow -> near white, h in 0..1
    let c0 = vec3<f32>(0.80, 0.08, 0.02);
    let c1 = vec3<f32>(2.50, 0.60, 0.06);
    // 2026-11-04: burning ground glows orange, never white (it drew white
    // patches in the band seen from a hundred metres)
    let c2 = vec3<f32>(2.90, 1.05, 0.12);
    let c3 = vec3<f32>(3.30, 1.65, 0.30);
    let a = clamp(h * 3.0, 0.0, 1.0);
    let b = clamp(h * 3.0 - 1.0, 0.0, 1.0);
    let c = clamp(h * 3.0 - 2.0, 0.0, 1.0);
    return mix(mix(mix(c0, c1, a), c2, b), c3, c);
}


// The colour the fire leaves at this point and how much it covers, before
// the scene's light (the cold scar is lit here; the glow is light itself).
fn fire_colour(p: vec3<f32>, fire: vec4<f32>) -> vec4<f32> {
    let wp = vec2<f32>(p.x, -p.z);
    // Coverage feathered around the half-burnt contour.
    let cover = clamp((fire.g - 0.42) / 0.20, 0.0, 1.0);
    if cover <= 0.0 {
        // Fuel a crew has cleared: bare earth, with the grain of the ash so
        // the strip does not read as a painted road.
        let bare = clamp((fire.a - 0.42) / 0.20, 0.0, 1.0);
        let grain = 0.8 + 0.4 * (0.6 * value_noise(wp / 17.0) + 0.4 * value_noise(wp / 6.0 + 3.0));
        return vec4<f32>(vec3<f32>(0.36, 0.28, 0.19) * grain, bare * 0.95);
    }
    // the interpolated arrival, unburnt corners dragging it later (`field::arrival`)
    let age = max(-fire.b / max(fire.g, 1e-3) - (1.0 - fire.g) * 90.0, 0.0);
    // Incandescent only just behind the front, then straight to scar.
    let glow = pow(clamp(1.0 - age / GLOWING_S, 0.0, 1.0), 3.0);
    let embers = clamp(1.0 - age / (GLOWING_S * 4.0), 0.0, 1.0);
    // Char and ash at a scale the fire model has no opinion about.
    let ash = 0.72 + 0.85 * (0.6 * value_noise(wp / 23.0 + 7.0) + 0.4 * value_noise(wp / 8.0 + 13.0));
    return vec4<f32>(
        (0.085 + 0.10 * embers) * ash + 2.4 * glow,
        0.065 * ash + 0.70 * glow * glow,
        0.060 * ash + 0.06 * glow,
        cover * 0.95,
    );
}

// What the fire puts on the ground at `p`, in two parts the terrain applies
// at different moments: `scar`, the cold scar (colour as albedo, opacity),
// mixed into the ground's colour before the light, so the scene's light and
// shadows fall on it once, as on the ground; `glow`, the burning band
// (colour as light, opacity), mixed over the lit ground. `bump` roughens the
// normal where the scar is, so the ash catches the sun grain by grain.
struct FireGround {
    scar: vec4<f32>,
    glow: vec4<f32>,
    bump: vec3<f32>,
};

// `road_d`: distance to the drivable roads and the tracks (`ground_mask`);
// `mpp` metres per pixel.
fn fire_ground(p: vec3<f32>, fire: vec4<f32>, road_d: vec2<f32>, mpp: f32) -> FireGround {
    var out: FireGround;
    out.scar = vec4<f32>(0.0);
    out.glow = vec4<f32>(0.0);
    out.bump = vec3<f32>(0.0);
    // nothing burnt or cut near this point: the common case, cheap
    if max(fire.g, fire.a) < 0.3 {
        return out;
    }
    var col = fire_colour(p, fire);
    if col.a <= 0.004 {
        return out;
    }
    let wpc = p.xz;

    // The burning band: `w` fades the fractal treatment in smoothly over the
    // glow gradient so there is no seam where the colour crosses the hot
    // threshold.
    let w = smoothstep(0.35, 1.0, col.r);
    if w > 0.0 {
        let t = globals.time;
        // Heat decoded from the glow (r = char + 2.4 * glow).
        let heat = clamp((col.r - 0.3) / 2.4, 0.0, 1.0);
        // Domain warp, slowly advected by the wind-ish drift.
        let q = wpc * 0.05 + vec2<f32>(t * 0.35, -t * 0.28);
        let warp = vec2<f32>(fbm(q + 3.7), fbm(q + 11.2)) - 0.5;
        let pw = wpc * 0.11 + warp * 2.6;
        let n = fbm(pw + vec2<f32>(t * 0.6, -t * 0.45));
        // Fine, fast crackle on top: individual embers winking.
        let crackle = value_noise(wpc * 0.9 + vec2<f32>(t * 3.0, t * 2.1));
        // Hot cells are where local heat beats the noise threshold: the front
        // breaks into patches and fingers. Slow breathing of the whole front,
        // offset by position so it rolls along the line.
        let breath = 0.5 + 0.5 * sin(t * 2.2 + wpc.x * 0.035 + wpc.y * 0.027);
        // (2026-11-04: patchier, from heat * 1.75 - (1 - n) * 0.70: away
        // from the front itself the band is char with glowing patches,
        // not an orange carpet)
        let local = heat * 1.5 - (1.0 - n) * 0.95 + (crackle - 0.5) * 0.40 + breath * 0.10;
        let hot = smoothstep(0.05, 0.25, local);
        // (2026-11-04: from 0.85 + 0.55: the hottest patches burnt out to white)
        let flame = fire_ramp(clamp(local, 0.0, 1.0)) * (0.7 + 0.4 * crackle);
        // The cooler gaps: dark char with a faint red glint.
        let char_col = vec3<f32>(0.06, 0.03, 0.025) + vec3<f32>(0.9, 0.12, 0.0) * (1.0 - hot) * heat * n;
        let rgb = mix(col.rgb, mix(char_col, flame, hot), w);
        // Ragged outer edge.
        let edge = smoothstep(0.12, 0.5, col.a + (n - 0.5) * 0.55);
        col = vec4<f32>(rgb, mix(col.a, col.a * edge, w));
    }

    // The cold scar: ragged edge and ash grain at pixel scale, and the roads
    // showing through it. The fire map is 20 m: without this the border is a
    // soft blob and roads vanish under it.
    let cold = 1.0 - w;
    var scar = col.rgb;
    var scar_a = col.a;
    if cold > 0.0 {
        // Fractal border: a domain-warped fbm (large lobes down to ~1 m
        // fingers) moves the coverage threshold, so the burn edge is a
        // self-similar coastline with peninsulas and unburned islands. The
        // push is strongest in the border band and fades inside the scar.
        let warp = vec2<f32>(value_noise(wpc * 0.06 + 7.1), value_noise(wpc * 0.06 + 23.9)) - 0.5;
        let fr = fbm_lod(wpc * 0.13 + warp * 3.2, 0.13, mpp);
        let near = 1.0 - smoothstep(0.25, 0.6, mpp);
        var fine = 0.5;
        var grit = 0.5;
        if near > 0.0 {
            fine = mix(0.5, value_noise(wpc * 1.9 + 5.3), near);
            grit = mix(0.5, value_noise(wpc * 4.3) * 0.6 + value_noise(wpc * 11.0 + 2.0) * 0.4, near);
        }
        let n2 = 0.75 * fr + 0.25 * fine;
        let band = 1.0 - smoothstep(0.35, 0.95, col.a);
        let m = col.a + (n2 - 0.5) * (0.55 + 0.85 * band);
        let ragged = smoothstep(0.22, 0.34, m);
        // Scorched fringe: browned, half-burnt vegetation just inside the
        // border, darkening to ash towards the interior.
        let fringe = smoothstep(0.22, 0.30, m) * (1.0 - smoothstep(0.30, 0.62, m));
        let scorch = mix(vec3<f32>(1.0), vec3<f32>(1.9, 1.15, 0.55), fringe * 0.7);
        // Burnt ground as a material, not a flat tint: `fire_colour` is a
        // near-neutral dark; here it is pushed between warm brown soil, cool
        // grey ash and near-black charcoal by noise at three scales, then
        // broken up by fine grit and hairline cracks.
        let big = fbm_lod(wpc * 0.035 + 31.0, 0.035, mpp);
        // 2026-11-04: no warp on the middle scale (it drew swirls that read
        // as wet asphalt from the play camera) and a warm grey, not a blue
        // one: a burnt Mediterranean slope is charcoal and brown soil with
        // pale ash where the shrubs stood
        let mid = fbm_lod(wpc * 0.16 + 9.0, 0.16, mpp);
        let lum = dot(col.rgb, vec3<f32>(0.33, 0.45, 0.22));
        let brown = vec3<f32>(1.35, 0.92, 0.62);
        let grey = vec3<f32>(1.0, 0.96, 0.92);
        let hue = smoothstep(0.30, 0.70, big * 0.65 + mid * 0.35);
        let tone = mix(brown, grey, hue);
        // Ash drifts: pale grey patches, charcoal pockets, both patchy.
        let ash = smoothstep(0.60, 0.84, mid) * (1.0 - fringe) * (0.5 + 0.5 * smoothstep(0.4, 0.7, grit));
        let coal = smoothstep(0.62, 0.20, mid * 0.7 + grit * 0.3);
        var base = vec3<f32>(lum) * tone * (0.9 + 0.7 * (mid - 0.5));
        base = mix(base, vec3<f32>(0.27, 0.25, 0.23) * (0.8 + 0.4 * grit), ash * 0.6);
        base = mix(base, base * 0.35, coal * 0.6);
        // Hairline cracks and pits: dark ridges where two noise fields cross.
        let cr = abs(value_noise(wpc * 0.9 + warp * 6.0) - 0.5);
        let crack = (1.0 - smoothstep(0.0, 0.035, cr)) * (1.0 - smoothstep(0.6, 1.5, mpp));
        // faint: strong, they drew contour lines over the slope once the
        // grass on it was burnt away (2026-11-04)
        base *= 1.0 - 0.2 * crack;
        // Matte, gritty finish: high-frequency speckle, no uniform sheen.
        base *= 0.82 + 0.36 * grit;
        scar = base * scorch;
        scar_a = col.a * ragged;
        // Roads show through the scar.
        let aa = max(0.5 * mpp, 0.25);
        let on_road = max(
            1.0 - smoothstep(max(0.0, 0.8 * mpp - 4.8) - aa, max(0.0, 0.8 * mpp - 4.8) + aa, road_d.x),
            1.0 - smoothstep(max(0.0, 0.8 * mpp - 1.6) - aa, max(0.0, 0.8 * mpp - 1.6) + aa, road_d.y),
        );
        scar_a *= 1.0 - 0.72 * on_road;
        // The ash's bump, only where a pixel is small enough to show it.
        if near > 0.0 {
            let e = 0.35;
            let h0 = value_noise(wpc * 1.3) + 0.5 * value_noise(wpc * 3.1);
            let hx = value_noise((wpc + vec2<f32>(e, 0.0)) * 1.3) + 0.5 * value_noise((wpc + vec2<f32>(e, 0.0)) * 3.1);
            let hz = value_noise((wpc + vec2<f32>(0.0, e)) * 1.3) + 0.5 * value_noise((wpc + vec2<f32>(0.0, e)) * 3.1);
            out.bump = vec3<f32>(h0 - hx, 0.0, h0 - hz) * 0.9 * near * scar_a;
        }
    }
    out.scar = vec4<f32>(scar * SCAR_ALBEDO, scar_a * cold);
    out.glow = vec4<f32>(col.rgb, col.a * w);
    return out;
}

// The scar's colour as the ground's albedo: measured so the scar has the
// tone of the overlay it replaces, which was lit separately at 0.55 of the
// scene's light (mean RGB over the scar's pixels within 3 levels, from afar
// and up close).
const SCAR_ALBEDO: f32 = 0.5;
