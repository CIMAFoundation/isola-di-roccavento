// Structures (`materials/structure.rs`): StandardMaterial, so a model keeps
// its own colour, maps and alpha, plus a procedural finish chosen by a shader
// def (one pipeline per finish, no branch per pixel): plaster, tiles and
// planks (`FINISH_TOWN`), dressed stone (`FINISH_STONE`), boats
// (`FINISH_BOAT`), the island's limestone (`FINISH_ROCK`). Without a finish it
// is StandardMaterial as Bevy draws it.
//
// Each part of a surface also gets its own PBR response (roughness,
// reflectance, metal, clear coat, glow): a finish tells its parts apart from
// the geometry and the colour, a baked model (the units) says per vertex what
// it is made of, in the vertex colour's alpha (`Surface` in `structure.rs`).


// What a baked vertex is made of: as `materials::Surface` (and the bakes'
// `SURFACES`). The alpha is id / 255; 1.0 (no id) leaves the material alone.

import bevy_pbr::render::pbr_functions::{alpha_discard, main_pass_post_lighting_processing};
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
import bevy_pbr::render::pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT};
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::mesh_functions::get_world_from_local;
import game::shaders::noise::{hash21, value_noise};
import game::shaders::scenery::{limestone, rock_bed, rock_bump};
import game::shaders::faded_shadow::apply_pbr_lighting_faded;

const SURFACE_PAINT: u32 = 1u;
const SURFACE_METAL: u32 = 2u;
const SURFACE_RUBBER: u32 = 3u;
const SURFACE_GLASS: u32 = 4u;
const SURFACE_LAMP: u32 = 5u;
const SURFACE_SKIN: u32 = 6u;
const SURFACE_CLOTH: u32 = 7u;
const SURFACE_REFLECTIVE: u32 = 8u;
const SURFACE_LEATHER: u32 = 9u;
const SURFACE_WOOD: u32 = 10u;
const SURFACE_SHUTTER: u32 = 11u;

fn surface_id(alpha: f32) -> u32 {
    let id = u32(round(alpha * 255.0));
    return select(0u, id, id < 32u);
}

// Sets the PBR response of a part, without a clear coat. `reflectance` is
// Bevy's (0.5 = 4 % at normal incidence, the dielectric norm).
fn set_pbr(pbr: ptr<function, PbrInput>, roughness: f32, metallic: f32, reflectance: f32) {
    (*pbr).material.perceptual_roughness = roughness;
    (*pbr).material.metallic = metallic;
    (*pbr).material.reflectance = vec3<f32>(reflectance);
    (*pbr).material.clearcoat = 0.0;
}

// A baked model's surfaces, with what makes a real one read at a distance:
// lacquered paint with a clear coat, metal that mirrors the sky, dark glass,
// lamps that glow, matte cloth, hi-vis strips that catch the light. `lp` is
// the object-space position (metres, y up from the ground): road dust climbs
// the lower part of a vehicle and the boots of a crew.
fn apply_surface(pbr: ptr<function, PbrInput>, id: u32, lp: vec3<f32>, p: vec3<f32>) {
    let c = (*pbr).material.base_color.rgb;
    let px_m = max(length(dpdx(p)), length(dpdy(p)));
    let fine = 1.0 - smoothstep(0.05, 0.4, px_m);
    let dust = (1.0 - smoothstep(0.15, 0.9, lp.y)) * (0.55 + 0.45 * value_noise(lp.xz * 6.0 + lp.y * 3.0));
    let dust_col = vec3<f32>(0.42, 0.36, 0.28);
    var rgb = c;
    switch id {
        case SURFACE_PAINT: {
            set_pbr(pbr, 0.30, 0.0, 0.5);
            (*pbr).material.clearcoat = 1.0;
            (*pbr).material.clearcoat_perceptual_roughness = 0.07;
            rgb = mix(c, dust_col, dust * 0.45);
            (*pbr).material.perceptual_roughness = mix(0.30, 0.8, dust);
            (*pbr).material.clearcoat = 1.0 - dust;
        }
        case SURFACE_METAL: {
            // brushed, a little uneven
            set_pbr(pbr, 0.22 + 0.16 * value_noise(lp.xz * 9.0 + lp.y * 14.0) * fine, 1.0, 0.5);
            rgb = mix(c, dust_col * 0.6, dust * 0.5);
        }
        case SURFACE_RUBBER: {
            set_pbr(pbr, 0.88, 0.0, 0.3);
            rgb = mix(c, dust_col * 0.5, dust * 0.6);
        }
        case SURFACE_GLASS: {
            set_pbr(pbr, 0.04, 0.0, 0.8);
        }
        case SURFACE_LAMP: {
            set_pbr(pbr, 0.15, 0.0, 0.5);
            (*pbr).material.emissive = vec4<f32>(c * 6.0, 1.0);
        }
        case SURFACE_SKIN: {
            set_pbr(pbr, 0.52, 0.0, 0.35);
        }
        case SURFACE_CLOTH: {
            // a weave too fine to see, but not too fine to break the sheen
            let weave = value_noise(lp.xz * 60.0 + lp.y * 45.0) - 0.5;
            rgb = c * (1.0 + weave * 0.10 * fine);
            rgb = mix(rgb, dust_col, dust * 0.35);
            set_pbr(pbr, 0.93, 0.0, 0.25);
        }
        case SURFACE_REFLECTIVE: {
            // retroreflective tape: bright toward any light, so it reads in shade
            set_pbr(pbr, 0.35, 0.0, 0.9);
            (*pbr).material.emissive = vec4<f32>(c * 0.35, 1.0);
        }
        case SURFACE_LEATHER: {
            set_pbr(pbr, 0.62, 0.0, 0.4);
            rgb = mix(c, dust_col, dust * 0.5);
        }
        case SURFACE_WOOD: {
            set_pbr(pbr, 0.75, 0.0, 0.35);
        }
        case SURFACE_SHUTTER: {
            // painted louvres: a lit slat, the shadow line under it
            let f = fract(p.y / 0.13);
            let slat = smoothstep(0.0, 0.3, f) * (1.0 - 0.6 * smoothstep(0.72, 1.0, f));
            let near = 1.0 - smoothstep(0.04, 0.2, px_m);
            rgb = c * mix(0.85, 0.5 + 0.7 * slat, near);
            set_pbr(pbr, 0.5, 0.0, 0.45);
        }
        default: {}
    }
    (*pbr).material.base_color = vec4<f32>(rgb, (*pbr).material.base_color.a);
}

// Object-space position in metres: the world offset from the mesh origin
// projected on its (orthogonal) axes. For merged meshes with an identity
// transform (all the town) it is simply the world position.
fn local_metres(p: vec3<f32>, instance_index: u32) -> vec3<f32> {
    let m = get_world_from_local(instance_index);
    let d = p - m[3].xyz;
    let ax = normalize(m[0].xyz);
    let ay = normalize(m[1].xyz);
    let az = normalize(m[2].xyz);
    return vec3<f32>(dot(d, ax), dot(d, ay), dot(d, az));
}

// Exposed so windows (dark glass), lit windows and burning or charred
// vertices keep their colour: only ordinary painted surfaces get a texture.
fn surface_gate(c: vec3<f32>) -> f32 {
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return smoothstep(0.09, 0.16, luma) * (1.0 - smoothstep(1.0, 1.3, max(c.r, max(c.g, c.b))));
}

// Town: plaster, roof tiles, flat-roof gravel and the
// planks of the pier, all in world metres.
fn town_albedo(base: vec4<f32>, p: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let n = normalize(normal);
    let px_m = max(length(dpdx(p)), length(dpdy(p)));
    let fine = 1.0 - smoothstep(0.25, 1.2, px_m);
    let mid = 1.0 - smoothstep(0.9, 4.0, px_m);
    let c = base.rgb;
    let gate = surface_gate(c);
    var tone = 0.0;
    var rgb = c;
    let wood = c.r > c.g && c.g > c.b && c.r < 0.6 && c.r - c.b > 0.12 && c.r - c.b < 0.3;
    if n.y > 0.35 && wood {
        // Pier and decking: planks across the walkway, each with its own
        // tone, a dark gap between them and a long grain.
        let row = floor(p.z / 0.28);
        let f = fract(p.z / 0.28);
        let gap = 1.0 - smoothstep(0.0, 0.12 + 0.5 * px_m, min(f, 1.0 - f) * 2.0);
        let grain = value_noise(vec2<f32>(p.x * 5.0, p.z * 40.0));
        tone = (hash21(vec2<f32>(row, 3.0)) - 0.5) * 0.35 + (grain - 0.5) * 0.25 * fine - gap * 0.35 * mid;
    } else if n.y > 0.35 {
        // Roofs. Pitched ones: courses of clay tiles along the slope, rows
        // at constant height, columns along the eave. Flat ones: gravel.
        let along = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-4, 0.0));
        let u = dot(p.xz, along) / 0.34;
        let v = p.y / (0.30 * max(n.y, 0.25) / max(length(n.xz), 0.25) + 0.12);
        if n.y < 0.93 {
            let rowi = floor(v);
            let uu = u + 0.5 * (rowi - 2.0 * floor(rowi * 0.5));
            let col = hash21(vec2<f32>(floor(uu), rowi));
            let fv = fract(v);
            let fu = fract(uu);
            let lip = smoothstep(0.0, 0.35, fv) * (1.0 - 0.45 * smoothstep(0.6, 1.0, fv)) ;
            let seam = 1.0 - smoothstep(0.0, 0.2 + 0.4 * px_m, min(fu, 1.0 - fu));
            let moss = smoothstep(0.55, 0.85, value_noise(p.xz * 0.5 + 11.0));
            tone = (col - 0.5) * 0.40 * mid + (lip - 0.75) * 0.55 * mid - seam * 0.22 * fine - moss * 0.08 * mid
                 + (value_noise(p.xz * 0.12) - 0.5) * 0.25;
        } else {
            let grit = value_noise(p.xz * 6.0);
            tone = (value_noise(p.xz * 0.35) - 0.5) * 0.28 + (grit - 0.5) * 0.30 * fine;
        }
    } else {
        // Walls: lime plaster. Large damp and sun patches, small stucco
        // grain, vertical runs of dirt under the eaves and a darker foot.
        let blot = value_noise(vec2<f32>((p.x + p.z) * 0.22, p.y * 0.30) + 3.0);
        let grain = value_noise(vec2<f32>(p.x + p.z, p.y) * 3.5);
        let streak = value_noise(vec2<f32>((p.x + p.z) * 2.2, p.y * 0.18));
        tone = (blot - 0.5) * 0.30 * mid + (grain - 0.5) * 0.22 * fine - smoothstep(0.62, 0.9, streak) * 0.12 * mid;
        // stone plinth: the lowest metre or so reads darker
        let foot = 1.0 - smoothstep(0.0, 1.6, p.y - 0.0);
        tone -= 0.0 * foot;
    }
    rgb = c * (1.0 + tone * gate);
    return vec4<f32>(rgb, base.a);
}

// Stone: dressed ashlar in running bond, in the object's
// own frame so the courses follow the walls, with lichen on the top faces.
fn stone_albedo(base: vec4<f32>, lp: vec3<f32>, normal: vec3<f32>, p: vec3<f32>) -> vec4<f32> {
    let n = normalize(normal);
    let px_m = max(length(dpdx(p)), length(dpdy(p)));
    let fine = 1.0 - smoothstep(0.35, 1.6, px_m);
    let mid = 1.0 - smoothstep(1.2, 6.0, px_m);
    let c = base.rgb;
    let gate = surface_gate(c);
    let along = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-4, 0.0));
    var q: vec2<f32>;
    if abs(n.y) > 0.7 {
        q = lp.xz / vec2<f32>(1.6, 1.0);
    } else {
        q = vec2<f32>(dot(lp.xz, along) / 1.6, lp.y / 0.8);
    }
    let row = floor(q.y);
    let qq = vec2<f32>(q.x + 0.5 * (row - 2.0 * floor(row * 0.5)) + hash21(vec2<f32>(row, 1.0)), q.y);
    let cell = floor(qq);
    let f = fract(qq);
    let e = min(min(f.x * 1.6, (1.0 - f.x) * 1.6), min(f.y * 0.8, (1.0 - f.y) * 0.8));
    let mortar = 1.0 - smoothstep(0.02, 0.05 + 0.6 * px_m, e);
    let block = (hash21(cell) - 0.5) * 0.30 + (value_noise(lp.xz * 1.3 + lp.y * 0.7) - 0.5) * 0.18 * fine;
    let weather = (value_noise(lp.xz * 0.15 + lp.y * 0.1) - 0.5) * 0.30;
    let lichen = smoothstep(0.55, 0.8, value_noise(lp.xz * 0.8 + 5.0)) * smoothstep(0.2, 0.8, n.y);
    var rgb = c * (1.0 + (block - mortar * 0.38) * mid * gate + weather * gate);
    rgb = mix(rgb, rgb * vec3<f32>(0.78, 0.92, 0.55), lichen * 0.55 * gate * mid);
    return vec4<f32>(rgb, base.a);
}

// Bare rock: the island's own limestone on meshes that are not
// terrain (the crag under the rocca, the coast's boulders). Same beds as the
// terrain's rock faces and, because they are cut from the pixel's world height,
// the same bands at the same altitudes: the crag reads as the mountain's
// bedrock coming through, not as a separate object. The vertex colour only
// tints (so shaded and sunlit blocks still differ).
fn rock_albedo(base: vec4<f32>, p: vec3<f32>, nrm: vec3<f32>) -> vec4<f32> {
    let n = normalize(nrm);
    let mpp = max(length(dpdx(p)), length(dpdy(p)));
    let fine = 1.0 - smoothstep(1.0, 3.5, mpp);
    let rb = rock_bed(p, n);
    let along = rb.x;
    let yy = rb.y;
    let u = rb.z;
    let bed = value_noise(vec2<f32>(along * 0.10, yy * 0.8));
    let seam = smoothstep(0.86, 1.0, abs(fract(u) - 0.5) * 2.0);
    let joint_off = hash21(vec2<f32>(floor(u), 3.0));
    let joint = smoothstep(0.93, 1.0, abs(fract(along * 0.45 + joint_off) - 0.5) * 2.0)
        * (1.0 - smoothstep(0.0, 1.0, abs(fract(u) - 0.5) * 2.0));
    let tone = 0.80 + 0.34 * bed + 0.10 * (value_noise(vec2<f32>(along * 0.7, yy * 0.25)) - 0.5);
    // the baked colour sets the brightness (warm grey, darker than the terrain's pale
    // faces and the ochre masonry); the limestone beds only modulate it, relative to
    // their mean, so the bands line up with the mountain's without its glare
    let beds = limestone(p) / vec3<f32>(0.59, 0.54, 0.45);
    var rock = base.rgb * 1.55 * mix(vec3<f32>(1.0), beds, 0.85) * (0.94 + 0.12 * value_noise(p.xz * 0.5)) * tone
             * (1.0 - fine * (0.42 * seam + 0.24 * joint)) * vec3<f32>(1.0, 0.95, 0.88);
    // a little lichen and dust on the upward faces
    let lichen = smoothstep(0.55, 0.8, value_noise(p.xz * 0.6 + 11.0)) * smoothstep(0.5, 0.9, n.y);
    rock = mix(rock, rock * vec3<f32>(0.82, 0.93, 0.62), lichen * 0.45);
    return vec4<f32>(rock, base.a);
}

// Boats: planked hulls, panelled sails. Told apart by
// the baked colour: pale and neutral is cloth, anything else is timber or paint.
fn boat_albedo(base: vec4<f32>, lp: vec3<f32>, normal: vec3<f32>, p: vec3<f32>) -> vec4<f32> {
    let n = normalize(normal);
    let px_m = max(length(dpdx(p)), length(dpdy(p)));
    let fine = 1.0 - smoothstep(0.4, 2.5, px_m);
    let mid = 1.0 - smoothstep(1.5, 8.0, px_m);
    let c = base.rgb;
    let gate = surface_gate(c);
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sat = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
    let cloth = smoothstep(0.55, 0.75, luma) * (1.0 - smoothstep(0.12, 0.3, sat)) * (1.0 - smoothstep(0.55, 0.9, abs(n.y)));
    var tone = 0.0;
    if cloth > 0.5 && abs(n.y) < 0.8 {
        // Sail: cloths sewn in vertical panels with a seam, billowing shade
        // and a sun-bleached upper edge.
        let panel = lp.x * 0.7 + lp.z * 0.7;
        let seam = 1.0 - smoothstep(0.0, 0.05 + 0.5 * px_m, abs(fract(panel / 0.55) - 0.5) * 2.0 - 0.0);
        let seam_line = smoothstep(0.92, 1.0, abs(fract(panel / 0.55) - 0.5) * 2.0);
        tone = -seam_line * 0.30 * mid + (value_noise(vec2<f32>(panel * 0.6, lp.y * 0.5)) - 0.5) * 0.22 * mid
             + (value_noise(vec2<f32>(panel * 4.0, lp.y * 4.0)) - 0.5) * 0.07 * fine;
    } else if abs(n.y) > 0.6 {
        // Deck: teak boards along the boat.
        let row = floor(lp.z / 0.22);
        let f = fract(lp.z / 0.22);
        let gap = 1.0 - smoothstep(0.0, 0.1 + 0.5 * px_m, min(f, 1.0 - f) * 2.0);
        tone = (hash21(vec2<f32>(row, 9.0)) - 0.5) * 0.35 * mid - gap * 0.50 * mid
             + (value_noise(vec2<f32>(lp.x * 3.0, lp.z * 30.0)) - 0.5) * 0.14 * fine;
    } else {
        // Hull: strakes of planking, a slightly glossy paint, grain.
        let f = fract(lp.y / 0.28);
        let gap = 1.0 - smoothstep(0.0, 0.1 + 0.5 * px_m, min(f, 1.0 - f) * 2.0);
        tone = -gap * 0.45 * mid + (hash21(vec2<f32>(floor(lp.y / 0.28), 4.0)) - 0.5) * 0.22 * mid
             + (value_noise(vec2<f32>(lp.x * 2.5, lp.y * 24.0)) - 0.5) * 0.10 * fine;
    }
    return vec4<f32>(c * (1.0 + tone * gate), base.a);
}

// The town's parts as `town_albedo` tells them apart, given the baked colour
// `c` and the face normal: tiles glazed enough to catch the sun, plaster and
// gravel matte, planks between, window glass a mirror, lit windows a light.
fn town_pbr(pbr: ptr<function, PbrInput>, c: vec3<f32>, normal: vec3<f32>) {
    let n = normalize(normal);
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let wood = c.r > c.g && c.g > c.b && c.r < 0.6 && c.r - c.b > 0.12 && c.r - c.b < 0.3;
    if max(c.r, max(c.g, c.b)) > 1.0 {
        (*pbr).material.emissive = vec4<f32>(c * 0.8, 1.0);
    } else if luma < 0.09 {
        set_pbr(pbr, 0.06, 0.0, 0.8);
    } else if wood {
        set_pbr(pbr, 0.72, 0.0, 0.35);
    } else if n.y > 0.35 && n.y < 0.93 {
        set_pbr(pbr, 0.55, 0.0, 0.45);
    } else if n.y > 0.35 {
        set_pbr(pbr, 0.95, 0.0, 0.3);
    } else {
        set_pbr(pbr, 0.88, 0.0, 0.3);
    }
}

// Boats: sails matte cloth, decks oiled teak, hulls glossy paint.
fn boat_pbr(pbr: ptr<function, PbrInput>, c: vec3<f32>, normal: vec3<f32>) {
    let n = normalize(normal);
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sat = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
    let cloth = smoothstep(0.55, 0.75, luma) * (1.0 - smoothstep(0.12, 0.3, sat)) * (1.0 - smoothstep(0.55, 0.9, abs(n.y)));
    if cloth > 0.5 && abs(n.y) < 0.8 {
        set_pbr(pbr, 0.9, 0.0, 0.3);
    } else if abs(n.y) > 0.6 {
        set_pbr(pbr, 0.6, 0.0, 0.4);
    } else {
        set_pbr(pbr, 0.32, 0.0, 0.5);
        (*pbr).material.clearcoat = 0.6;
        (*pbr).material.clearcoat_perceptual_roughness = 0.12;
    }
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    @if(VERTEX_COLORS)
    let surface = surface_id(in.color.a);
    @else
    let surface = 0u;
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    let p = in.world_position.xyz;
    let c = pbr.material.base_color;
    @if(FINISH_TOWN) {
        town_pbr(&pbr, c.rgb, in.world_normal);
        pbr.material.base_color = town_albedo(c, p, in.world_normal);
    } @elif(FINISH_STONE) {
        set_pbr(&pbr, 0.85, 0.0, 0.3);
        pbr.material.base_color = stone_albedo(c, local_metres(p, in.instance_index), in.world_normal, p);
    } @elif(FINISH_BOAT) {
        boat_pbr(&pbr, c.rgb, in.world_normal);
        pbr.material.base_color = boat_albedo(c, local_metres(p, in.instance_index), in.world_normal, p);
    } @elif(FINISH_ROCK) {
        set_pbr(&pbr, 0.9, 0.0, 0.3);
        pbr.material.base_color = rock_albedo(c, p, in.world_normal);
        pbr.N = normalize(pbr.N + rock_bump(p, normalize(in.world_normal)));
    }
    if surface != 0u {
        apply_surface(&pbr, surface, local_metres(p, in.instance_index), p);
    }

    var out: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting_faded(pbr);
    } else {
        out.color = pbr.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr, out.color);
    return out;
}
