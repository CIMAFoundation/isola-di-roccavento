// The vegetation material's bindings (`materials/foliage.rs`) and the plants'
// fire state on the GPU (`vegetation.rs`).
//
// The plants have no state of their own (2026-10-31): like the terrain, each
// works out how it looks from the fire grid at its foot. The grid is one RGBA8
// texel per fire cell (24-bit arrival time in seconds, the cleared flag), and
// the stage below interpolates it at the plant's position as
// `field::FireField::arrival` does, adds the plant's own lag (a hash of its
// position, `vegetation::plant_lag`), and from the simulated clock decides
// whether the plant stands, burns, is charcoal or gone
// (`vegetation::plant_look` is the CPU's copy, for the tests).
//
// It runs in the vertex stage, shadow and depth passes included, so a plant that
// is gone casts no shadow. The colour comes out in the encoding the fragment
// stage reads (`foliage.wgsl`): a negative alpha holds the ignition time.


struct Foliage {
    // x: tactical desaturation (`tactical.rs`); y: the simulated clock, s;
    // z: how long a plant flames, s; w: the share of plants shown
    fire: vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> foliage: Foliage;
// one texel per fire cell, row 0 to the north: Rgba8Unorm, read with textureLoad
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var fire_grid: texture_2d<f32>;
// the foliage atlas (`assets/models/foliage_atlas.png`, sRGB, mipmapped with
// the leaves' coverage kept): leaf cards cut out by its alpha, bark, canopy
// tiles of the proxies, and a flat grey (linear 0.5) for the untextured
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atlas: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var atlas_sampler: sampler;
// cell size (m), north edge (m), columns, rows
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var<uniform> grid: vec4<f32>;
// x = 1: the burnt remains' material; y = 1: the plants have remains (a burnt
// plant loses its crown and keeps its wood, rather than turning all charcoal)
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var<uniform> layer: vec4<f32>;

// below it the atlas texel is not leaf (`AlphaMode::Mask` in `materials/foliage.rs`)
const ALPHA_CUT: f32 = 0.5;
// vertex alpha of a charred vertex
const CHARRED: f32 = 0.5;

// as `vegetation::NEVER`, `CLEARED_BIT`, `LAG_S`, `CLEARED_SHARE`
const NEVER: u32 = 0xffffffu;
const CLEARED_BIT: u32 = 1u;
const LAG_S: f32 = 45.0;
const CLEARED_SHARE: f32 = 0.85;

// what a plant shows, as `vegetation::Look`
const UNBURNT: u32 = 0u;
const BURNING: u32 = 1u;
const BURNT_REMAINS: u32 = 2u;
const BURNT: u32 = 3u;
const CLEARED: u32 = 4u;

// as `vegetation::LEAF_OFFSET` and the next two
const LEAF_OFFSET: f32 = 1.0e5;
const REMAINS_OFFSET: f32 = 2.0e5;
const DEBRIS_OFFSET: f32 = 4.0e5;

struct Planted {
    position: vec3<f32>,
    color: vec4<f32>,
}

fn charcoal(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(0.022 + c.r * 0.03, 0.019 + c.g * 0.025, 0.017 + c.b * 0.025, CHARRED);
}

// as `vegetation::hash01`: a number in [0, 1) from the bits of a position
fn hash01(x: f32, y: f32, salt: u32) -> f32 {
    var h = (bitcast<u32>(x) * 0x9E3779B1u) ^ (bitcast<u32>(y) * 0x85EBCA6Bu) ^ salt;
    h = (h ^ (h >> 15u)) * 0x2C1B3C6Du;
    h = h ^ (h >> 13u);
    return f32(h >> 8u) / 16777216.0;
}

struct Cell {
    // arrival time, s; < 0: not reached
    arrival: f32,
    cleared: f32,
}

fn cell(col: u32, row: u32) -> Cell {
    let px = textureLoad(fire_grid, vec2<i32>(i32(col), i32(row)), 0);
    let b = vec4<u32>(round(px * 255.0));
    let t = b.r | (b.g << 8u) | (b.b << 16u);
    return Cell(select(f32(t), -1.0, t == NEVER), f32(b.a & CLEARED_BIT));
}

struct Ground {
    // interpolated arrival, s; < 0: no neighbouring cell reached
    arrival: f32,
    cleared: f32,
}

// as `field::FireField::arrival` and `cleared_fraction`, at a point of the
// scenario frame (x east, y north)
fn ground(x: f32, y: f32) -> Ground {
    let cols = u32(grid.z);
    let rows = u32(grid.w);
    let fx = clamp(x / grid.x - 0.5, 0.0, f32(cols - 1u));
    let fy = clamp((grid.y - y) / grid.x - 0.5, 0.0, f32(rows - 1u));
    let x0 = u32(floor(fx));
    let y0 = u32(floor(fy));
    let x1 = min(x0 + 1u, cols - 1u);
    let y1 = min(y0 + 1u, rows - 1u);
    let tx = fract(fx);
    let ty = fract(fy);
    let c = array<Cell, 4>(cell(x0, y0), cell(x1, y0), cell(x0, y1), cell(x1, y1));
    let w = array<f32, 4>((1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty);
    var sum = 0.0;
    var total = 0.0;
    var cleared = 0.0;
    for (var i = 0; i < 4; i++) {
        cleared += c[i].cleared * w[i];
        if c[i].arrival >= 0.0 && w[i] > 0.0 {
            sum += c[i].arrival * w[i];
            total += w[i];
        }
    }
    // unburnt corners drag the estimate later (see the Rust)
    let arrival = select(-1.0, sum / total + (1.0 - total) * 90.0, total > 0.0);
    return Ground(arrival, cleared);
}

// What a plant standing at `foot` (Bevy frame) shows now, as `vegetation::plant_look`;
// `t0` is set to its ignition instant.
fn look(foot: vec3<f32>, t0: ptr<function, f32>) -> u32 {
    let x = foot.x;
    let y = -foot.z;
    let g = ground(x, y);
    let lag = hash01(x, y, 0x51EDu) * LAG_S;
    var mode = UNBURNT;
    *t0 = 0.0;
    if g.arrival >= 0.0 {
        *t0 = g.arrival + lag;
        let age = foliage.fire.y - *t0;
        if age >= foliage.fire.z {
            mode = select(BURNT, BURNT_REMAINS, layer.y > 0.5);
        } else if age >= 0.0 {
            mode = BURNING;
        }
    }
    // thinned for the machine's sake (a burnt plant stays)
    if mode != BURNT && mode != BURNT_REMAINS && hash01(x, y, 0u) >= foliage.fire.w {
        return CLEARED;
    }
    // cut by a crew: thinned, not razed, the closer to the middle of the line
    // the likelier, by the plant's own number
    let inside = clamp((g.cleared - 0.45) / 0.15, 0.0, 1.0);
    if mode == UNBURNT && lag / LAG_S < CLEARED_SHARE * inside {
        return CLEARED;
    }
    return mode;
}

// A plant's vertex after its fire state. `foot`: where the plant stands;
// `color.a` as built: 1 on leaves (and, in the remains layer, on debris), 0
// elsewhere; `tint` the plant's (leaf rgb, wood a; multipliers / 2), not on
// the remains.
fn plant_apply(foot: vec3<f32>, position: vec3<f32>, color: vec4<f32>, tint: vec4<f32>) -> Planted {
    var out: Planted;
    out.position = position;
    let flag = color.a > 0.5;
    let remains_layer = layer.x > 0.5;
    var c = color;
    if !remains_layer {
        c = vec4<f32>(color.rgb * select(vec3<f32>(tint.a), tint.rgb, flag) * 2.0, color.a);
    }
    out.color = c;
    @if(NO_PLANT_STATE) {
        if remains_layer {
            out.position = vec3<f32>(0.0);
        }
        return out;
    }
    var t0 = 0.0;
    let mode = look(foot, &t0);
    // collapsed to one point: its triangles have no area and draw nothing
    let gone = vec3<f32>(0.0);

    if remains_layer {
        // the remains: hidden until the plant ignites
        if mode == UNBURNT || mode == CLEARED {
            out.position = gone;
            return out;
        }
        if flag {
            // ash is soot: the model's pale grey reads as a platform under the trees
            out.color = vec4<f32>(c.r * 0.2, c.g * 0.17, c.b * 0.15, c.a);
        }
        out.color.a = select(0.0, -(1.0 + REMAINS_OFFSET + select(0.0, DEBRIS_OFFSET, flag) + t0), mode == BURNING);
        return out;
    }

    if mode == CLEARED {
        out.position = gone;
    } else if mode == BURNING {
        out.color.a = -(1.0 + select(0.0, LEAF_OFFSET, flag) + t0);
    } else if mode == BURNT_REMAINS {
        // the crown is gone; trunk and boughs stay, charred
        if flag {
            out.position = gone;
        } else {
            out.color = charcoal(c);
        }
    } else if mode == BURNT {
        out.color = charcoal(c);
    }
    return out;
}

// The instance transform (`veg_instancing::PlantInstance`): model space to
// world, and a model normal to world.
fn place(local: vec3<f32>, foot: vec3<f32>, yaw: f32, scale: vec3<f32>) -> vec3<f32> {
    let p = local * scale;
    let c = cos(yaw);
    let s = sin(yaw);
    return foot + vec3<f32>(c * p.x + s * p.z, p.y, -s * p.x + c * p.z);
}

fn place_normal(n: vec3<f32>, yaw: f32, scale: vec3<f32>) -> vec3<f32> {
    let m = n / scale;
    let c = cos(yaw);
    let s = sin(yaw);
    return normalize(vec3<f32>(c * m.x + s * m.z, m.y, -s * m.x + c * m.z));
}
