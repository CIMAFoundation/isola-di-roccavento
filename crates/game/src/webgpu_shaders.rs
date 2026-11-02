//! WebGPU only: Bevy 0.20's PBR shaders as the browser's compiler accepts them.
//!
//! `pbr_fragment`, `pbr_prepass` and `pbr_prepass_functions` pick a texture
//! and its sampler into a `let` (`let texture = pbr_bindings::base_color_texture;`),
//! the bindless one or the bound one. naga lets it through, so it works on
//! the desktop; WGSL forbids a texture or a sampler in a `let`, and the
//! browser rejects the module (2026-11-02, Chrome: «texture_2d<f32> cannot be
//! used as the type of a 'let'»), which stops the game. Still so on Bevy's
//! main. Here each pair of `let` is dropped and its uses get the bound
//! texture's name (WebGPU in the browser has no bindless textures).
//!
//! And a way back: a rendering error the game did not foresee (another
//! browser's compiler, a lost device) stops Bevy's renderer; here the page is
//! asked to load again on WebGL2 (`roccaWebGpuFailed`, `web/index.html`).

use bevy::prelude::*;
use bevy::render::error_handler::{RenderErrorHandler, RenderErrorPolicy};
use bevy::shader::Source;

pub struct WebGpuShadersPlugin;

impl Plugin for WebGpuShadersPlugin {
    fn build(&self, app: &mut App) {
        // after `AssetEventSystems` (PostUpdate), before the extraction: the
        // render world compiles the shader as rewritten
        app.add_systems(Last, inline_handle_lets);
        app.insert_resource(RenderErrorHandler(|error, _, _| {
            error!("WebGPU: {:?} {}; back to WebGL2", error.ty, error.description);
            #[cfg(target_arch = "wasm32")]
            if let Ok(f) = js_sys::Reflect::get(&js_sys::global(), &"roccaWebGpuFailed".into()) {
                if let Some(f) = wasm_bindgen::JsCast::dyn_ref::<js_sys::Function>(&f) {
                    let _ = f.call0(&wasm_bindgen::JsValue::NULL);
                }
            }
            RenderErrorPolicy::StopRendering
        }));
    }
}

fn inline_handle_lets(mut events: MessageReader<AssetEvent<Shader>>, mut shaders: ResMut<Assets<Shader>>) {
    for e in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::Modified { id }) = e else { continue };
        let Some(Source::Wesl(src)) = shaders.get(*id).map(|s| &s.source) else { continue };
        if let Some(new) = inlined(src) {
            if let Some(mut shader) = shaders.get_mut(*id) {
                shader.source = Source::Wesl(new.into());
            }
        }
    }
}

/// The source with every
/// ```text
/// @if(BINDLESS)
///     let texture = bindless_textures_2d[...];
/// @else
///     let texture = pbr_bindings::base_color_texture;
/// ```
/// removed and `texture` read as `pbr_bindings::base_color_texture` until the
/// next such pair; `None` if there is none.
fn inlined(src: &str) -> Option<String> {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut names: Vec<(String, String)> = Vec::new();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some((name, expr, taken)) = handle_pair(&lines[i..]) {
            match names.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = expr,
                None => names.push((name, expr)),
            }
            i += taken;
            continue;
        }
        out.push_str(&replaced(lines[i], &names));
        i += 1;
    }
    (!names.is_empty()).then_some(out)
}

/// `@if(BINDLESS)`, `let n = bindless...;`, `@else`, `let n = e;` → `(n, e,
/// lines taken)`; either statement may run over several lines.
fn handle_pair(lines: &[&str]) -> Option<(String, String, usize)> {
    // a statement from `lines[at]` to the line ending in `;`: its text and the next line
    let statement = |at: usize| -> Option<(String, usize)> {
        let mut text = String::new();
        for (k, l) in lines.iter().enumerate().skip(at) {
            text.push_str(l.trim());
            if l.trim_end().ends_with(';') {
                return Some((text, k + 1));
            }
        }
        None
    };
    let parse = |t: &str| -> Option<(String, String)> {
        let (name, expr) = t.strip_prefix("let ")?.strip_suffix(';')?.split_once('=')?;
        Some((name.trim().to_string(), expr.trim().to_string()))
    };
    if lines.first()?.trim() != "@if(BINDLESS)" {
        return None;
    }
    let (bindless, next) = statement(1)?;
    if !bindless.contains("bindless_") || lines.get(next)?.trim() != "@else" {
        return None;
    }
    let (bound, end) = statement(next + 1)?;
    let ((n1, _), (n2, expr)) = (parse(&bindless)?, parse(&bound)?);
    (n1 == n2).then_some((n2, expr, end))
}

/// `line` with each identifier in `names` (not a field or a path segment)
/// replaced by its expression.
fn replaced(line: &str, names: &[(String, String)]) -> String {
    if names.is_empty() {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < bytes.len() {
        if ident(bytes[i]) {
            let start = i;
            while i < bytes.len() && ident(bytes[i]) {
                i += 1;
            }
            let word = &line[start..i];
            let after_path = start > 0 && matches!(bytes[start - 1], b'.' | b':');
            let declared = line[..start].trim_end().ends_with("let") || line[..start].trim_end().ends_with("var");
            match names.iter().find(|(n, _)| n == word) {
                Some((_, expr)) if !after_path && !declared => out.push_str(expr),
                _ => out.push_str(word),
            }
        } else {
            let ch = line[i..].chars().next().expect("inside the line");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_textures_are_read_where_they_are_used() {
        let src = "\
@if(VERTEX_UVS)
    if (x) {
        @if(BINDLESS)
        let texture = bindless_textures_2d[material_indices[slot].base_color_texture];
        @else
        let texture = pbr_bindings::base_color_texture;
        @if(BINDLESS)
        let sampler_ = bindless_samplers_filtering[material_indices[slot].base_color_sampler];
        @else
        let sampler_ = pbr_bindings::base_color_sampler;
        c *= textureSampleBias(texture, sampler_, uv, b.texture);
        @if(BINDLESS)
        let texture = bindless_textures_2d[
            material_indices[slot].clearcoat_texture
        ];
        @else
        let texture = pbr_bindings::clearcoat_texture;
        c *= textureSample(texture, pbr_bindings::clearcoat_sampler, uv);
    }
";
        let out = inlined(src).expect("two pairs");
        assert!(!out.contains("let texture") && !out.contains("let sampler_"));
        assert!(out.contains("textureSampleBias(pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv, b.texture);"));
        assert!(out.contains("textureSample(pbr_bindings::clearcoat_texture, pbr_bindings::clearcoat_sampler, uv);"));
        assert_eq!(inlined("fn f() {}\n"), None);
    }

    /// Bevy's own sources, from Cargo's registry: nothing of the pattern is left.
    #[test]
    fn bevys_pbr_shaders_have_no_texture_left_in_a_let() {
        let home = std::env::var("CARGO_HOME").unwrap_or_else(|_| format!("{}/.cargo", std::env::var("HOME").unwrap_or_default()));
        let Some(pbr) = std::fs::read_dir(format!("{home}/registry/src")).into_iter().flatten().flatten()
            .map(|d| d.path().join("bevy_pbr-0.20.0/src/render"))
            .find(|p| p.is_dir())
        else {
            eprintln!("bevy_pbr 0.20.0 not in the registry: skipped");
            return;
        };
        for name in ["pbr_fragment", "pbr_prepass", "pbr_prepass_functions"] {
            let src = std::fs::read_to_string(pbr.join(format!("{name}.wesl"))).expect("a Bevy shader");
            let out = inlined(&src).unwrap_or_else(|| panic!("{name}: nothing rewritten"));
            assert!(!out.contains("let texture") && !out.contains("let sampler_"), "{name}");
        }
    }
}
