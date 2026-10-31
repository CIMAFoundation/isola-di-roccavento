// The vegetation material's bindings (`materials/foliage.rs`) and the plants'
// fire state on the GPU (`vegetation.rs`).
//
// The meshes never change: every vertex carries the global index of its plant
// (a float), one RGBA8 texture for the island holds one texel per plant (24-bit
// ignition instant in quarter seconds, 8 bits of flags: `vegetation::Texel`),
// and the stage below works out from the simulated clock whether the plant
// stands, burns, is charcoal or gone (`vegetation::shown` is the CPU's copy).
// No float texture, no integer vertex attribute (2026-10-31, the iPhone).
//
// It runs in the vertex stage, shadow and depth passes included, so a plant that
// is gone casts no shadow. The colour comes out in the encoding the fragment
// stage reads (`foliage.wgsl`): a negative alpha holds the ignition time.


struct Foliage {
    // x: tactical desaturation (`tactical.rs`); y: the simulated clock, s;
    // z: how long a plant flames, s (`vegetation::fire_clock`)
    fire: vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> foliage: Foliage;
// one texel per plant, rows of `ROW`: Rgba8Unorm, read with textureLoad
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var veg_state: texture_2d<f32>;
// the foliage atlas (`assets/models/foliage_atlas.png`, sRGB, mipmapped with
// the leaves' coverage kept): leaf cards cut out by its alpha, bark, canopy
// tiles of the proxies, and a flat grey (linear 0.5) for the untextured
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atlas: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var atlas_sampler: sampler;

// below it the atlas texel is not leaf (`AlphaMode::Mask` in `materials/foliage.rs`)
const ALPHA_CUT: f32 = 0.5;
// vertex alpha of a charred vertex (`vegetation::CHARRED_ALPHA`)
const CHARRED: f32 = 0.5;

const ROW: u32 = 1024u;
// as `vegetation::TICK_S`, `NEVER`, `HAS_REMAINS` and `GONE` (bits of alpha)
const TICK_S: f32 = 0.25;
const NEVER: u32 = 0xffffffu;
const HAS_REMAINS: u32 = 1u;
const GONE: u32 = 2u;

// what a plant shows, as `vegetation::shown`
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

// as `vegetation::charcoal`
fn charcoal(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(0.022 + c.r * 0.03, 0.019 + c.g * 0.025, 0.017 + c.b * 0.025, CHARRED);
}

// A plant's vertex after its fire state. `color.a` as built: 1 on leaves (and,
// in the remains layer, on debris), 0 elsewhere.
fn plant_apply(plant: f32, position: vec3<f32>, color: vec4<f32>) -> Planted {
    var out: Planted;
    out.position = position;
    out.color = color;
    @if(NO_PLANT_STATE) {
        return out;
    }
    // -1: no plant; -(index + 2): the remains layer
    if plant > -1.5 && plant < -0.5 {
        return out;
    }
    let remains_layer = plant < -1.5;
    let idx = u32(select(plant, -plant - 2.0, remains_layer) + 0.5);
    // two texels a plant: the state, then (instances) the tint
    let at = idx * 2u;
    let px = textureLoad(veg_state, vec2<i32>(i32(at % ROW), i32(at / ROW)), 0);
    let b = vec4<u32>(round(px * 255.0));
    let ticks = b.r | (b.g << 8u) | (b.b << 16u);
    let t0 = f32(ticks) * TICK_S;
    let age = foliage.fire.y - t0;
    var mode = UNBURNT;
    if (b.a & GONE) != 0u {
        mode = CLEARED;
    } else if ticks != NEVER && age >= 0.0 {
        if age < foliage.fire.z {
            mode = BURNING;
        } else {
            mode = select(BURNT, BURNT_REMAINS, (b.a & HAS_REMAINS) != 0u);
        }
    }
    let flag = color.a > 0.5;
    @if(PLANT_TAG) {
        // an instance of its variety's mesh: its tint is in the image, not
        // in the vertex colours (`vegetation::tint_texel`, halved)
        let tint = textureLoad(veg_state, vec2<i32>(i32((at + 1u) % ROW), i32((at + 1u) / ROW)), 0) * 2.0;
        out.color = vec4<f32>(color.rgb * select(vec3<f32>(tint.a), tint.rgb, flag), color.a);
    }
    let tinted = out.color;
    // collapsed to one point (the origin): its triangles have no area and
    // draw nothing
    let gone = vec3<f32>(0.0);

    if remains_layer {
        // the remains: hidden until the plant ignites
        if mode == UNBURNT || mode == CLEARED {
            out.position = gone;
            return out;
        }
        if flag {
            // ash is soot: the model's pale grey reads as a platform under the trees
            out.color = vec4<f32>(tinted.r * 0.2, tinted.g * 0.17, tinted.b * 0.15, tinted.a);
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
            out.color = charcoal(tinted);
        }
    } else if mode == BURNT {
        out.color = charcoal(tinted);
    }
    return out;
}
