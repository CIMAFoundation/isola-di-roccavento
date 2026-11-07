// The flames' fluid (`fire_fluid.rs`, which explains it): a grid of voxels
// round the camera's focus, following the ground, stepped each frame by
// semi-Lagrangian advection, buoyancy, the wind, vorticity confinement and
// a pressure projection (Stam's «stable fluids», with Fedkiw's
// confinement). One voxel holds the velocity (m/s; x east, y up, z the
// Bevy z) and the heat (0 cold, 1 the flame's root).
//
// The grid is fixed to the world and wraps (a voxel of world index `w` is
// stored at `w mod N`): when the focus moves, only the voxels that come in
// start anew, and the march samples it with a repeating sampler.
//
// Plain WGSL, no imports: a test validates it with naga
// (`fire_fluid::tests`).

struct Fluid {
    // the grid's first voxel, world index (x, z); then the voxels a side
    // and up
    origin: vec4<i32>,
    // last frame's first voxel (x, z)
    prev: vec4<i32>,
    // dx, dz (m), dt (s), the time (s)
    step: vec4<f32>,
    // the wind (Bevy x, z, m/s); buoyancy (m/s² at heat 1); the wind's hold (1/s)
    wind: vec4<f32>,
    // confinement, the rise's drag (1/s), the source's height (m), the
    // source's heat
    look: vec4<f32>,
    // the fire map: 1 / world width, 1 / world height, cell (m); the
    // drawing clock less the front map's base (s)
    map: vec4<f32>,
    // seconds a cell flames, the flames' height factor, the heat at the
    // flame's tip, the speed (m/s) the flame's top is reckoned at
    flame: vec4<f32>,
}

@group(0) @binding(0) var<uniform> f: Fluid;
@group(0) @binding(1) var tex_a: texture_3d<f32>;
@group(0) @binding(2) var tex_b: texture_3d<f32>;
@group(0) @binding(3) var out_vec: texture_storage_3d<rgba16float, write>;
@group(0) @binding(4) var out_scalar: texture_storage_3d<r32float, write>;
@group(0) @binding(5) var front_tex: texture_2d<f32>;
@group(0) @binding(6) var plume_tex: texture_2d<f32>;

fn n_side() -> i32 {
    return f.origin.z;
}

fn n_up() -> i32 {
    return f.origin.w;
}

// Where a voxel of world index (x, z) and level k is stored.
fn slot(w: vec2<i32>, k: i32) -> vec3<i32> {
    let n = n_side();
    return vec3<i32>(((w.x % n) + n) % n, clamp(k, 0, n_up() - 1), ((w.y % n) + n) % n);
}

fn inside(w: vec2<i32>, origin: vec2<i32>) -> bool {
    let d = w - origin;
    return all(d >= vec2<i32>(0)) && all(d < vec2<i32>(n_side()));
}

// The world index of the voxel this invocation writes.
fn world_of(id: vec3<u32>) -> vec2<i32> {
    return f.origin.xy + vec2<i32>(i32(id.x), i32(id.z));
}

fn ambient() -> vec4<f32> {
    return vec4<f32>(f.wind.x, 0.0, f.wind.y, 0.0);
}

// The state at world index (x, z), level k, as last stepped: the air the
// wind brings where the grid did not reach last frame.
fn state_at(w: vec2<i32>, k: i32) -> vec4<f32> {
    if !inside(w, f.origin.xy) {
        return ambient();
    }
    return textureLoad(tex_a, slot(w, k), 0);
}

fn ok(id: vec3<u32>) -> bool {
    return i32(id.x) < n_side() && i32(id.y) < n_up() && i32(id.z) < n_side();
}

fn hash(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

// Smooth value noise in [0, 1].
fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let u = fract(p);
    let s = u * u * (3.0 - 2.0 * u);
    let a = mix(mix(hash(i), hash(i + vec3<f32>(1.0, 0.0, 0.0)), s.x), mix(hash(i + vec3<f32>(0.0, 1.0, 0.0)), hash(i + vec3<f32>(1.0, 1.0, 0.0)), s.x), s.y);
    let b = mix(mix(hash(i + vec3<f32>(0.0, 0.0, 1.0)), hash(i + vec3<f32>(1.0, 0.0, 1.0)), s.x), mix(hash(i + vec3<f32>(0.0, 1.0, 1.0)), hash(i + vec3<f32>(1.0, 1.0, 1.0)), s.x), s.y);
    return mix(a, b, s.z);
}

// A fire map texel (row 0 the south edge) at a point (Bevy x, z).
fn map_texel(xz: vec2<f32>, dims: vec2<i32>) -> vec2<i32> {
    let uv = vec2<f32>(xz.x * f.map.x, -xz.y * f.map.y);
    return clamp(vec2<i32>(floor(uv * vec2<f32>(dims))), vec2<i32>(0), dims - 1);
}

// How much heat the ground gives the fluid at a point (0 to 1), from the
// front map on the drawing clock (`fire_ahead`): most at the front's edge.
fn band_at(xz: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(front_tex));
    let uv = vec2<f32>(xz.x * f.map.x, -xz.y * f.map.y) * vec2<f32>(dims) - 0.5;
    let c0 = vec2<i32>(floor(uv));
    let fr = uv - floor(uv);
    let clock = f.map.w;
    var band = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let wk = mix(1.0 - fr.x, fr.x, f32(o.x)) * mix(1.0 - fr.y, fr.y, f32(o.y));
        let l = textureLoad(front_tex, clamp(c0 + o, vec2<i32>(0), dims - 1), 0);
        let v = l.g + l.b * clock;
        let s = select(clamp(v, l.a, 1.0), clamp(v, 0.0, l.a), l.b >= 0.0);
        let flaming = clamp(1.0 - max(clock - l.r, 0.0) / f.flame.x, 0.0, 1.0);
        let edge = flaming * flaming * flaming;
        // the fluid's source is the front's edge, the ground just lit: the
        // whole band as a source is a sheet too wide for the air to rise
        // through (the band behind it keeps the tongues of before)
        band += wk * s * smoothstep(0.05, 0.3, flaming) * (0.15 + 0.85 * edge);
    }
    return band;
}

// The flame's height there (m), from the plume map (b).
fn flame_h(xz: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(plume_tex));
    return textureLoad(plume_tex, map_texel(xz, dims), 0).b * f.flame.y;
}

// --- 1. advection, forces, sources ----------------------------------------

// The state at a point in voxel coordinates (world index units, level),
// trilinear over the eight voxels round it.
fn sample_state(q: vec3<f32>) -> vec4<f32> {
    let b = floor(q);
    let t = q - b;
    let w0 = vec2<i32>(i32(b.x), i32(b.z));
    let k0 = i32(b.y);
    var acc = vec4<f32>(0.0);
    for (var c = 0; c < 8; c++) {
        let o = vec3<i32>(c & 1, (c >> 1) & 1, c >> 2);
        let wt = mix(1.0 - t.x, t.x, f32(o.x)) * mix(1.0 - t.y, t.y, f32(o.y)) * mix(1.0 - t.z, t.z, f32(o.z));
        acc += wt * state_at(w0 + vec2<i32>(o.x, o.z), clamp(k0 + o.y, 0, n_up() - 1));
    }
    return acc;
}

@compute @workgroup_size(4, 4, 4)
fn advect(@builtin(global_invocation_id) id: vec3<u32>) {
    if !ok(id) {
        return;
    }
    let w = world_of(id);
    let k = i32(id.y);
    let dx = f.step.x;
    let dz = f.step.y;
    let dt = f.step.z;
    var s = ambient();
    // came in this frame: nothing to carry
    if inside(w, f.prev.xy) {
        let here = textureLoad(tex_a, slot(w, k), 0);
        let back = vec3<f32>(f32(w.x), f32(k), f32(w.y)) - dt * vec3<f32>(here.x / dx, here.y / dz, here.z / dx);
        s = sample_state(vec3<f32>(back.x, clamp(back.y, 0.0, f32(n_up() - 1)), back.z));
    }
    let pos = vec2<f32>(f32(w.x) * dx, f32(w.y) * dx);
    let h = f32(k) * dz;
    // the wind holds the air, less inside the flames' rise
    let hold = 1.0 - exp(-dt * f.wind.w * (1.0 - 0.6 * clamp(s.w * 2.0, 0.0, 1.0)));
    s.x += (f.wind.x - s.x) * hold;
    s.z += (f.wind.y - s.z) * hold;
    // buoyancy, and the drag that keeps the rise to flame speeds
    s.y += dt * f.wind.z * s.w;
    s.y *= exp(-dt * f.look.y);
    // cools so that the flame's tip, rising at about `flame.w`, is at the
    // flame's height of the fire under it
    let hf = max(flame_h(pos), 6.0);
    let cool = log(1.0 / f.flame.z) * f.flame.w / hf;
    s.w *= exp(-dt * cool);
    // The burning ground breathes heat in, in patches a few metres across
    // that come and go. Heated evenly, a band cannot rise: an
    // incompressible fluid's pressure takes off any lift the same
    // everywhere (it is a gradient), and the first version stood as a slab
    // as tall as its source, still. Patches rise as plumes, the air
    // between them drawn in at their feet.
    if h < f.look.z {
        let band = band_at(pos);
        if band > 0.0 {
            let t = f.step.w;
            let blot = vnoise(vec3<f32>(pos * 0.13, t * 0.5)) * 0.65 + vnoise(vec3<f32>(pos * 0.37 + 17.0, t * 1.7)) * 0.35;
            let puff = 0.75 + 0.5 * vnoise(vec3<f32>(pos * 0.05 + 3.0, t * 2.4));
            let heat = band * f.look.w * smoothstep(0.55, 0.8, blot) * puff * 1.4;
            s.w = max(s.w, min(heat, 1.5));
            s.y = max(s.y, 2.0 * heat);
        }
    }
    if k == 0 {
        s.y = max(s.y, 0.0);
    }
    // the top lets out what reaches it
    if k == n_up() - 1 {
        s.w *= 0.5;
    }
    s.w = clamp(s.w, 0.0, 1.5);
    textureStore(out_vec, slot(w, k), s);
}

// --- 2. vorticity ---------------------------------------------------------

// A neighbour's state for the differences: itself across the grid's edge.
fn nb(w: vec2<i32>, k: i32, d: vec3<i32>) -> vec4<f32> {
    let wn = w + vec2<i32>(d.x, d.z);
    let kn = clamp(k + d.y, 0, n_up() - 1);
    if !inside(wn, f.origin.xy) {
        return textureLoad(tex_a, slot(w, k), 0);
    }
    return textureLoad(tex_a, slot(wn, kn), 0);
}

fn nb_b(w: vec2<i32>, k: i32, d: vec3<i32>) -> vec4<f32> {
    let wn = w + vec2<i32>(d.x, d.z);
    let kn = clamp(k + d.y, 0, n_up() - 1);
    if !inside(wn, f.origin.xy) {
        return textureLoad(tex_b, slot(w, k), 0);
    }
    return textureLoad(tex_b, slot(wn, kn), 0);
}

@compute @workgroup_size(4, 4, 4)
fn curl(@builtin(global_invocation_id) id: vec3<u32>) {
    if !ok(id) {
        return;
    }
    let w = world_of(id);
    let k = i32(id.y);
    let dx = f.step.x;
    let dz = f.step.y;
    let xp = nb(w, k, vec3<i32>(1, 0, 0));
    let xm = nb(w, k, vec3<i32>(-1, 0, 0));
    let yp = nb(w, k, vec3<i32>(0, 1, 0));
    let ym = nb(w, k, vec3<i32>(0, -1, 0));
    let zp = nb(w, k, vec3<i32>(0, 0, 1));
    let zm = nb(w, k, vec3<i32>(0, 0, -1));
    let om = vec3<f32>(
        (yp.z - ym.z) / (2.0 * dz) - (zp.y - zm.y) / (2.0 * dx),
        (zp.x - zm.x) / (2.0 * dx) - (xp.z - xm.z) / (2.0 * dx),
        (xp.y - xm.y) / (2.0 * dx) - (yp.x - ym.x) / (2.0 * dz),
    );
    textureStore(out_vec, slot(w, k), vec4<f32>(om, length(om)));
}

// a: the state, b: the vorticity
@compute @workgroup_size(4, 4, 4)
fn confine(@builtin(global_invocation_id) id: vec3<u32>) {
    if !ok(id) {
        return;
    }
    let w = world_of(id);
    let k = i32(id.y);
    let dx = f.step.x;
    let dz = f.step.y;
    let s = textureLoad(tex_a, slot(w, k), 0);
    let om = textureLoad(tex_b, slot(w, k), 0);
    let eta = vec3<f32>(
        (nb_b(w, k, vec3<i32>(1, 0, 0)).w - nb_b(w, k, vec3<i32>(-1, 0, 0)).w) / (2.0 * dx),
        (nb_b(w, k, vec3<i32>(0, 1, 0)).w - nb_b(w, k, vec3<i32>(0, -1, 0)).w) / (2.0 * dz),
        (nb_b(w, k, vec3<i32>(0, 0, 1)).w - nb_b(w, k, vec3<i32>(0, 0, -1)).w) / (2.0 * dx),
    );
    let len = length(eta);
    var v = s.xyz;
    if len > 1.0e-5 {
        // only where it is warm: the swirl of the flames, not of the wind
        let force = f.look.x * dx * cross(eta / len, om.xyz) * smoothstep(0.02, 0.3, s.w);
        v += f.step.z * force;
    }
    textureStore(out_vec, slot(w, k), vec4<f32>(v, s.w));
}

// --- 3. pressure ------------------------------------------------------------

@compute @workgroup_size(4, 4, 4)
fn divergence(@builtin(global_invocation_id) id: vec3<u32>) {
    if !ok(id) {
        return;
    }
    let w = world_of(id);
    let k = i32(id.y);
    let dx = f.step.x;
    let dz = f.step.y;
    let s = textureLoad(tex_a, slot(w, k), 0);
    // the ground: no air through it
    var ym = nb(w, k, vec3<i32>(0, -1, 0)).y;
    if k == 0 {
        ym = -s.y;
    }
    let d = (nb(w, k, vec3<i32>(1, 0, 0)).x - nb(w, k, vec3<i32>(-1, 0, 0)).x) / (2.0 * dx)
        + (nb(w, k, vec3<i32>(0, 1, 0)).y - ym) / (2.0 * dz)
        + (nb(w, k, vec3<i32>(0, 0, 1)).z - nb(w, k, vec3<i32>(0, 0, -1)).z) / (2.0 * dx);
    textureStore(out_scalar, slot(w, k), vec4<f32>(d, 0.0, 0.0, 0.0));
}

// A neighbour's pressure: 0 past the sides and the top (open air), the
// voxel's own under the ground.
fn p_nb(w: vec2<i32>, k: i32, d: vec3<i32>) -> f32 {
    let wn = w + vec2<i32>(d.x, d.z);
    let kn = k + d.y;
    if kn < 0 {
        return textureLoad(tex_b, slot(w, k), 0).r;
    }
    if kn >= n_up() || !inside(wn, f.origin.xy) {
        return 0.0;
    }
    return textureLoad(tex_b, slot(wn, kn), 0).r;
}

// a: the divergence, b: the pressure so far
@compute @workgroup_size(4, 4, 4)
fn jacobi(@builtin(global_invocation_id) id: vec3<u32>) {
    if !ok(id) {
        return;
    }
    let w = world_of(id);
    let k = i32(id.y);
    let ix = 1.0 / (f.step.x * f.step.x);
    let iy = 1.0 / (f.step.y * f.step.y);
    let sx = p_nb(w, k, vec3<i32>(1, 0, 0)) + p_nb(w, k, vec3<i32>(-1, 0, 0)) + p_nb(w, k, vec3<i32>(0, 0, 1)) + p_nb(w, k, vec3<i32>(0, 0, -1));
    let sy = p_nb(w, k, vec3<i32>(0, 1, 0)) + p_nb(w, k, vec3<i32>(0, -1, 0));
    let d = textureLoad(tex_a, slot(w, k), 0).r;
    let p = (sx * ix + sy * iy - d) / (4.0 * ix + 2.0 * iy);
    textureStore(out_scalar, slot(w, k), vec4<f32>(p, 0.0, 0.0, 0.0));
}

// a: the state, b: the pressure
@compute @workgroup_size(4, 4, 4)
fn project(@builtin(global_invocation_id) id: vec3<u32>) {
    if !ok(id) {
        return;
    }
    let w = world_of(id);
    let k = i32(id.y);
    let dx = f.step.x;
    let dz = f.step.y;
    let s = textureLoad(tex_a, slot(w, k), 0);
    let g = vec3<f32>(
        (p_nb(w, k, vec3<i32>(1, 0, 0)) - p_nb(w, k, vec3<i32>(-1, 0, 0))) / (2.0 * dx),
        (p_nb(w, k, vec3<i32>(0, 1, 0)) - p_nb(w, k, vec3<i32>(0, -1, 0))) / (2.0 * dz),
        (p_nb(w, k, vec3<i32>(0, 0, 1)) - p_nb(w, k, vec3<i32>(0, 0, -1))) / (2.0 * dx),
    );
    var v = s.xyz - g;
    if k == 0 {
        v.y = max(v.y, 0.0);
    }
    // kept to what a fire's air does: no blow-ups at a big step
    let speed = length(v);
    if speed > 30.0 {
        v *= 30.0 / speed;
    }
    textureStore(out_vec, slot(w, k), vec4<f32>(v, s.w));
}
