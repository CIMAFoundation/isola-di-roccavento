// The houses of the towns in the realistic style (`materials/house.rs`,
// `KIOSK_SCANS=1`): photographed plaster, clay and slate, rubble stone and
// brick (Poly Haven, CC0), mapped in world metres on the faces' own axes,
// with the weather: damp rising from the foot, dirt run down from the eaves,
// plaster flaked off to the brick. The vertex colour stays the tint, so the
// fire's recolouring (`buildings::damage`) reads through: a charred house is
// its own plaster and tiles gone black, a burning one glows.
//
// What a vertex is comes from the vertex colour's alpha (`surface_alpha`):
// the structures' ids 1-11 (4 glass, 11 shutters), `masonry::Part` 12-16 and
// `house::HousePart` 20-24. The first uv is the house's: the height of the
// ground at its foot, and a number of its own in 0..1.
// Every texture is sampled with explicit gradients from the world position's
// derivatives, taken once at the top: no derivative after a branch (the
// browser's WGSL refuses them).

import bevy_pbr::render::pbr_functions::{alpha_discard, main_pass_post_lighting_processing};
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
import bevy_pbr::render::pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT};
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import game::shaders::noise::{hash21, value_noise};
import game::shaders::faded_shadow::apply_pbr_lighting_faded;

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var house_color: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var house_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var house_normal: texture_2d_array<f32>;

const SURFACE_PAINT: u32 = 1u;
const SURFACE_METAL: u32 = 2u;
const SURFACE_RUBBER: u32 = 3u;
const SURFACE_GLASS: u32 = 4u;
const SURFACE_CLOTH: u32 = 7u;
const SURFACE_WOOD: u32 = 10u;
const SURFACE_SHUTTER: u32 = 11u;
const PART_PLASTER: u32 = 12u;
const PART_TILES: u32 = 13u;
const PART_STONE: u32 = 14u;
const PART_BUILDING: u32 = 15u;
const PART_FRAME: u32 = 16u;
const HOUSE_PLASTER: u32 = 20u;
const HOUSE_SLATE: u32 = 21u;
const HOUSE_RUSTIC: u32 = 22u;
const HOUSE_PAINT: u32 = 23u;
const HOUSE_REVEAL: u32 = 24u;

// texture layers (`house.rs`, LAYERS)
const L_NONE: i32 = -1;
const L_PLASTER: i32 = 0;
const L_ROOF: i32 = 1;
const L_STONE: i32 = 2;
const L_SLATE: i32 = 3;
const L_RUSTIC: i32 = 4;
const L_BRICK: i32 = 5;
const L_PLASTER2: i32 = 6;
const L_ROOF2: i32 = 7;

// Mean linear colour of each photograph (`scripts/pack_building_textures.py`
// prints them): the detail is the photo over its mean, so a wall's average is
// its vertex colour.
const MEAN_PLASTER: vec3<f32> = vec3<f32>(0.3843, 0.3455, 0.3313);
const MEAN_ROOF: vec3<f32> = vec3<f32>(0.2391, 0.064, 0.0174);
const MEAN_STONE: vec3<f32> = vec3<f32>(0.3763, 0.2932, 0.2072);
const MEAN_SLATE: vec3<f32> = vec3<f32>(0.1899, 0.1625, 0.1252);
const MEAN_RUSTIC: vec3<f32> = vec3<f32>(0.2075, 0.1497, 0.0913);
const MEAN_BRICK: vec3<f32> = vec3<f32>(0.2059, 0.1453, 0.0844);
const MEAN_PLASTER2: vec3<f32> = vec3<f32>(0.3678, 0.3638, 0.3372);
const MEAN_ROOF2: vec3<f32> = vec3<f32>(0.224, 0.0744, 0.0294);

// Metres of world one repeat of each photograph covers (the houses are drawn
// larger than life, `visual_scale`, and their materials with them).
const SIZE_PLASTER: f32 = 3.0;
const SIZE_ROOF: f32 = 4.6;
const SIZE_STONE: f32 = 4.0;
const SIZE_SLATE: f32 = 11.0;
const SIZE_RUSTIC: f32 = 4.5;
const SIZE_BRICK: f32 = 3.5;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn surface_id(alpha: f32) -> u32 {
    let id = u32(round(alpha * 255.0));
    return select(0u, id, id < 32u);
}

fn set_pbr(pbr: ptr<function, PbrInput>, roughness: f32, metallic: f32, reflectance: f32) {
    (*pbr).material.perceptual_roughness = roughness;
    (*pbr).material.metallic = metallic;
    (*pbr).material.reflectance = vec3<f32>(reflectance);
    (*pbr).material.clearcoat = 0.0;
}

fn mean_of(layer: i32) -> vec3<f32> {
    switch layer {
        case 1: { return MEAN_ROOF; }
        case 2: { return MEAN_STONE; }
        case 3: { return MEAN_SLATE; }
        case 4: { return MEAN_RUSTIC; }
        case 5: { return MEAN_BRICK; }
        case 6: { return MEAN_PLASTER2; }
        case 7: { return MEAN_ROOF2; }
        default: { return MEAN_PLASTER; }
    }
}

fn size_of(layer: i32) -> f32 {
    switch layer {
        case 1, 7: { return SIZE_ROOF; }
        case 2: { return SIZE_STONE; }
        case 3: { return SIZE_SLATE; }
        case 4: { return SIZE_RUSTIC; }
        case 5: { return SIZE_BRICK; }
        default: { return SIZE_PLASTER; }
    }
}

// Props of the town kit (benches, cars, umbrellas, the pier): as the
// structures' `FINISH_TOWN` draws them, which they keep.
fn prop_albedo(c: vec3<f32>, p: vec3<f32>, n: vec3<f32>, px_m: f32) -> vec3<f32> {
    let fine = 1.0 - smoothstep(0.25, 1.2, px_m);
    let mid = 1.0 - smoothstep(0.9, 4.0, px_m);
    let gate = smoothstep(0.09, 0.16, luma(c)) * (1.0 - smoothstep(1.0, 1.3, max(c.r, max(c.g, c.b))));
    var tone = 0.0;
    let wood = c.r > c.g && c.g > c.b && c.r < 0.6 && c.r - c.b > 0.12 && c.r - c.b < 0.3;
    if n.y > 0.35 && wood {
        let row = floor(p.z / 0.28);
        let f = fract(p.z / 0.28);
        let gap = 1.0 - smoothstep(0.0, 0.12 + 0.5 * px_m, min(f, 1.0 - f) * 2.0);
        let grain = value_noise(vec2<f32>(p.x * 5.0, p.z * 40.0));
        tone = (hash21(vec2<f32>(row, 3.0)) - 0.5) * 0.35 + (grain - 0.5) * 0.25 * fine - gap * 0.35 * mid;
    } else {
        let grain = value_noise(vec2<f32>(p.x + p.z, p.y) * 3.5);
        tone = (grain - 0.5) * 0.12 * fine;
    }
    return c * (1.0 + tone * gate);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    @if(VERTEX_COLORS)
    let surface = surface_id(in.color.a);
    @else
    let surface = 0u;
    // the house's ground and its own number (none: the kit's props)
    var foot = -1.0e4;
    var seed = 0.5;
    @if(VERTEX_UVS_A) {
        foot = in.uv.x;
        seed = in.uv.y;
    }
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    let p = in.world_position.xyz;
    let c = pbr.material.base_color.rgb;
    let n = normalize(pbr.N);
    // derivatives first, before anything branches
    let dpx = dpdx(p);
    let dpy = dpdy(p);
    let px_m = max(length(dpx), length(dpy));
    // The photographs are laid on the face's axes, read from its (flat)
    // vertex normal: never from the derivatives of a position kilometres
    // from the origin (a percent off, and the texture slides tens of metres).
    let ng = n;

    // what the pixel is, from its part and its face
    let slope = n.y > 0.35 && n.y < 0.93;
    let worn = seed;
    var layer = L_NONE;
    let plaster = select(L_PLASTER, L_PLASTER2, fract(seed * 7.13) < 0.35);
    if surface == PART_PLASTER || surface == PART_FRAME || surface == HOUSE_PAINT {
        layer = L_PLASTER;
    } else if surface == HOUSE_PLASTER || surface == HOUSE_REVEAL {
        layer = plaster;
    } else if surface == PART_TILES {
        // the slopes and the ridge's cap; the soffit under the eave is plaster
        layer = select(L_PLASTER, select(L_ROOF, L_ROOF2, fract(seed * 3.71) < 0.4), n.y > 0.35);
    } else if surface == HOUSE_SLATE {
        layer = select(L_PLASTER, L_SLATE, n.y > 0.35);
    } else if surface == PART_STONE {
        layer = L_STONE;
    } else if surface == HOUSE_RUSTIC {
        layer = L_RUSTIC;
    } else if surface == PART_BUILDING {
        layer = select(L_PLASTER, L_ROOF, slope);
    }
    let glowing = max(c.r, max(c.g, c.b)) > 1.0;
    let glass = surface == SURFACE_GLASS || (surface != SURFACE_SHUTTER && luma(c) < 0.075 && layer == L_NONE && surface == 0u);
    if glowing || glass || surface == SURFACE_SHUTTER {
        layer = L_NONE;
    }

    // The face's axes: along it (horizontal) and up it (up the slope, or
    // north on a flat top).
    let t_h = normalize(vec3<f32>(-ng.z, 0.0, ng.x) + vec3<f32>(1e-5, 0.0, 0.0));
    var t = t_h;
    var b = normalize(vec3<f32>(0.0, 1.0, 0.0) - ng * ng.y);
    if ng.y >= 0.93 {
        t = vec3<f32>(1.0, 0.0, 0.0);
        b = vec3<f32>(0.0, 0.0, -1.0);
    }
    // a house's own offset of its photographs, so neighbours do not repeat
    let shift = vec2<f32>(fract(seed * 13.7), fract(seed * 29.3)) * 7.0;
    let size = size_of(layer);
    let uv = vec2<f32>(dot(p, t), -dot(p, b)) / size + shift;
    let duv_x = vec2<f32>(dot(dpx, t), -dot(dpx, b)) / size;
    let duv_y = vec2<f32>(dot(dpy, t), -dot(dpy, b)) / size;
    var tex = vec4<f32>(0.0);
    var tn = vec3<f32>(0.0, 0.0, 1.0);
    let relief = px_m < 2.5;
    if layer >= 0 {
        tex = textureSampleGrad(house_color, house_sampler, uv, layer, duv_x, duv_y);
        if relief {
            tn = textureSampleGrad(house_normal, house_sampler, uv, layer, duv_x, duv_y).xyz * 2.0 - 1.0;
        }
    }

    // Weathering over whole walls, in metres: damp patches, dirt run down
    // from the eaves. Strongest near, a tone far.
    let mid = 1.0 - smoothstep(1.5, 6.0, px_m);
    let along = dot(p.xz, t_h.xz);
    let blot = value_noise(vec2<f32>(along * 0.18, p.y * 0.25) + 3.0 + seed * 40.0) - 0.5;
    let streak = smoothstep(0.55, 0.9, value_noise(vec2<f32>(along * 1.6, p.y * 0.12) + seed * 17.0));
    // ordinary surfaces, not a charred one (dark grey) or a lit one
    let ordinary = smoothstep(0.16, 0.24, luma(c));
    // height above the foot of the house (the ground under it is `foot` +
    // 1.5 m and up)
    let h_foot = p.y - foot - 1.5;
    let wall = abs(n.y) < 0.3;

    var rgb = c;
    var normal_mix = 0.0;
    switch layer {
        case 0, 6: {
            // plaster: the photo's light and shade over the house's colour
            let detail = tex.rgb / mean_of(layer);
            // the photo's contrast raised (lime wash is patchier than a fresh
            // wall), the second, already patchy, photo's lowered
            var d = pow(mix(vec3<f32>(luma(detail)), detail, 0.5), vec3<f32>(1.6));
            if layer == L_PLASTER2 {
                d = mix(vec3<f32>(1.0), vec3<f32>(luma(detail)), 0.45);
            }
            let big = value_noise(vec2<f32>(along, p.y) * 0.09 + 7.0 + seed * 11.0) - 0.5;
            let weather = select(1.0, 0.35 + worn, surface == HOUSE_PLASTER);
            rgb = c * d * (1.0 + big * 0.35 * ordinary + blot * 0.30 * mid * ordinary * weather - streak * 0.12 * mid * ordinary * weather);
            set_pbr(&pbr, clamp(tex.a, 0.6, 1.0), 0.0, 0.3);
            normal_mix = 0.8;
            if surface == HOUSE_PLASTER && wall {
                // Damp rising from the foot: a darker, greyer band with a
                // ragged top, higher on an old house.
                let edge = (value_noise(vec2<f32>(along * 0.45, 0.0) + seed * 9.0) - 0.5) * 1.4;
                let damp_top = 1.0 + 1.6 * worn + edge;
                let damp = (1.0 - smoothstep(damp_top - 0.5, damp_top + 0.3, h_foot)) * step(-2.0, h_foot);
                let dirt = mix(vec3<f32>(luma(rgb)), rgb, 0.55) * vec3<f32>(0.70, 0.66, 0.60);
                rgb = mix(rgb, dirt, damp * 0.75 * ordinary);
                // Flaked plaster, on the worn ones: patches near the foot and
                // in blotches higher up, the brick (or rubble) under them,
                // a pale broken rim round each.
                let fl = value_noise(vec2<f32>(along, p.y) * 0.6 + seed * 23.0) * 0.6
                    + value_noise(vec2<f32>(along, p.y) * 2.1 + seed * 5.0) * 0.4;
                let low = 1.0 - smoothstep(1.0, 6.0, h_foot);
                let thresh = 0.9 - 0.16 * worn * worn - 0.16 * low * worn;
                let flake = smoothstep(thresh, thresh + 0.02 + 0.5 * px_m * 0.02, fl) * ordinary * step(0.45, worn);
                if flake > 0.0 {
                    let bl = select(L_BRICK, L_RUSTIC, fract(seed * 5.3) < 0.4);
                    let bsize = size_of(bl);
                    let buv = vec2<f32>(dot(p, t), -dot(p, b)) / bsize + shift;
                    let btex = textureSampleGrad(house_color, house_sampler, buv, bl, duv_x * size / bsize, duv_y * size / bsize);
                    let under = btex.rgb * 1.05;
                    let rim = smoothstep(thresh - 0.03, thresh, fl) * (1.0 - flake);
                    rgb = mix(rgb, under, flake);
                    rgb = mix(rgb, rgb * 1.12 + vec3<f32>(0.02), rim * 0.6);
                    if relief {
                        let bn = textureSampleGrad(house_normal, house_sampler, buv, bl, duv_x * size / bsize, duv_y * size / bsize).xyz * 2.0 - 1.0;
                        tn = normalize(mix(tn, bn, flake));
                    }
                }
            }
            if surface == HOUSE_REVEAL {
                rgb *= 0.86;
            }
            if surface == HOUSE_PAINT {
                // paint over the plaster: flatter, a little faded in patches
                rgb = c * (0.92 + 0.08 * luma(detail)) * (1.0 + blot * 0.18 * mid);
                normal_mix = 0.35;
            }
            if surface == PART_FRAME {
                rgb = c * (0.9 + 0.1 * luma(detail));
                set_pbr(&pbr, 0.55, 0.0, 0.4);
                normal_mix = 0.2;
            }
        }
        case 1, 7: {
            // clay tiles: the photo's colour, tinted toward the house's roof;
            // a charred roof keeps only the photo's shade
            let m = mean_of(layer);
            let detail = luma(tex.rgb) / luma(m);
            let photo = tex.rgb * (luma(c) / luma(m)) * mix(vec3<f32>(1.0), c / max(luma(c), 1e-3) / (m / luma(m)), 0.4);
            rgb = mix(c * detail, photo, 0.75 * ordinary);
            // lichen and soot in blotches on an old roof
            let lich = smoothstep(0.55, 0.85, value_noise(p.xz * 0.21 + seed * 31.0)) * worn;
            rgb = mix(rgb, rgb * vec3<f32>(0.62, 0.62, 0.55), lich * 0.6 * ordinary);
            rgb *= 1.0 + blot * 0.25 * mid * ordinary;
            set_pbr(&pbr, clamp(tex.a, 0.45, 0.95), 0.0, 0.4);
            normal_mix = 1.0;
        }
        case 3: {
            // slate: the photo's grain over the vertex's grey
            let detail = tex.rgb / MEAN_SLATE;
            rgb = c * mix(vec3<f32>(luma(detail)), detail, 0.3);
            rgb *= 1.0 + blot * 0.2 * mid * ordinary;
            set_pbr(&pbr, clamp(tex.a, 0.4, 0.85), 0.0, 0.45);
            normal_mix = 1.0;
        }
        case 2, 4: {
            // stone: the photo's colour scaled to the vertex's brightness
            let m = mean_of(layer);
            let detail = luma(tex.rgb) / luma(m);
            let photo = tex.rgb * (luma(c) / luma(m)) * mix(vec3<f32>(1.0), c / max(luma(c), 1e-3) / (m / luma(m)), 0.5);
            rgb = mix(c * detail, photo, 0.7 * ordinary);
            if surface == HOUSE_RUSTIC && wall {
                let damp = 1.0 - smoothstep(0.5, 2.5 + blot * 2.0, h_foot);
                rgb = mix(rgb, rgb * 0.7, damp * 0.6 * ordinary);
            }
            set_pbr(&pbr, clamp(0.55 + tex.a * 0.4, 0.6, 0.95), 0.0, 0.3);
            normal_mix = 1.0;
        }
        default: {
            if glowing {
                pbr.material.emissive = vec4<f32>(c * 0.8, 1.0);
            } else if glass {
                // dark glass with a little sky in it, and behind it the room,
                // curtained or not, a tone of its own per pane
                set_pbr(&pbr, 0.08, 0.0, 0.5);
                let room = hash21(floor(vec2<f32>(along * 0.4, p.y * 0.3)) + seed * 3.0);
                rgb = c * select(1.0, 0.45, luma(c) > 0.085) * (0.8 + 0.5 * room);
            } else if surface == SURFACE_SHUTTER {
                // painted louvres, the paint worn and faded
                let f = fract(p.y / 0.13);
                let slat = smoothstep(0.0, 0.3, f) * (1.0 - 0.6 * smoothstep(0.72, 1.0, f));
                let near = 1.0 - smoothstep(0.04, 0.2, px_m);
                let wear = value_noise(p.xz * 7.0 + p.y * 5.0) - 0.5;
                let fade = 1.0 + (value_noise(vec2<f32>(along, p.y) * 0.6 + seed * 7.0) - 0.5) * 0.3 * worn;
                rgb = c * mix(0.85, 0.5 + 0.7 * slat, near) * (1.0 + wear * 0.25 * near) * fade;
                set_pbr(&pbr, 0.6, 0.0, 0.4);
            } else if surface == SURFACE_PAINT {
                // a car's lacquer under its clear coat
                set_pbr(&pbr, 0.3, 0.0, 0.5);
                pbr.material.clearcoat = 1.0;
                pbr.material.clearcoat_perceptual_roughness = 0.08;
            } else if surface == SURFACE_METAL {
                // wrought iron (railings, grilles) or a lamp's metal
                set_pbr(&pbr, 0.45 + 0.15 * value_noise(p.xz * 9.0 + p.y * 14.0), 0.6, 0.5);
            } else if surface == SURFACE_RUBBER {
                set_pbr(&pbr, 0.9, 0.0, 0.3);
            } else if surface == SURFACE_CLOTH {
                let near = 1.0 - smoothstep(0.05, 0.4, px_m);
                rgb = c * (1.0 + (value_noise(p.xz * 60.0 + p.y * 45.0) - 0.5) * 0.12 * near);
                set_pbr(&pbr, 0.93, 0.0, 0.25);
            } else if surface == SURFACE_WOOD {
                // weathered planks: a long grain, boards of their own tone
                let near = 1.0 - smoothstep(0.1, 0.8, px_m);
                let grain = value_noise(vec2<f32>(along * 3.0, p.y * 40.0)) - 0.5;
                let board = hash21(vec2<f32>(floor(p.y / 0.18), floor(along / 2.0))) - 0.5;
                rgb = c * (1.0 + (grain * 0.3 + board * 0.25) * near);
                set_pbr(&pbr, 0.8, 0.0, 0.3);
            } else {
                rgb = prop_albedo(c, p, n, px_m);
                set_pbr(&pbr, 0.8, 0.0, 0.3);
            }
        }
    }
    if layer >= 0 {
        // a charred house (`buildings::recolor_structure`): as black as the
        // flat one, the photograph's relief still in it
        rgb *= mix(0.6, 1.0, ordinary);
        // the photograph's relief, on the face's axes; faded where a texel
        // is smaller than a pixel would make it glitter
        let k = normal_mix * (1.0 - smoothstep(0.3, 2.5, px_m));
        let bent = normalize(t * tn.x + b * tn.y + n * max(tn.z, 0.2));
        pbr.N = normalize(mix(n, bent, k));
    }
    pbr.material.base_color = vec4<f32>(rgb, pbr.material.base_color.a);

    var out: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting_faded(pbr);
    } else {
        out.color = pbr.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr, out.color);
    return out;
}
