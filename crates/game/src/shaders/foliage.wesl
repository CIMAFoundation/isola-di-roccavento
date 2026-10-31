// The vegetation (`materials/foliage.rs`): leaf clumps and bark over the
// vertex colours, and the burning crown eaten against the simulated clock.
// The vertex stage (`veg_vertex.wgsl`, `veg_prepass.wgsl`) applies each
// plant's fire state from the chunk's state image (`veg_state.wgsl`) and hands
// it over in the vertex colour's alpha: 1 on living leaves, 0 on wood, rock,
// grass and anything burnt; below -0.5 the vertex is burning and the alpha
// holds its ignition time (offsets below).
// A material of its own, lit by Bevy's PBR (`apply_pbr_lighting`): an
// extension of StandardMaterial would bring textures it never reads, and on
// WebGL2 every texture of the layout takes one of the 16 units.


// Leaves: matte with a faint waxy sheen (a full dielectric sheen on a
// hundred thousand crowns turns the canopy to tinsel), and thin, so the sun
// comes through them: a crown seen against the light glows green-gold.
// Wood: rough bark, no sheen, opaque.

import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::mesh_bindings::mesh;
import bevy_pbr::render::mesh_view_bindings::{globals, view};
import bevy_pbr::render::{pbr_functions, pbr_types};
import game::shaders::noise::{hash21, value_noise};
import game::shaders::scenery::tactical_grey;
import game::shaders::veg_state::foliage;

const LEAF_ROUGHNESS: f32 = 0.78;
const LEAF_REFLECTANCE: f32 = 0.18;
const LEAF_TRANSMISSION: f32 = 0.38;
const LEAF_THICKNESS: f32 = 0.08;
const WOOD_ROUGHNESS: f32 = 0.9;
const WOOD_REFLECTANCE: f32 = 0.08;

// Breaks the flat vertex colour into
// leaf clumps and a fine speckle, plus a warm sunlit / cool shaded lean.
// Two value-noise taps, faded out once a pixel spans more than ~2 m, so
// the far forest costs next to nothing and never shimmers.
fn foliage_albedo(base: vec4<f32>, p: vec3<f32>, normal: vec3<f32>, alive: f32) -> vec4<f32> {
    // `?trama=0` in the browser: the flat vertex colour (`materials/foliage.rs`)
    @if(PLAIN_FOLIAGE) {
        return base;
    }
    let px_m = max(length(dpdx(p)), length(dpdy(p)));
    let fine = 1.0 - smoothstep(1.0, 3.5, px_m);
    let mid = 1.0 - smoothstep(2.0, 7.0, px_m);
    // Per-patch tint on a ~3.5 m lattice: about one macchia shrub each, so
    // neighbouring bushes differ in tone; it outlives the fine detail.
    let patch_tone = hash21(floor(p.xz / 3.5)) - 0.5;
    let clump = value_noise(p.xz * 0.9 + vec2<f32>(p.y * 0.6, p.y * 0.35));
    // Posterised clumps: distinct light and dark leaf masses with soft
    // steps between them, rather than a smooth gradient.
    let steps = floor(clump * 3.0) / 3.0 + smoothstep(0.0, 0.18, fract(clump * 3.0)) / 3.0;
    var tone = patch_tone * 0.45 * mid + (mix(clump, steps, 0.7) - 0.5) * 0.75 * mid;
    if fine > 0.0 {
        let speck = value_noise(p.xz * 3.2 + vec2<f32>(p.y * 3.3, -p.y * 2.1));
        // Sharp leaf dots: bright tips, dark gaps between them.
        let leaves = smoothstep(0.38, 0.62, speck);
        tone += (leaves - 0.5) * 0.60 * fine - (1.0 - leaves) * 0.10 * fine;
    }
    let up = clamp(normalize(normal).y * 0.5 + 0.5, 0.0, 1.0);
    let warm = vec3<f32>(1.14, 1.06, 0.76);
    let cool = vec3<f32>(0.80, 0.96, 1.08);
    let lean = mix(cool, warm, up * (0.5 + clump * 0.5));
    let c = base.rgb;
    let k = mid * alive;
    let rgb = c * (1.0 + tone * alive) * mix(vec3<f32>(1.0), lean, k);
    return vec4<f32>(rgb, base.a);
}


// Burning vegetation (`vegetation.rs`, `paint`): a burning vertex keeps its
// colour and carries its ignition time in a negative alpha. Here the plant is
// eaten against the simulated clock (`foliage.fire.y`, flaming time in
// `foliage.fire.z`): scorch, then a glowing front that crawls through the crown
// in patches, red embers veining the char, leaves falling away. The trunk and
// boughs are the plant's own wood and char in place; what the fire adds (the
// twig skeleton inside the crown, `remains`, and the ash and fallen wood at
// its foot, `debris`) is shown where the front has passed, so the leaves open
// onto it and nothing is ever swapped. `prog` is 0 at ignition and 1 when the
// crown is gone.
const LEAF_OFFSET: f32 = 1.0e5;
const REMAINS_OFFSET: f32 = 2.0e5;
const DEBRIS_OFFSET: f32 = 4.0e5;

struct Burnt {
    rgb: vec3<f32>,
    glow: vec3<f32>,
    // 1 where the leaf has been eaten away
    gone: f32,
    hole: f32,
};

// How far the front has got at a point: below 0 not yet, above 0.3 the leaves
// there are mostly gone. Where the fire takes hold first is a property of the
// place, so patches of one crown light up at different moments and the front
// crawls. The skeleton uses the same field to show up where leaves go.
fn burn_front(p: vec3<f32>, prog: f32) -> f32 {
    let n = 0.45 * value_noise(p.xz * 0.8 + vec2<f32>(p.y * 0.5, 3.0))
          + 0.55 * value_noise(p.xz * 2.7 + vec2<f32>(-p.y * 1.9, 7.0));
    return prog * 1.6 - n * 0.9;
}

fn burn_plant(base: vec3<f32>, p: vec3<f32>, prog: f32, t: f32) -> Burnt {
    let l = burn_front(p, prog);

    let scorch = smoothstep(-0.35, 0.0, l);
    let hot = smoothstep(0.0, 0.07, l) * (1.0 - smoothstep(0.10, 0.36, l));
    let smoulder = smoothstep(0.08, 0.20, l) * (1.0 - smoothstep(0.30, 0.70, l));
    // Charred leaves do not hang around as a black crown: they go soon after.
    let charred = smoothstep(0.10, 0.30, l);
    let gone = smoothstep(0.20, 0.48, l);

    let flick = 0.70 + 0.55 * value_noise(p.xz * 2.4 + vec2<f32>(t * 5.0, t * 7.3 + p.y));
    // Embers show in veins and flecks of the char, not as a flat wash.
    let vein = smoothstep(0.50, 0.72, value_noise(p.xz * 3.6 + vec2<f32>(p.y * 2.7, 11.0)));
    let speck = smoothstep(0.78, 0.95, value_noise(p.xz * 9.0 + vec2<f32>(t * 0.6, p.y * 5.0)));

    var rgb = mix(base, base * vec3<f32>(0.80, 0.55, 0.30), scorch * 0.65);
    // warm soot, not neutral grey
    let ash = vec3<f32>(0.040, 0.027, 0.020) * (0.7 + 0.8 * value_noise(p.xz * 1.7 + p.y));
    rgb = mix(rgb, ash, charred);

    let hot_col = mix(vec3<f32>(3.6, 2.1, 0.45), vec3<f32>(2.5, 0.60, 0.07), smoothstep(0.0, 0.22, l));
    let ember_col = vec3<f32>(1.5, 0.20, 0.02);
    // Fades out as the plant approaches its charred remains.
    let late = 1.0 - smoothstep(0.80, 1.0, prog);
    let glow = (hot * hot_col * flick + smoulder * ember_col * (0.25 + 0.9 * vein + 1.2 * speck) * flick) * late;
    return Burnt(rgb, glow, gone, value_noise(p.xz * 3.1 + vec2<f32>(p.y * 2.3, 17.0)));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let t = globals.time;
    let p = in.world_position.xyz;
    var burn_glow = vec3<f32>(0.0);

    // Foliage is thin: lit from both sides, or a forest reads as a flat dark
    // mass from the shaded quarter.
    var pbr = pbr_types::pbr_input_new();
    pbr.flags = mesh[in.instance_index].flags;
    pbr.is_orthographic = view.clip_from_view[3].w == 1.0;
    pbr.V = pbr_functions::calculate_view(in.world_position, pbr.is_orthographic);
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    pbr.world_normal = pbr_functions::prepare_world_normal(in.world_normal, true, is_front);
    pbr.N = normalize(pbr.world_normal);
    pbr.material.metallic = 0.0;
    pbr.material.thickness = LEAF_THICKNESS;
    pbr.material.flags |= pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT
        | pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT;
    @if(VERTEX_COLORS)
    pbr.material.base_color = vec4<f32>(in.color.rgb, 1.0);
    @if(VERTEX_COLORS)
    let a = in.color.a;
    @else
    let a = 0.0;

    let px_m = max(length(dpdx(p)), length(dpdy(p)));
    let burning = a < -0.5;
    let b = -a;
    // decode: debris carries DEBRIS_OFFSET and remains REMAINS_OFFSET on
    // top of the ignition time; a standing leaf carries LEAF_OFFSET
    let debris = burning && b >= DEBRIS_OFFSET * 0.9;
    let b1 = b - select(0.0, DEBRIS_OFFSET, debris);
    let remains = burning && b1 > REMAINS_OFFSET * 0.75;
    let leaf = burning && !remains && b1 > LEAF_OFFSET * 0.5;
    let alive = select(clamp(a, 0.0, 1.0), select(0.0, 1.0, leaf), burning);
    pbr.material.base_color = foliage_albedo(pbr.material.base_color, p, in.world_normal, alive);
    pbr.material.perceptual_roughness = mix(WOOD_ROUGHNESS, LEAF_ROUGHNESS, alive);
    pbr.material.reflectance = vec3<f32>(mix(WOOD_REFLECTANCE, LEAF_REFLECTANCE, alive));
    pbr.material.diffuse_transmission = LEAF_TRANSMISSION * alive;
    if burning {
        let t0 = b1 - 1.0 - select(select(0.0, LEAF_OFFSET, leaf), REMAINS_OFFSET, remains);
        let prog = clamp((foliage.fire.y - t0) / max(foliage.fire.z, 1.0), 0.0, 1.0);
        // Holes are cut at every distance, in cells about a pixel wide,
        // so the crown thins out instead of darkening to grey.
        let cell = max(0.3, px_m * 1.5);
        let coarse = hash21(floor(p.xz / cell) + vec2<f32>(floor(p.y / cell) * 37.0, 5.0));
        let fine_h = value_noise(p.xz * 3.1 + vec2<f32>(p.y * 2.3, 17.0));
        let hole_n = mix(coarse, fine_h, 1.0 - smoothstep(1.0, 3.5, px_m));
        if remains {
            // What the fire adds: it is there from the start, hidden in
            // the crown, and shows where the front has eaten the leaves.
            let l = burn_front(p, prog);
            if debris {
                // ash and fallen wood last, patchily
                if prog + (hole_n - 0.5) * 0.4 < 0.70 {
                    discard;
                }
                // ash is soot, in patches: a pale solid disc reads as a
                // grey platform under the trees
                let drift = value_noise(p.xz * 0.7 + vec2<f32>(5.0, 9.0));
                if drift < 0.45 {
                    discard;
                }
                pbr.material.base_color = vec4<f32>(
                    pbr.material.base_color.rgb * vec3<f32>(0.20, 0.17, 0.15),
                    pbr.material.base_color.a);
            } else {
                if l < 0.26 {
                    discard;
                }
                // a faint ember still in the twigs the front has just left
                let ember = smoothstep(0.26, 0.40, l) * (1.0 - smoothstep(0.40, 0.95, l));
                burn_glow = vec3<f32>(1.5, 0.20, 0.02) * ember * 0.5
                    * (0.4 + 0.8 * value_noise(p.xz * 3.6 + vec2<f32>(t * 2.0, p.y * 2.7)));
            }
        } else {
            let r = burn_plant(pbr.material.base_color.rgb, p, prog, t);
            if leaf && hole_n < r.gone {
                discard;
            }
            pbr.material.base_color = vec4<f32>(r.rgb, pbr.material.base_color.a);
            burn_glow = r.glow;
        }
    }
    // Bark: wood is the brown (or charred, near-black) vertices that are
    // not leaves. Vertical, irregular ridges and fissures from the world
    // position, so every trunk and bough gets its own grain with no UVs
    // or texture; the fine layer fades with distance.
    let wc = pbr.material.base_color.rgb;
    let woody = !leaf && alive < 0.5 && wc.r >= wc.g && wc.g >= wc.b && max(wc.r, wc.g) < 0.55;
    if woody {
        let mid_b = 1.0 - smoothstep(2.0, 7.0, px_m);
        let fine_b = 1.0 - smoothstep(0.6, 2.5, px_m);
        let across = p.x * 5.0 + p.z * 3.7;
        let ridge = value_noise(vec2<f32>(across, p.y * 0.9));
        let grain = value_noise(vec2<f32>(across * 2.7 + 3.0, p.y * 3.2));
        let fissure = 1.0 - smoothstep(0.22, 0.46, ridge);
        let tone = (ridge - 0.5) * 1.1 * mid_b + (grain - 0.5) * 0.7 * fine_b - fissure * 0.6 * mid_b;
        pbr.material.base_color = vec4<f32>(wc * (1.0 + tone), pbr.material.base_color.a);
    }

    var out: FragmentOutput;
    out.color = pbr_functions::apply_pbr_lighting(pbr);
    // Fire is light, not a lit surface: added after lighting so a burning
    // crown glows on its shaded side too (and in the bloom pass).
    out.color = vec4<f32>(out.color.rgb + burn_glow, out.color.a);
    out.color = pbr_functions::main_pass_post_lighting_processing(pbr, out.color);
    out.color = tactical_grey(out.color, foliage.fire.x);
    return out;
}
