//! WESL modules shared between the game's shaders, and how the game's
//! shaders are registered.

use bevy::asset::uuid_handle;
use bevy::prelude::*;

const NOISE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("2d6ac0f3-5e19-4b87-8a41-7c03e9b51d62");

const SCENE_LIGHT_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("5b1e93a7-c4d2-4f08-b6e3-1a7d28c9f054");

const FADED_SHADOW_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("5b1e93a7-c4d2-4f08-b6e3-1a7d28c9f055");

/// Registers `game::shaders::noise` (`shaders/noise.wesl`),
/// `game::shaders::scene_light` and `game::shaders::faded_shadow`. Shaders
/// `import` them by path, so the order of the plugins does not matter.
pub struct ShaderLibPlugin;

impl Plugin for ShaderLibPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, NOISE_SHADER_HANDLE, "shaders/noise.wesl");
        crate::load_wesl!(app, SCENE_LIGHT_SHADER_HANDLE, "shaders/scene_light.wesl");
        crate::load_wesl!(app, FADED_SHADOW_SHADER_HANDLE, "shaders/faded_shadow.wesl");
    }
}

/// Puts a WESL shader (Bevy 0.20 dropped naga_oil's WGSL dialect) under a
/// fixed handle, importable by the other shaders as `game::shaders::<stem>`.
#[macro_export]
macro_rules! load_wesl {
    ($app:expr, $handle:expr, $path:literal) => {
        $crate::shader_lib::insert_wesl($app, &$handle, include_str!($path), $path)
    };
}

pub fn insert_wesl(app: &mut App, handle: &Handle<Shader>, source: &'static str, path: &str) {
    let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).expect("a shader file name");
    // WebGPU's compiler in the browser rejects a derivative after a branch on
    // anything not uniform (an instance's finish, a fragment's distance),
    // which naga lets through: the rule is turned off where derivatives are
    // taken. Once per program: in the shaders with an entry point, which the
    // imported modules are composed into.
    #[cfg(feature = "webgpu")]
    let source: std::borrow::Cow<'static, str> = if source.contains("@fragment") {
        derivatives_anywhere(source).into()
    } else {
        source.into()
    };
    let shader = Shader::from_wesl(source, format!("embedded://game/shaders/{stem}.wesl"));
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(handle.id(), shader)
        .expect("a fixed shader handle");
}

/// `source` with the derivative uniformity rule off: WGSL wants the directive
/// after WESL's imports and before any declaration.
#[cfg(any(feature = "webgpu", test))]
fn derivatives_anywhere(source: &str) -> String {
    let mut at = 0;
    let mut in_import = false;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let t = line.trim();
        let starts = t.starts_with("import ") || (t.starts_with('@') && t.contains(" import "));
        if starts || in_import {
            in_import = !t.contains(';');
            at = offset + line.len();
        } else if !(t.is_empty() || t.starts_with("//")) {
            break;
        }
        offset += line.len();
    }
    format!("{}diagnostic(off, derivative_uniformity);\n{}", &source[..at], &source[at..])
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_directive_goes_after_the_imports() {
        let src = "// a\nimport a::b;\n@if(X) import c::{d,\n  e};\n\nfn f() {}\n";
        assert_eq!(
            super::derivatives_anywhere(src),
            "// a\nimport a::b;\n@if(X) import c::{d,\n  e};\ndiagnostic(off, derivative_uniformity);\n\nfn f() {}\n"
        );
        assert!(super::derivatives_anywhere("fn f() {}").starts_with("diagnostic("));
    }
}
