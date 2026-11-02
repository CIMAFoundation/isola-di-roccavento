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
    let shader = Shader::from_wesl(source, format!("embedded://game/shaders/{stem}.wesl"));
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(handle.id(), shader)
        .expect("a fixed shader handle");
}
