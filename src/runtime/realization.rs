//! Where the self-drawn realizations of semantic components enter the
//! environment.
//!
//! A component such as `video` or `map` describes what is on screen, not what
//! draws it. On a platform whose system frameworks own that domain the backend
//! bridges the native primitive and registers nothing here. On a platform with
//! no such primitive the application links `WaterUI`'s own GPU realization,
//! selected by the `video-gpu` and `map-gpu` features, and this is where that
//! realization is installed — from the composition root, before any view
//! resolves, so a backend never has to know which component crates exist.

use waterui_core::Environment;

/// Installs every self-drawn realization this build selected.
///
/// [`App::new`](crate::app::App::new) calls this on the application
/// environment, which is early enough for every view: the environment reaches
/// a view only through the `App` it was handed to. A host that renders without
/// building an `App` — an offscreen preview harness, for one — calls this
/// itself.
#[cfg_attr(
    not(any(feature = "video-gpu", feature = "map-gpu")),
    expect(
        clippy::missing_const_for_fn,
        reason = "the body is empty only in the feature configuration being linted; selecting a realization makes it install one"
    )
)]
pub fn install(env: &mut Environment) {
    #[cfg(feature = "video-gpu")]
    waterui_video_gpu::install(env);
    #[cfg(feature = "map-gpu")]
    install_map(env);
    let _ = env;
}

/// The GPU map yields to a native one: a backend that bridged the platform's
/// map registered its hook on the environment before the application was
/// built, and that bridge is the realization the platform wants.
#[cfg(feature = "map-gpu")]
fn install_map(env: &mut Environment) {
    use waterui_core::view::Hook;
    use waterui_map::MapConfig;

    if env.get::<Hook<MapConfig>>().is_none() {
        waterui_map_gpu::install(env);
    }
}

#[cfg(all(test, any(feature = "video-gpu", feature = "map-gpu")))]
mod tests {
    use waterui_core::{Environment, view::Hook};

    use crate::app::App;
    use crate::component::text;

    /// The realization has to be on the environment the application was built
    /// with, not merely on one someone remembers to prepare: every view a
    /// backend renders reaches its environment through the `App`.
    #[test]
    #[cfg(feature = "map-gpu")]
    fn building_an_app_installs_the_map_realization() {
        let env = Environment::new();
        assert!(env.get::<Hook<waterui_map::MapConfig>>().is_none());
        let app = App::new(|| text("hello"), env);
        assert!(app.env.get::<Hook<waterui_map::MapConfig>>().is_some());
    }

    /// A backend that bridged the platform's own map registered its hook before
    /// the application was built, and that bridge wins.
    #[test]
    #[cfg(feature = "map-gpu")]
    fn a_native_map_bridge_is_left_alone() {
        use std::cell::Cell;
        use std::rc::Rc;

        use waterui_core::AnyView;

        let bridged = Rc::new(Cell::new(false));
        let mut env = Environment::new();
        env.insert_hook::<waterui_map::MapConfig, AnyView>({
            let bridged = Rc::clone(&bridged);
            move |_env, _config| {
                bridged.set(true);
                AnyView::new(text("native map"))
            }
        });

        let app = App::new(|| text("hello"), env);
        let hook = app
            .env
            .get::<Hook<waterui_map::MapConfig>>()
            .expect("the bridge's hook must survive");
        let region = waterui_map::Region::new(waterui_map::Coordinate::default(), 1.0, 1.0);
        let config = waterui_core::view::ConfigurableView::config(waterui_map::Map::new(region));
        let _ = hook.apply(&app.env, config);
        assert!(
            bridged.get(),
            "the map must still be drawn by the platform bridge"
        );
    }

    #[test]
    #[cfg(feature = "video-gpu")]
    fn building_an_app_installs_the_video_realization() {
        let env = Environment::new();
        assert!(env.get::<Hook<waterui_video::VideoConfig>>().is_none());
        let app = App::new(|| text("hello"), env);
        assert!(app.env.get::<Hook<waterui_video::VideoConfig>>().is_some());
    }
}
