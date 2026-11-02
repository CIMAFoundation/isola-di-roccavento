// The sea.
//
// The vertex shader displaces a flat lattice with a small stack of sine
// waves travelling in the wind's downwind direction, amplitude and speed
// scaled by wind speed (see `crate::sea`). The fragment shader never derives
// the wave slope by hand: `dpdx`/`dpdy` of the already-displaced world
// position gives the facet normal for free, which is what makes the big
// rollers shade correctly without a closed-form derivative of the sum of
// sines. A second, finer normal wobble from scrolling noise adds wind chop
// at a scale too small to be worth displacing geometry for.

import bevy_pbr::render::forward_io::{Vertex, VertexOutput};
import bevy_pbr::render::mesh_functions;
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::{view, globals, lights};
import game::shaders::scene_light::apply_scene_fog;
import game::shaders::noise::{hash21, value_noise};

struct WaterUniform {
    // xy: downwind unit direction in world XZ (bevy x, z). z: wind speed, m/s.
    wind: vec4<f32>,
    // xyz: unit direction toward the sun. w: day gate, 0 at night to 1 at noon.
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    deep_color: vec4<f32>,
    shallow_color: vec4<f32>,
    // xy: world metres to ground-mask coordinates.
    mask_rect: vec4<f32>,
    // x: tactical desaturation, 0 (full colour) to 1.
    tactical: vec4<f32>,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> water: WaterUniform;
// The ground mask; its alpha is the signed distance to the waterline, 1 m a
// step around 127.5 (above: in the water). See `ground_mask.rs`.
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var shore_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var shore_samp: sampler;

// A wave the lattice cannot sample (under ~2 points per wavelength) only
// aliases into a checkerboard, so each component fades out as the local
// lattice spacing (`stride`, carried in the vertex colour's green) approaches
// its wavelength: the near sea keeps the long swell and a shorter wave, the
// open sea's coarse lattice stays flat and leaves the chop to the fragment
// shader.
fn wave_height(p: vec2<f32>, t: f32, stride: f32, shore: f32) -> f32 {
    let dir0 = water.wind.xy;
    let perp = vec2<f32>(-dir0.y, dir0.x);
    let dir1 = normalize(dir0 * 0.6 + perp * 0.5);
    let speed = water.wind.z;
    // A dead calm sea is glassy: amplitude collapses toward zero with speed,
    // same as the ember/spotting model treats zero wind as no transport.
    // Capped at 0.5: `WATER_BASE_Y` (`crate::sea`) is only sized to clear a
    // trough this deep, and a bigger cap here needs a bigger base height to
    // match or the terrain pokes through at the bottom of the swell.
    let amp = clamp(speed * 0.045, 0.03, 0.5);
    let s = 0.7 + speed * 0.05;
    let f1 = smoothstep(2.0 * stride, 4.0 * stride, 114.0);
    let f2 = smoothstep(2.0 * stride, 4.0 * stride, 57.0);
    let f3 = smoothstep(2.0 * stride, 4.0 * stride, 30.0);
    var h = 0.0;
    h += f1 * amp * sin(dot(p, dir0) * 0.055 + t * s);
    h += f2 * amp * 0.5 * sin(dot(p, dir1) * 0.11 + t * s * 1.5 + 1.7);
    h += f3 * amp * 0.25 * sin(dot(p, dir0) * 0.21 - t * s * 2.1 + 4.1);
    // Shoaling: the swell loses height as it nears the land (never deeper
    // than the troughs the base height was sized for), and the water only
    // ever *rises* against the shore in a slow surge, so the ground never
    // pokes through.
    h *= mix(0.45, 1.0, smoothstep(0.0, 50.0, shore));
    let surge = 0.5 + 0.5 * sin(t * s * 0.9 + dot(p, dir0) * 0.03);
    h += surge * 0.16 * (0.4 + 0.6 * amp * 2.0) * exp(-shore / 14.0);
    return h;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    world_position.y += wave_height(world_position.xz, globals.time, vertex.color.g, vertex.color.r);

    out.position = position_world_to_clip(world_position.xyz);
    out.world_position = world_position;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.uv = vertex.uv;
    out.color = vertex.color;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = globals.time;
    // The normal comes from the waves' own slope (a finite difference of the
    // same height function the vertex stage displaces by), not from the
    // lattice's facets: facet normals are constant per 15 m triangle, and the
    // sun's glint on them reads as a grid of tiles.
    let stride = in.color.g;
    let e = 3.0;
    let q = in.world_position.xz;
    let hx = wave_height(q + vec2<f32>(e, 0.0), t, stride, in.color.r) - wave_height(q - vec2<f32>(e, 0.0), t, stride, in.color.r);
    let hz = wave_height(q + vec2<f32>(0.0, e), t, stride, in.color.r) - wave_height(q - vec2<f32>(0.0, e), t, stride, in.color.r);
    var normal = normalize(vec3<f32>(-hx / (2.0 * e), 1.0, -hz / (2.0 * e)));

    let speed = water.wind.z;
    let chop_dir = water.wind.xy;
    let chop = clamp(speed * 0.05, 0.0, 1.0);
    let uv = in.world_position.xz * 0.06 + chop_dir * t * (0.15 + speed * 0.03);
    let n1 = value_noise(uv * 3.0) - 0.5;
    let n2 = value_noise(uv * 7.0 + vec2<f32>(11.0, 5.0)) - 0.5;
    normal = normalize(normal + vec3<f32>(n1 * 0.35 * chop, 0.0, n2 * 0.35 * chop));

    let to_camera = view.world_position - in.world_position.xyz;
    let view_distance = length(to_camera);
    let view_dir = to_camera / max(view_distance, 0.001);
    // Two-sided water: use the same angle response when swimming below it.
    let facing = clamp(abs(dot(normal, view_dir)), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - facing, 5.0);

    // `in.color.r` carries the distance to the shore in metres from `crate::sea` (0 at the
    // shoreline), `.g` the lattice spacing, sampled from the same terrain heightfield the ground
    // colour under the water uses.
    // Inside the mask the distance is read per pixel from its texture: the
    // lattice's own value is one probe per 15 m vertex and bands visibly.
    let mask_uv = vec2<f32>(in.world_position.x, -in.world_position.z) * water.mask_rect.xy;
    let mask_a = textureSample(shore_tex, shore_samp, mask_uv).a;
    let inside = all(mask_uv > vec2<f32>(0.0)) && all(mask_uv < vec2<f32>(1.0));
    let shore_d = select(in.color.r, max(mask_a * 255.0 - 127.5, 0.0), inside);
    // The lattice reaches over low ground that the mask calls land (a quay, a
    // hollow below sea level): no water there.
    if inside && mask_a * 255.0 - 127.5 < -0.5 {
        discard;
    }
    let depth = clamp(shore_d / 70.0, 0.0, 1.0);
    // Shallows hold their turquoise a while, then drop to deep blue.
    var col = mix(water.shallow_color.rgb, water.deep_color.rgb, smoothstep(0.0, 1.0, pow(depth, 0.8)));

    // This shader does not run through the scene's own lighting, so it has
    // to dim itself: without this the sea stays lit like noon after the sun
    // sets, which is exactly the always-bright-water bug this comment is
    // here to stop someone reintroducing. `sun_color.a` carries the sun's
    // own illuminance-normalised brightness (`SunState::brightness`), not
    // the coarser `sin(elevation)` gate used for the specular term below —
    // that gate alone stayed under 0.5 at a properly sunny mid-afternoon
    // sun and made the sea visibly dimmer than a shoreline lit by the same
    // light.
    let light = mix(0.12, 1.0, water.sun_color.a);
    col *= light;

    let sky_reflection = (water.deep_color.rgb * 0.4 + vec3<f32>(0.55, 0.62, 0.68)) * light;
    col = mix(col, sky_reflection, fresnel * 0.6);

    // the sun light's direction, this frame: the material keeps only what
    // changes slowly (`crate::sea::update_water`)
    var sun_dir = normalize(water.sun_dir.xyz);
    if (lights.n_directional_lights > 0u) {
        sun_dir = normalize(lights.directional_lights[0].direction_to_light);
    }
    let half_dir = normalize(sun_dir + view_dir);
    // A calm sea gives a tight glitter; wind-chopped water spreads the same
    // light over a wider, softer streak.
    let spec_power = mix(700.0, 60.0, chop);
    let spec = pow(clamp(dot(normal, half_dir), 0.0, 1.0), spec_power) * water.sun_dir.w;
    col += water.sun_color.rgb * spec * 2.5;

    // Surf. Lacy foam where the water meets the land, and wash lines that
    // run in toward the shore and thin out as they go: bands of the distance
    // field, eroded by two octaves of noise so they break into curls.
    let wp2 = in.world_position.xz;
    let lace = value_noise(wp2 * 0.55 + t * 0.2) * 0.6 + value_noise(wp2 * 1.7 - t * 0.35) * 0.4;
    let foam_w = 2.5 + chop * 4.0 + 1.5 * sin(t * 0.9 + shore_d * 0.25 + wp2.x * 0.02);
    let foam_edge = 1.0 - smoothstep(0.0, foam_w, shore_d);
    let shore_foam = smoothstep(0.42, 0.62, lace + foam_edge * 0.55) * foam_edge;
    let wash_phase = shore_d * 0.16 - t * (0.35 + chop * 0.25);
    let wash_line = smoothstep(0.55, 0.95, 0.5 + 0.5 * sin(wash_phase * 6.2832));
    let wash_zone = (1.0 - smoothstep(4.0, 30.0 + chop * 20.0, shore_d)) * smoothstep(1.5, 5.0, shore_d);
    let wash = wash_line * wash_zone * smoothstep(0.50, 0.75, lace) * 0.75;
    let foam = clamp(shore_foam + wash, 0.0, 1.0);
    col = mix(col, vec3<f32>(0.92, 0.95, 0.96) * light, foam * 0.88);
    // Sunlit caustics on the shallow bed: a bright network that drifts slowly.
    let cau_a = value_noise(wp2 * 0.9 + vec2<f32>(t * 0.25, t * 0.18));
    let cau_b = value_noise(wp2 * 0.9 * 1.37 - vec2<f32>(t * 0.2, -t * 0.27) + 9.0);
    let cau = pow(1.0 - abs(cau_a - cau_b) * 2.0, 6.0);
    col += vec3<f32>(0.20, 0.34, 0.30) * cau * (1.0 - smoothstep(4.0, 22.0, shore_d)) * smoothstep(0.5, 3.0, shore_d) * light * water.sun_dir.w;

    // Reveal the shoreline floor and submerged objects up close, especially
    // looking down through calm water. Long views and grazing angles retain
    // an opaque horizon; shore distance is a clarity proxy, not measured depth.
    let near_view = 1.0 - smoothstep(35.0, 260.0, view_distance);
    let clear_angle = smoothstep(0.15, 0.75, facing);
    let clarity = 1.0 - smoothstep(10.0, 90.0, shore_d);
    let transmission = near_view * clear_angle * (1.0 - fresnel)
        * mix(0.16, 0.70, clarity);
    // Foam and sun glitter cover the transmitted view rather than fading out.
    let coverage = clamp(max(foam * 0.85, spec * 1.5), 0.0, 1.0);
    let opacity = mix(1.0 - transmission, 1.0, coverage);
    // Tactical view: grey sea, after the fog (its blue is most of what the
    // far sea shows).
    let fogged = apply_scene_fog(vec4<f32>(col, opacity), in.world_position.xyz);
    let sea_luma = dot(fogged.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(max(mix(fogged.rgb, vec3<f32>(sea_luma) * 0.9, water.tactical.x), vec3<f32>(0.0)), fogged.a);
}
