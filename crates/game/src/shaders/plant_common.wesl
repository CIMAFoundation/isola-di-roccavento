// What the plants' main and shadow passes share (`vegetation/render.rs`):
// their bindings, their vertex input, and how a plant is placed, swayed and
// eaten by the fire. The fire state comes from one number per plant decided
// on the CPU (`vegetation::plant_fire`), the instant it catches; the rest is
// a function of the simulated clock.

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
