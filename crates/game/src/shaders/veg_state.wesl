// The vegetation material's bindings (`materials/foliage.rs`) and the plants'
// fire state on the GPU (`vegetation.rs`).
//
// Plants used to be redrawn on the CPU when the fire reached them, which meant
// re-uploading whole chunk meshes. Now the meshes never change: every vertex
// carries the index of its plant, a small per-chunk texture holds one texel per
// plant (x = state, y = ignition time), and the vertex stage does what
// `vegetation::draw_standing` / `draw_char` / `paint` do on the CPU (those stay
// as the reference for the tests).
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
// one texel per plant, rows of 256: Rg32Float, read with textureLoad
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var veg_state: texture_2d<f32>;

const ROW: u32 = 256u;
// bit 31 of a vertex's plant index: the vertex belongs to the burnt-remains layer
const CHAR_BIT: u32 = 0x80000000u;
const NO_PLANT: u32 = 0x7fffffffu;

// state codes, as `vegetation::GpuState`
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
    return vec4<f32>(0.022 + c.r * 0.03, 0.019 + c.g * 0.025, 0.017 + c.b * 0.025, 0.0);
}

// A plant's vertex after its fire state. `color.a` as built: 1 on leaves (and,
// in the remains layer, on debris), 0 elsewhere.
fn plant_apply(plant: u32, position: vec3<f32>, color: vec4<f32>) -> Planted {
    var out: Planted;
    out.position = position;
    out.color = color;
    @if(NO_PLANT_STATE) {
        return out;
    }
    let idx = plant & NO_PLANT;
    if idx == NO_PLANT {
        return out;
    }
    let st = textureLoad(veg_state, vec2<i32>(i32(idx % ROW), i32(idx / ROW)), 0);
    let mode = u32(st.x + 0.5);
    let t0 = max(st.y, 0.0);
    let flag = color.a > 0.5;
    // collapsed to one point, the way the CPU path folds a plant onto its first
    // vertex: its triangles have no area and draw nothing
    let gone = vec3<f32>(0.0);

    if (plant & CHAR_BIT) != 0u {
        // the remains: hidden until the plant ignites
        if mode == UNBURNT || mode == CLEARED {
            out.position = gone;
            return out;
        }
        if flag {
            // ash is soot: the model's pale grey reads as a platform under the trees
            out.color = vec4<f32>(color.r * 0.2, color.g * 0.17, color.b * 0.15, color.a);
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
            out.color = charcoal(color);
        }
    } else if mode == BURNT {
        out.color = charcoal(color);
    }
    return out;
}
