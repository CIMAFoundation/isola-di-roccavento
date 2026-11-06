// Photographed masonry (`materials/masonry.rs`): the buildings and the
// landmarks when the plants are scans (`KIOSK_SCANS=1`, 2026-11-06). Lime
// plaster, clay roof tiles and rubble stone from Poly Haven (CC0), mapped in
// world metres on the faces' own axes: along the wall and up it, along the
// eave and up the slope. The vertex colour stays the tint, so the fire's
// recolouring (`buildings::damage`) reads through: a charred house is its
// own plaster and tiles gone black, a burning one glows.
//
// What a vertex is comes from the vertex colour's alpha (`surface_alpha`):
// the structures' ids 1-11 (4 glass, 11 shutters) and `masonry::Part` 12-16.
// Everything is sampled with explicit gradients from the world position's
// derivatives, taken once at the top: no derivative after a branch (the
// browser's WGSL refuses them) and no seam where a face's uv jumps.

import bevy_pbr::render::pbr_functions::{alpha_discard, main_pass_post_lighting_processing};
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
import bevy_pbr::render::pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT};
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import game::shaders::noise::{hash21, value_noise};
import game::shaders::faded_shadow::apply_pbr_lighting_faded;

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var masonry_color: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var masonry_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var masonry_normal: texture_2d_array<f32>;

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
const PART_ROCK: u32 = 17u;

// texture layers
const L_NONE: i32 = -1;
const L_PLASTER: i32 = 0;
const L_ROOF: i32 = 1;
const L_STONE: i32 = 2;
const L_ROCK: i32 = 3;

// Mean linear colour of each photograph (`scripts/pack_building_textures.py`
// prints them): the detail is the photo over its mean, so a wall's average is
// its vertex colour.
const MEAN_PLASTER: vec3<f32> = vec3<f32>(0.3843, 0.3455, 0.3313);
const MEAN_ROOF: vec3<f32> = vec3<f32>(0.2391, 0.064, 0.0174);
const MEAN_STONE: vec3<f32> = vec3<f32>(0.3763, 0.2932, 0.2072);
const MEAN_ROCK: vec3<f32> = vec3<f32>(0.2918, 0.2637, 0.2283);

// Metres of world one repeat of each photograph covers. The photos are 2 m
// (plaster), 2.5 m (tiles, stone) of the real thing; the houses are drawn
// larger than life (`visual_scale`), and the tiles and stones with them.
const SIZE_PLASTER: f32 = 3.0;
const SIZE_ROOF: f32 = 4.6;
const SIZE_STONE: f32 = 4.0;
// the landmarks' masonry (3 m of sandstone blocks in the photo) is drawn at
// their diorama scale
const SIZE_STONE_LANDMARK: f32 = 5.5;
// the limestone at the size the terrain's cliffs draw it (`terrain.wesl`,
// 20 m), so a boulder and the cliff behind it read as the same rock
const SIZE_ROCK: f32 = 20.0;

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

// Value noise in 3-D (on a rounded rock a 2-D one read along a slant draws
// stripes).
fn value_noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let k = vec2<f32>(57.0, 113.0);
    let z0 = vec2<f32>(i.x, i.y) + k * i.z;
    let z1 = z0 + k;
    let a = mix(mix(hash21(z0), hash21(z0 + vec2<f32>(1.0, 0.0)), u.x), mix(hash21(z0 + vec2<f32>(0.0, 1.0)), hash21(z0 + vec2<f32>(1.0, 1.0)), u.x), u.y);
    let b = mix(mix(hash21(z1), hash21(z1 + vec2<f32>(1.0, 0.0)), u.x), mix(hash21(z1 + vec2<f32>(0.0, 1.0)), hash21(z1 + vec2<f32>(1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

// What grows on old stone left alone: crusts of lichen (grey-green and
// ochre, in patches) and moss (dark olive, on what faces up), strongest
// near, a faint tone far.
fn overgrown(rgb: vec3<f32>, p: vec3<f32>, n: vec3<f32>, px_m: f32, amount: f32) -> vec3<f32> {
    let near = 1.0 - smoothstep(0.5, 2.5, px_m);
    let blotch = value_noise3(p * 0.19 + 31.0);
    let fine = mix(0.55, value_noise3(p * 2.7), near);
    let crust = smoothstep(0.6, 0.8, blotch) * smoothstep(0.35, 0.7, fine) * amount;
    let ochre = value_noise3(p * 0.7 + 5.0);
    let lichen = mix(vec3<f32>(0.24, 0.25, 0.19), vec3<f32>(0.40, 0.28, 0.10), smoothstep(0.6, 0.85, ochre));
    var out = mix(rgb, lichen * (0.7 + 0.6 * fine), crust * 0.45);
    let up = smoothstep(0.5, 0.9, n.y);
    let moss = up * smoothstep(0.5, 0.72, value_noise3(p * 0.33 + 17.0)) * smoothstep(0.3, 0.6, fine) * amount;
    out = mix(out, vec3<f32>(0.06, 0.07, 0.03) * (0.7 + 0.6 * fine), moss * 0.55);
    return out;
}

// A world position (or its derivative) projected on the plane across `axis`
// (0 x, 1 y, 2 z), as the terrain's cliffs lay their rock.
fn rock_uv(v: vec3<f32>, axis: u32) -> vec2<f32> {
    if axis == 0u {
        return vec2<f32>(v.z, -v.y);
    }
    if axis == 1u {
        return v.xz;
    }
    return vec2<f32>(v.x, -v.y);
}

// The photograph's tangent-space normal `m` on that plane, as a bend of the
// world normal `n`.
fn rock_bend(m: vec3<f32>, axis: u32, n: vec3<f32>) -> vec3<f32> {
    if axis == 0u {
        return vec3<f32>(0.0, m.y, m.x * sign(n.x));
    }
    if axis == 1u {
        return vec3<f32>(m.x, 0.0, -m.y);
    }
    return vec3<f32>(m.x * sign(n.z), m.y, 0.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    @if(VERTEX_COLORS)
    let surface = surface_id(in.color.a);
    @else
    let surface = 0u;
    // the terrain's height under a crag's vertex (`landmarks::ground_under`)
    var ground = -1.0e4;
    @if(VERTEX_UVS_A) {
        ground = in.uv.x;
    }
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    let p = in.world_position.xyz;
    let c = pbr.material.base_color.rgb;
    // the face's normal as lit (turned toward the eye on a two-sided landmark)
    let n = normalize(pbr.N);
    // derivatives first, before anything branches
    let dpx = dpdx(p);
    let dpy = dpdy(p);
    let px_m = max(length(dpx), length(dpy));
    // The photographs are laid on the face's axes, read from its normal: the
    // vertex normal, flat on every building (only the rocks, which are drawn
    // triplanar on the world's axes, are smoothed). Not the face from the
    // derivatives of the position: kilometres from the origin they are a
    // percent off, and `dot(p, t)` with an axis a percent off moves the
    // texture tens of metres between neighbouring pixels.
    let ng = n;

    // What the pixel is, from its part and its face.
    let slope = n.y > 0.35 && n.y < 0.93;
    let flat_top = n.y >= 0.93;
    var layer = L_NONE;
    if surface == PART_PLASTER || surface == PART_FRAME {
        layer = L_PLASTER;
    } else if surface == PART_TILES {
        // the slopes and the ridge's cap; the soffit under the eave is plaster
        layer = select(L_PLASTER, L_ROOF, n.y > 0.35);
    } else if surface == PART_STONE {
        layer = L_STONE;
    } else if surface == PART_BUILDING {
        layer = select(L_PLASTER, L_ROOF, slope);
    } else if surface == PART_ROCK {
        layer = L_ROCK;
    }
    @if(MASONRY_ROCK) {
        if surface == 0u {
            layer = L_ROCK;
        }
    }
    @if(MASONRY_LANDMARK) {
        // the landmarks' bake: stone, but not the ivy (green) nor the dark
        // openings
        if surface == 0u && luma(c) > 0.035 && !(c.g > c.r * 1.05) {
            layer = L_STONE;
        }
    }
    let glowing = max(c.r, max(c.g, c.b)) > 1.0;
    let glass = surface == SURFACE_GLASS || (surface != SURFACE_SHUTTER && luma(c) < 0.075 && layer == L_NONE);
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
    var size = SIZE_PLASTER;
    if layer == L_ROOF {
        size = SIZE_ROOF;
    } else if layer == L_STONE {
        size = SIZE_STONE;
        @if(MASONRY_LANDMARK) {
            size = SIZE_STONE_LANDMARK;
        }
    }
    let uv = vec2<f32>(dot(p, t), -dot(p, b)) / size;
    let duv_x = vec2<f32>(dot(dpx, t), -dot(dpx, b)) / size;
    let duv_y = vec2<f32>(dot(dpy, t), -dot(dpy, b)) / size;
    // Sampled only where they are drawn (explicit gradients: allowed in a
    // branch), and the relief only where it is still bigger than a pixel.
    var tex = vec4<f32>(0.0);
    var tn = vec3<f32>(0.0, 0.0, 1.0);
    var rock_bent = n;
    if layer == L_ROCK {
        // Rock: on the world's axes, as the terrain's cliffs, so a rounded
        // boulder is never stretched and never seams. Biplanar (the two
        // axes the normal leans on most, never the third), and the second
        // only where it weighs: one or two reads of colour and of relief
        // instead of up to three of each (2026-11-06, ~1 ms at the rocca).
        let a = abs(n);
        let major = select(select(2u, 1u, a.y > a.z), 0u, a.x > a.y && a.x > a.z);
        let minor = select(select(2u, 1u, a.y < a.z), 0u, a.x < a.y && a.x < a.z);
        let median = 3u - major - minor;
        // past 1/sqrt(3) a normal leans on an axis; sharpened so the seam is short
        var w = clamp((vec2<f32>(a[major], a[median]) - 0.5773) / (1.0 - 0.5773), vec2<f32>(0.0), vec2<f32>(1.0));
        w = w * w;
        w = w / (w.x + w.y);
        let k = 1.0 / SIZE_ROCK;
        let relief = px_m < 2.5;
        let q0 = rock_uv(p, major) * k;
        let gx0 = rock_uv(dpx, major) * k;
        let gy0 = rock_uv(dpy, major) * k;
        tex = textureSampleGrad(masonry_color, masonry_sampler, q0, L_ROCK, gx0, gy0) * w.x;
        var bend = vec3<f32>(0.0);
        if relief {
            let m = textureSampleGrad(masonry_normal, masonry_sampler, q0, L_ROCK, gx0, gy0).xyz * 2.0 - 1.0;
            bend = rock_bend(m, major, n) * w.x;
        }
        if w.y > 0.02 {
            let q1 = rock_uv(p, median) * k;
            let gx1 = rock_uv(dpx, median) * k;
            let gy1 = rock_uv(dpy, median) * k;
            tex += textureSampleGrad(masonry_color, masonry_sampler, q1, L_ROCK, gx1, gy1) * w.y;
            if relief {
                let m = textureSampleGrad(masonry_normal, masonry_sampler, q1, L_ROCK, gx1, gy1).xyz * 2.0 - 1.0;
                bend += rock_bend(m, median, n) * w.y;
            }
        } else {
            tex = tex / w.x;
            bend = bend / w.x;
        }
        rock_bent = normalize(n + bend);
    } else if layer >= 0 {
        tex = textureSampleGrad(masonry_color, masonry_sampler, uv, layer, duv_x, duv_y);
        if px_m < 2.5 {
            tn = textureSampleGrad(masonry_normal, masonry_sampler, uv, layer, duv_x, duv_y).xyz * 2.0 - 1.0;
        }
    }

    // Weathering over whole walls, in metres: damp patches, dirt run down
    // from the eaves, a darker foot. Strongest near, gone far.
    let mid = 1.0 - smoothstep(1.5, 6.0, px_m);
    let blot = value_noise(vec2<f32>(dot(p.xz, t_h.xz) * 0.18, p.y * 0.25) + 3.0) - 0.5;
    let streak = smoothstep(0.6, 0.9, value_noise(vec2<f32>(dot(p.xz, t_h.xz) * 1.6, p.y * 0.12)));
    // ordinary surfaces, not a charred one (dark grey) or a lit one
    let ordinary = smoothstep(0.16, 0.24, luma(c));

    var rgb = c;
    var normal_mix = 0.0;
    switch layer {
        case 0: {
            // plaster: the photo's light and shade over the house's colour,
            // with a little of the photo's own wear
            // the photo's contrast raised: lime wash is patchier than this
            // freshly painted wall
            let detail = tex.rgb / MEAN_PLASTER;
            let d = pow(mix(vec3<f32>(luma(detail)), detail, 0.5), vec3<f32>(1.6));
            // larger, slower patches break the photo's repeat
            let big = value_noise(vec2<f32>(dot(p.xz, t_h.xz), p.y) * 0.09 + 7.0) - 0.5;
            rgb = c * d * (1.0 + big * 0.35 * ordinary + blot * 0.30 * mid * ordinary - streak * 0.09 * mid * ordinary);
            set_pbr(&pbr, clamp(tex.a, 0.6, 1.0), 0.0, 0.3);
            normal_mix = 0.8;
            if surface == PART_FRAME {
                rgb = c * (0.9 + 0.1 * luma(detail));
                set_pbr(&pbr, 0.55, 0.0, 0.4);
                normal_mix = 0.2;
            }
        }
        case 1: {
            // tiles: the photo's colour, tinted toward the house's roof;
            // a charred roof keeps only the photo's shade
            let detail = luma(tex.rgb) / luma(MEAN_ROOF);
            let photo = tex.rgb * (luma(c) / luma(MEAN_ROOF)) * mix(vec3<f32>(1.0), c / max(luma(c), 1e-3) / (MEAN_ROOF / luma(MEAN_ROOF)), 0.35);
            rgb = mix(c * detail, photo, 0.75 * ordinary);
            rgb *= 1.0 + blot * 0.25 * mid * ordinary;
            set_pbr(&pbr, clamp(tex.a, 0.45, 0.95), 0.0, 0.4);
            normal_mix = 1.0;
        }
        case 2: {
            // stone: the photo's colour scaled to the vertex's brightness
            let detail = luma(tex.rgb) / luma(MEAN_STONE);
            let photo = tex.rgb * (luma(c) / luma(MEAN_STONE)) * mix(vec3<f32>(1.0), c / max(luma(c), 1e-3) / (MEAN_STONE / luma(MEAN_STONE)), 0.5);
            rgb = mix(c * detail, photo, 0.7 * ordinary);
            @if(MASONRY_LANDMARK) {
                rgb = overgrown(rgb, p, n, px_m, 1.0);
            }
            set_pbr(&pbr, clamp(0.55 + tex.a * 0.4, 0.6, 0.95), 0.0, 0.3);
            normal_mix = 1.0;
        }
        case 3: {
            // bare limestone: the photograph's own colour, the terrain's
            // cliffs' (mean ~0.29), shaded by the bake (the wet foot of a
            // coast boulder, the crag's darker clefts) around its usual value
            // (the crag's bake is shaded face by face: only a little of it,
            // or its facets show through)
            var dry = 0.54;
            var follow = 0.5;
            @if(MASONRY_CRAG) {
                dry = 0.20;
                follow = 0.15;
            }
            if surface == PART_ROCK {
                // the tower's boulders, shaded face by face in their bake too
                dry = 0.32;
                follow = 0.15;
            }
            let shade = clamp(luma(c) / dry, 0.3, 1.3);
            // a little darker and warmer than the bare photograph: the tone of
            // the terrain's cliffs (`terrain.wesl`, limestone x photo)
            rgb = tex.rgb * vec3<f32>(0.82, 0.79, 0.74) * mix(1.0, shade, follow);
            @if(!MASONRY_CRAG) {
                // the coast's boulders: their bake is the cliffs' limestone
                // already (pale, ochre beds, wet at the foot), and keeps it;
                // the photograph brings the grain and the cracks
                if surface == 0u {
                    let detail = tex.rgb / MEAN_ROCK;
                    // a reef's salt-white crown, under the photograph's
                    // grain, read as snow: kept to a pale limestone
                    let cc = c * min(1.0, 0.42 / max(luma(c), 1e-3));
                    rgb = cc * mix(vec3<f32>(luma(detail)), detail, 0.5);
                }
            }
            @if(MASONRY_CRAG) {
                rgb = overgrown(rgb, p, n, px_m, 0.8);
                // scree and dry soil gathered at the foot, up to ~15 m
                // over the terrain (its height under the vertex is in the
                // first uv, `landmarks::ground_under`), more on what faces up
                // and in tongues, so the rock does not stop on the ground in
                // a clean line
                let tongue = value_noise3(p * 0.11 + 13.0) - 0.5;
                let grit = value_noise3(p * 1.3 + 3.0);
                let soil = (1.0 - smoothstep(2.0, 16.0, p.y - ground + tongue * 14.0)) * mix(0.5, 1.0, smoothstep(0.2, 0.8, n.y));
                let earth = vec3<f32>(0.27, 0.22, 0.15) * (0.75 + 0.5 * grit) * mix(vec3<f32>(1.0), tex.rgb / MEAN_ROCK, 0.5);
                rgb = mix(rgb, earth, soil * 0.85);
            }
            if surface == PART_ROCK {
                rgb = overgrown(rgb, p, n, px_m, 0.6);
            }
            // dark and wet where the spray reaches, as on the cliffs
            let edge_n = value_noise(p.xz * 0.35) - 0.5;
            let spray = 1.0 - smoothstep(0.6, 2.8, p.y + edge_n * 1.6);
            rgb = mix(rgb, rgb * vec3<f32>(0.46, 0.47, 0.50), spray * 0.8);
            set_pbr(&pbr, mix(clamp(tex.a, 0.6, 0.95), 0.35, spray), 0.0, 0.3);
            normal_mix = 1.0;
        }
        default: {
            if glowing {
                pbr.material.emissive = vec4<f32>(c * 0.8, 1.0);
            } else if glass {
                // dark glass with a little sky in it; a charred house's
                // (`recolor_structure` lifts it to ~0.1) as dark as its walls
                set_pbr(&pbr, 0.05, 0.0, 0.7);
                rgb = c * select(1.0, 0.45, luma(c) > 0.085);
            } else if surface == SURFACE_SHUTTER {
                // painted louvres, the paint a little worn
                let f = fract(p.y / 0.13);
                let slat = smoothstep(0.0, 0.3, f) * (1.0 - 0.6 * smoothstep(0.72, 1.0, f));
                let near = 1.0 - smoothstep(0.04, 0.2, px_m);
                let wear = value_noise(p.xz * 7.0 + p.y * 5.0) - 0.5;
                rgb = c * mix(0.85, 0.5 + 0.7 * slat, near) * (1.0 + wear * 0.25 * near);
                set_pbr(&pbr, 0.6, 0.0, 0.4);
            } else if surface == SURFACE_PAINT {
                // a car's lacquer under its clear coat
                set_pbr(&pbr, 0.3, 0.0, 0.5);
                pbr.material.clearcoat = 1.0;
                pbr.material.clearcoat_perceptual_roughness = 0.08;
            } else if surface == SURFACE_METAL {
                set_pbr(&pbr, 0.3 + 0.15 * value_noise(p.xz * 9.0 + p.y * 14.0), 1.0, 0.5);
            } else if surface == SURFACE_RUBBER {
                set_pbr(&pbr, 0.9, 0.0, 0.3);
            } else if surface == SURFACE_CLOTH {
                let near = 1.0 - smoothstep(0.05, 0.4, px_m);
                rgb = c * (1.0 + (value_noise(p.xz * 60.0 + p.y * 45.0) - 0.5) * 0.12 * near);
                set_pbr(&pbr, 0.93, 0.0, 0.25);
            } else if surface == SURFACE_WOOD {
                // weathered planks: a long grain, boards of their own tone
                let near = 1.0 - smoothstep(0.1, 0.8, px_m);
                let along = dot(p.xz, t_h.xz);
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
        if layer != L_ROCK {
            rgb *= mix(0.6, 1.0, ordinary);
        }
        // the photograph's relief, on the face's axes; faded where a texel
        // is smaller than a pixel would make it glitter
        let k = normal_mix * (1.0 - smoothstep(0.3, 2.5, px_m));
        var bent = normalize(t * tn.x + b * tn.y + n * max(tn.z, 0.2));
        if layer == L_ROCK {
            bent = rock_bent;
        }
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
