// What the plants' main and shadow passes share (`vegetation/render.rs`):
// their bindings, their vertex input, and how a plant is placed, swayed and
// eaten by the fire. The fire state comes from one number per plant decided
// on the CPU (`vegetation::plant_fire`), the instant it catches; the rest is
// a function of the simulated clock.

import game::shaders::noise::{hash21, value_noise};

struct PlantUniform {
    // x: the simulated clock (s); y: how long a plant flames (s);
    // z: the share of plants shown (thinning); w: the tactical view's greying
    clock: vec4<f32>,
    // xy: where the wind blows to (Bevy x, z); z: its strength (0..1);
    // w: real seconds (wrapped), for sway and flicker
    wind: vec4<f32>,
    // x = 1: a burnt crown is gone and its remains show (desktop); 0: a
    // burnt plant turns all charcoal, its crown shrunk (the browser);
    // yz: the size of a canopy tile in the atlas (`PLANT_CANOPY`)
    look: vec4<f32>,
}

@group(2) @binding(0) var<uniform> plants: PlantUniform;
// the foliage atlas: leaf twigs cut out by their alpha, the proxies' canopy
// tiles, bark, and a flat grey (linear 0.5) for what has no texture
@group(2) @binding(1) var atlas: texture_2d<f32>;
@group(2) @binding(2) var atlas_sampler: sampler;

// `vegetation/meshes.rs` and `vegetation::instance`
struct PlantVertex {
    @location(0) position: vec3<f32>,
    // xyz: the normal; w: what the vertex is (`kind_*`)
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    // rgb: the colour (linear; halved on leaves and wood); a: occlusion
    @location(3) color: vec4<f32>,
    // the plant: foot (Bevy frame) and yaw, crown width and height (m),
    // tint (leaf rgb, wood a; multipliers / 2), its own numbers (thinning
    // rank, sway phase, shade)
    @location(4) foot_yaw: vec4<f32>,
    @location(5) size: vec2<f32>,
    @location(6) tint: vec4<f32>,
    @location(7) seed: vec4<f32>,
    // x: the instant it catches (`NEVER` while the fire is not near); y = 1: cut by a crew
    @location(8) fire: vec2<f32>,
    // a scanned plant's card: its right and up axes in model space as
    // scanned (zero elsewhere), card_a.w half the stretch across
    // (`meshes::Builder::scan`)
    @location(9) card_a: vec4<f32>,
    @location(10) card_b: vec4<f32>,
}

fn turn(x: vec3<f32>, c: f32, s: f32) -> vec3<f32> {
    return vec3<f32>(c * x.x + s * x.z, x.y, -s * x.x + c * x.z);
}

// A scanned plant's card is one of three crossed on a cluster of leaves
// (`tools/vegetation/lod.py`): only the one most square to `toward` (from
// the plant to the viewer, or to the sun) is drawn. Its axes and normal are
// the three cards' normals, so each card can tell alone. The three
// vertices of a triangle agree: same card, same plant.
//
// The plant is stretched across (x, z) by `k` and scaled by its size: a
// card's normal goes as the inverse of both, so its facing to `toward` is
// the scanned card's to `toward` in model space, divided by the same.
fn card_hidden(v: PlantVertex, toward: vec3<f32>) -> bool {
    let a = v.card_a.xyz;
    if dot(a, a) < 0.01 {
        return false;
    }
    let b = v.card_b.xyz;
    let n = cross(a, b);
    // into the model's frame: turned back, unstretched, unscaled
    let c = cos(v.foot_yaw.w);
    let s = sin(v.foot_yaw.w);
    let local = turn(toward, c, -s);
    let k = max(v.card_a.w * 2.0, 0.1);
    let t = local / vec3<f32>(k * v.size.x, v.size.y, k * v.size.x);
    return abs(dot(n, t)) < max(abs(dot(a, t)), abs(dot(b, t))) - 1.0e-6;
}

// as `vegetation::NEVER`
const NEVER: f32 = 1.0e9;
// Seconds the fire takes to climb a metre of the plant (2026-11-01, from
// main's review in `KIOSK_PIANTE`): it catches at the foot and climbs, so a
// pine's crown lights some forty seconds after its trunk, a shrub's within a
// few. Per vertex, on top of the plant's instant; the plant's state (burning,
// burnt) is still the plant's.
const CLIMB_S_PER_M: f32 = 2.8;
// share of the flaming time a point of the crown takes to burn away; the
// embers in the wood last the rest
const CROWN_SHARE: f32 = 0.5;
// below it the atlas texel is not leaf
const ALPHA_CUT: f32 = 0.5;

// what a vertex is, from the normal's w (`meshes::Kind`)
fn kind_leaf(k: f32) -> bool { return k > 0.5; }
fn kind_remains(k: f32) -> bool { return k < -0.25; }
fn kind_debris(k: f32) -> bool { return k < -0.75; }

// A point outside every view: the triangles of a vertex that is gone (all
// three of its vertices are, they belong to one plant and one kind) are
// clipped away before they cost a fragment.
const GONE: vec4<f32> = vec4<f32>(2.0, 2.0, 2.0, 1.0);

struct Placed {
    world: vec3<f32>,
    // where the vertex is from the plant's foot, turned with it but before
    // the wind: what stays put on a swaying crown (`PLANT_CANOPY`)
    rest: vec3<f32>,
    normal: vec3<f32>,
    // -1: standing; 0..1: burning (how far); 2: burnt
    burn: f32,
    // how far the fire has got at this vertex, in crown burn-times
    // ([`CROWN_SHARE`] of the flaming time) since the climb reached it:
    // below 0 not yet, 1 the leaves here are gone, `1 / CROWN_SHARE` charcoal
    climb: f32,
    gone: bool,
}

fn place(v: PlantVertex) -> Placed {
    var out: Placed;
    let kind = v.normal.w;
    let age = plants.clock.x - v.fire.x;
    let reached = v.fire.x < NEVER * 0.5 && age >= 0.0;
    var burn = -1.0;
    if reached {
        burn = select(clamp(age / max(plants.clock.y, 1.0), 0.0, 1.0), 2.0, age >= plants.clock.y);
    }
    out.burn = burn;
    // the climb: this vertex catches when the fire reaches its height
    let height_m = max(v.position.y, 0.0) * v.size.y;
    out.climb = (age - CLIMB_S_PER_M * height_m) / max(plants.clock.y * CROWN_SHARE, 1.0);
    let burnt = burn > 1.5;
    let remains = plants.look.x > 0.5;

    var gone = false;
    if kind_remains(kind) {
        // what the fire adds is there from the moment the plant catches
        gone = !reached;
    } else {
        // thinned for the machine (by the plant's rank) or cut by a crew,
        // unless the fire has it already
        gone = !reached && (v.seed.x >= plants.clock.z || v.fire.y > 0.5);
        // the crown burnt away; trunk and boughs stay, charred
        gone = gone || (burnt && kind_leaf(kind) && remains);
    }
    out.gone = gone;

    var local = v.position;
    if kind_leaf(kind) && burn >= 0.0 && (burn <= 1.0 || !remains) {
        // late in the burn what is left of the crown slumps toward the trunk;
        // without remains to take its place (the browser) the charred crown
        // stays so, a tuft among the bare limbs rather than a black ball
        let k = 1.0 - 0.6 * smoothstep(0.55, 1.0, min(burn, 1.0));
        local = vec3<f32>(local.x * k, local.y * mix(1.0, 0.92, 1.0 - k), local.z * k);
    }
    let s = vec3<f32>(v.size.x, v.size.y, v.size.x);
    let p = local * s;
    let c = cos(v.foot_yaw.w);
    let sn = sin(v.foot_yaw.w);
    out.rest = vec3<f32>(c * p.x + sn * p.z, p.y, -sn * p.x + c * p.z);
    var world = v.foot_yaw.xyz + out.rest;

    // wind: the crown leans downwind and sways in gusts, the leaves flutter;
    // a burning plant shakes in its own draught. Weighted by height, so the
    // foot stays put. Charred wood and remains stand still.
    if !kind_remains(kind) && !burnt {
        let h = clamp(v.position.y, 0.0, 1.2);
        let t = plants.wind.w;
        let phase = v.seed.y * 6.2832 + dot(v.foot_yaw.xz, vec2<f32>(0.021, 0.017));
        let gust = 0.6 + 0.28 * sin(t * 0.83 + phase) + 0.12 * sin(t * 2.1 + phase * 1.7);
        let heat = select(0.0, 1.0, burn >= 0.0);
        let lean = (plants.wind.z * 0.022 * gust + heat * 0.012 * sin(t * 7.0 + phase)) * v.size.y * h * h;
        world += vec3<f32>(plants.wind.x, 0.0, plants.wind.y) * lean;
        if kind_leaf(kind) {
            let flutter = sin(t * 4.7 + phase + dot(v.position, vec3<f32>(17.0, 11.0, 13.0)));
            world.y += flutter * (0.006 + 0.012 * plants.wind.z + 0.02 * heat) * v.size.y * h;
        }
    }
    out.world = world;
    // the normal under the instance's scale (inverse transpose) and turn
    let n = v.normal.xyz / s;
    out.normal = normalize(vec3<f32>(c * n.x + sn * n.z, n.y, -sn * n.x + c * n.z) + vec3<f32>(0.0, 1.0e-5, 0.0));
    return out;
}

// --- fire, shared by the main and the shadow pass ------------------------

// How far the front has got at a point: below 0 not yet, above 0.3 the
// leaves there are mostly gone. Where the fire takes hold first is a
// property of the place, so patches of one crown light up at different
// moments and the front crawls. The skeleton uses the same field to show up
// where the leaves go.
fn burn_front(p: vec3<f32>, prog: f32) -> f32 {
    let n = 0.45 * value_noise(p.xz * 0.8 + vec2<f32>(p.y * 0.5, 3.0))
          + 0.55 * value_noise(p.xz * 2.7 + vec2<f32>(-p.y * 1.9, 7.0));
    return prog * 1.6 - n * 0.9;
}

// Where a leaf is eaten first: holes cut at every distance, in cells about
// a pixel wide (`px_m` metres), so a crown thins out instead of darkening
// to grey. Only the cut-out passes call it.
fn hole_at(p: vec3<f32>, px_m: f32) -> f32 {
    let cell = max(0.3, px_m * 1.5);
    let coarse = hash21(floor(p.xz / cell) + vec2<f32>(floor(p.y / cell) * 37.0, 5.0));
    let fine = value_noise(p.xz * 3.1 + vec2<f32>(p.y * 2.3, 17.0));
    return mix(coarse, fine, 1.0 - smoothstep(1.0, 3.5, px_m));
}

// How much of the leaves at `p` the front has eaten (`burn_plant`): what
// the shadow pass must also leave out, or a crown burnt to its twigs goes
// on casting the shadow of its foliage (2026-11-04).
fn burn_gone(p: vec3<f32>, prog: f32) -> f32 {
    return smoothstep(0.20, 0.48, burn_front(p, prog));
}
