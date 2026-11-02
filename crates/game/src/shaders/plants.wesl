// The plants' main pass (`vegetation/render.rs`): each plant an instance of
// its variety's mesh, placed and given its fire state in the vertex stage
// (`plant_common.wesl`), painted from the foliage atlas and lit for
// foliage rather than by the full PBR:
//
// - wrapped diffuse (light wraps round a crown, it does not stop at the
//   terminator like on a stone);
// - the sun through the leaves, toward the eye: a crown against the light
//   glows green-gold;
// - sky above and warm ground below as ambient, darkened toward the heart
//   and the bottom of the crown (the vertex occlusion);
// - the sun's cascaded shadows, Bevy's fog and tonemapping as the rest of
//   the scene has them.
//
// Burning: scorch, then a glowing front that crawls through the crown in
// patches, red embers veining the char, the leaves falling away (holes in
// the cut-out pass, where the leaf cards are cut; elsewhere the crown chars
// and slumps, `plant_common.wesl`). What the fire adds (the twig skeleton,
// ash and fallen wood) shows where the front has passed.

import bevy_pbr::render::mesh_view_bindings::{view, lights};
import bevy_pbr::render::mesh_view_types;
@if(PLANT_SHADOWS) import bevy_pbr::render::shadows;
@if(PLANT_SHADOWS) import game::shaders::scene_light::{view_depth, shadow_reach};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::{pbr_functions, pbr_types};
import game::shaders::noise::{hash21, value_noise};
import game::shaders::scenery::tactical_grey;
import game::shaders::plant_common::{plants, atlas, atlas_sampler, PlantVertex, place, GONE, ALPHA_CUT, CROWN_SHARE, kind_leaf, kind_remains, kind_debris};

// Normal and uv at the centroid: with MSAA a pixel only partly covered by a
// thin leaf card is shaded at its centre, outside the triangle, and the
// extrapolated normal (longer than 1: a sheen of 2^28) and uv (another
// atlas tile) made single white pixels (2026-11-01).
struct PlantOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(perspective, centroid) normal: vec3<f32>,
    @location(2) @interpolate(perspective, centroid) uv: vec2<f32>,
    // what the atlas texel is multiplied by: the plant's tint and shade,
    // the model's shading (on remains, their own colour)
    @location(3) albedo: vec3<f32>,
    // x: what the vertex is; y: burn (-1 standing, 0..1 burning, 2 burnt);
    // z: occlusion; w: the climb (`Placed::climb`)
    @location(4) info: vec4<f32>,
}

@vertex
fn vertex(v: PlantVertex) -> PlantOut {
    var out: PlantOut;
    let p = place(v);
    out.clip = select(position_world_to_clip(p.world), GONE, p.gone);
    out.world = p.world;
    out.normal = p.normal;
    out.uv = v.uv;
    let kind = v.normal.w;
    // colour and tint both travel halved
    var albedo = v.color.rgb * 2.0;
    if !kind_remains(kind) {
        albedo *= select(vec3<f32>(v.tint.a), v.tint.rgb, kind_leaf(kind)) * 2.0;
    } else if kind_debris(kind) {
        // ash is soot: the model's pale grey would read as a platform under the trees
        albedo *= vec3<f32>(0.2, 0.17, 0.15);
    }
    // no two plants of a stand quite the same tone
    out.albedo = albedo * (0.9 + 0.2 * v.seed.z);
    out.info = vec4<f32>(kind, p.burn, v.color.a, p.climb);
    return out;
}

// --- fire ----------------------------------------------------------------

struct Burnt {
    rgb: vec3<f32>,
    glow: vec3<f32>,
    // 1 where the leaf has been eaten away
    gone: f32,
}

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

// `prog`: 0 when the climb reaches the point, 1 when its leaves are gone;
// `after`: 0 then, 1 when the plant turns to charcoal.
//
// 2026-11-01, the middle of the fire as reviewed on main (`KIOSK_PIANTE`):
// the leaves darken under the flames, the flames are a narrow band in
// tongues that rise and flicker, and behind them the char keeps red veins;
// a broad hot band on a sunlit crown read as pastel salmon.
fn burn_plant(base: vec3<f32>, p: vec3<f32>, prog: f32, after: f32, t: f32) -> Burnt {
    let l = burn_front(p, prog);
    // ahead of the front the leaves dry and brown (not orange: that is light)
    let scorch = smoothstep(-0.30, 0.0, l);
    // the flames: a narrow band, cut into tongues that rise and flicker
    let band = smoothstep(-0.02, 0.04, l) * (1.0 - smoothstep(0.08, 0.20, l));
    let tongue = value_noise(vec2<f32>(p.x * 1.3 + p.z * 0.7, p.y * 0.45 - t * 2.6)) * 0.6
               + 0.4 * value_noise(vec2<f32>(p.z * 2.9 - p.x * 0.4, p.y * 1.1 - t * 4.1));
    let hot = band * smoothstep(0.30, 0.62, tongue);
    let flick = 0.55 + 0.9 * value_noise(p.xz * 3.1 + vec2<f32>(t * 6.0, t * 8.3 + p.y));
    // behind the flames: char with red veins, dimming
    let smoulder = smoothstep(0.06, 0.14, l) * (1.0 - smoothstep(0.22, 0.45, l));
    let charred = smoothstep(0.02, 0.16, l);
    let gone = smoothstep(0.20, 0.48, l);
    // embers are lines and dots in the char, never a coat of paint on a leaf
    let vein = smoothstep(0.66, 0.80, value_noise(p.xz * 4.8 + vec2<f32>(p.y * 3.4, 11.0)));
    let speck = smoothstep(0.80, 0.96, value_noise(p.xz * 9.0 + vec2<f32>(t * 0.6, p.y * 5.0)));

    var rgb = mix(base, base * vec3<f32>(0.50, 0.38, 0.20), scorch * 0.85);
    // warm soot, not neutral grey; under the flames too, so they burn on dark
    let ash = vec3<f32>(0.035, 0.024, 0.018) * (0.7 + 0.8 * value_noise(p.xz * 1.7 + p.y));
    rgb = mix(rgb, ash, max(charred, band * 0.85));

    // yellow at the root of a tongue, deep orange at its tips
    let hot_col = mix(vec3<f32>(3.2, 1.5, 0.22), vec3<f32>(2.6, 0.55, 0.05), smoothstep(0.45, 0.80, tongue));
    let ember_col = vec3<f32>(1.2, 0.16, 0.015);
    // the embers die down before the plant turns to charcoal
    let late = 1.0 - smoothstep(0.0, 0.85, after);
    let glow = hot * hot_col * flick + smoulder * ember_col * (0.7 * vein + 1.2 * speck) * flick * late;
    return Burnt(rgb, glow, gone);
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

// --- light ---------------------------------------------------------------

// The light a plant's surface sends to the eye. `leafy` is 1 on living
// leaves (wrap, light through, a faint sheen), 0 on wood and char.
fn foliage_light(albedo: vec3<f32>, n: vec3<f32>, world: vec3<f32>, frag: vec4<f32>, leafy: f32, occlusion: f32) -> vec3<f32> {
    let v = normalize(view.world_position.xyz - world);
    // sky above, warm ground below
    let ambient = lights.ambient_color.rgb;
    let hemi = mix(ambient * vec3<f32>(0.62, 0.56, 0.46), ambient * vec3<f32>(1.0, 1.04, 1.12), n.y * 0.5 + 0.5);
    var lit = albedo * hemi * occlusion;
    if lights.n_directional_lights > 0u {
        let light = lights.directional_lights[0];
        let l = light.direction_to_light;
        let ndl = dot(n, l);
        let wrap = mix(0.15, 0.5, leafy);
        let diffuse = max((ndl + wrap) / (1.0 + wrap), 0.0);
        var shadow = 1.0;
        @if(PLANT_SHADOWS)
        if (light.flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            let view_z = view_depth(world);
            shadow = shadows::fetch_directional_shadow(0u, vec4<f32>(world, 1.0), n, view_z, frag.xy);
            // faded out toward the end of the shadows' reach, as the ground
            shadow = mix(1.0, shadow, shadow_reach(view_z));
        }
        // the sun through the leaves, toward the eye, tinted by them
        let through = (pow(max(dot(-v, l), 0.0), 4.0) * 0.7 + max(-ndl, 0.0) * 0.25) * leafy;
        let leaf_light = albedo * (0.6 + 2.5 * albedo);
        let h = normalize(l + v);
        let sheen = pow(max(dot(n, h), 0.0), 28.0) * 0.05 * leafy;
        let sun = light.color.rgb * shadow * 0.31831;
        lit += sun * (albedo * diffuse * mix(1.0, occlusion, 0.5) + leaf_light * through * 0.8 + vec3<f32>(sheen));
    }
    return lit * view.exposure;
}

@fragment
fn fragment(in: PlantOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // the texel and the derivatives first, in uniform control flow
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    let tex_width = fwidth(tex.a);
    let p = in.world;
    let px_m = max(length(dpdx(p)), length(dpdy(p)));
    let t = plants.wind.w;
    let kind = in.info.x;
    let burn = in.info.y;
    let leaf = kind_leaf(kind);
    let remains = kind_remains(kind);
    let burning = burn >= 0.0 && burn <= 1.0;
    let burnt = burn > 1.5;
    // the fire at this point of the plant (`plant_common.wesl`, the climb)
    let prog = clamp(in.info.w, 0.0, 1.0);
    let after = clamp((in.info.w - 1.0) / (1.0 / CROWN_SHARE - 1.0), 0.0, 1.0);

    var alpha = 1.0;
    @if(PLANT_CUTOUT) {
        if leaf {
            @if(PLANT_A2C) {
                // alpha to coverage, sharpened to the pixel: crisp leaf
                // edges, smoothed by the samples
                alpha = clamp((tex.a - ALPHA_CUT) / max(tex_width, 1.0e-4) + 0.5, 0.0, 1.0);
            }
            @else {
                if tex.a < ALPHA_CUT {
                    discard;
                }
            }
        }
    }

    var albedo = in.albedo * tex.rgb;
    var glow = vec3<f32>(0.0);
    var leafy = select(0.0, 1.0, leaf);
    if remains {
        leafy = 0.0;
        if kind_debris(kind) {
            // ash and fallen wood last, in patches
            @if(PLANT_CUTOUT) {
                if prog + (hole_at(p, px_m) - 0.5) * 0.4 < 0.70 || value_noise(p.xz * 0.7 + vec2<f32>(5.0, 9.0)) < 0.45 {
                    discard;
                }
            }
        } else {
            // the skeleton shows where the front has eaten the leaves; the
            // twigs it has just left glow, then keep a few embers in their
            // joints until the plant is charcoal
            let pr = select(1.0, prog, burning);
            let l = burn_front(p, pr);
            @if(PLANT_CUTOUT) {
                if l < 0.26 {
                    discard;
                }
            }
            let ember = smoothstep(0.26, 0.40, l) * (1.0 - smoothstep(0.40, 0.95, l));
            let joints = smoothstep(0.62, 0.85, value_noise(p.xz * 4.3 + vec2<f32>(p.y * 3.1, 23.0)));
            let lasting = smoothstep(0.40, 0.70, l) * joints * (1.0 - smoothstep(0.2, 0.9, after));
            glow = vec3<f32>(1.5, 0.20, 0.02) * (ember * 0.5 + lasting * 0.8) * select(0.0, 1.0, burning)
                * (0.4 + 0.8 * value_noise(p.xz * 3.6 + vec2<f32>(t * 2.0, p.y * 2.7)));
        }
    } else if burnt {
        // charcoal keeps the bark's light and dark, not its colour
        let luma = dot(tex.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let c = in.albedo * 0.5;
        albedo = (vec3<f32>(0.022, 0.019, 0.017) + c * vec3<f32>(0.03, 0.025, 0.025)) * clamp(luma * 6.0, 0.4, 1.6);
        leafy = 0.0;
    } else if burning {
        let r = burn_plant(albedo, p, prog, after, t);
        @if(PLANT_CUTOUT) {
            if leaf && hole_at(p, px_m) < r.gone {
                discard;
            }
        }
        albedo = r.rgb;
        glow = r.glow;
        leafy *= 1.0 - r.gone;
    }

    // a leaf card keeps its crown's normal, seen from either side; wood is
    // lit on the side seen
    let n0 = normalize(in.normal + vec3<f32>(0.0, 1.0e-4, 0.0));
    let n = select(-n0, n0, front || leaf);
    var color = vec4<f32>(foliage_light(albedo, n, p, in.clip, leafy, clamp(in.info.z, 0.0, 1.0)), alpha);
    // fire is light, not a lit surface: a burning crown glows on its shaded
    // side too (and in the bloom)
    color = vec4<f32>(color.rgb + glow, color.a);
    var pbr = pbr_types::pbr_input_new();
    pbr.world_position = vec4<f32>(p, 1.0);
    pbr.frag_coord = in.clip;
    pbr.material.flags = pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    color = pbr_functions::main_pass_post_lighting_processing(pbr, color);
    return tactical_grey(color, plants.clock.w);
}
