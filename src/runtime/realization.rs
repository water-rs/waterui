//! Where the self-drawn realizations of semantic components enter the
//! environment.
//!
//! A component such as `video` describes what is on screen, not what draws it.
//! On a platform whose system frameworks own that domain the backend bridges
//! the native primitive and registers nothing here. On a platform with no such
//! primitive the application links `WaterUI`'s own GPU realization, selected by
//! the `video-gpu` feature, and this is where that realization is installed —
//! from the composition root, before any view resolves, so a backend never has
//! to know which component crates exist.
//!
//! Only the realizations `waterui` itself carries are installed here. A
//! component that lives in a crate of its own — `waterui-map-gpu`, say — is a
//! direct dependency of the application, and the application installs it in its
//! own `app(env)` exactly as it installs a browser engine.

use waterui_core::Environment;

/// Installs every self-drawn realization this build selected.
///
/// [`App::new`](crate::app::App::new) calls this on the application
/// environment, which is early enough for every view: the environment reaches
/// a view only through the `App` it was handed to. A host that renders without
/// building an `App` — an offscreen preview harness, for one — calls this
/// itself.
#[cfg_attr(
    not(feature = "video-gpu"),
    expect(
        clippy::missing_const_for_fn,
        reason = "the body is empty only in the feature configuration being linted; selecting a realization makes it install one"
    )
)]
pub fn install(env: &mut Environment) {
    #[cfg(feature = "video-gpu")]
    waterui_video_gpu::install(env);
    let _ = env;
}

#[cfg(all(test, feature = "video-gpu"))]
mod tests {
    use waterui_core::{Environment, view::Hook};

    use crate::app::App;
    use crate::component::text;

    /// The realization has to be on the environment the application was built
    /// with, not merely on one someone remembers to prepare: every view a
    /// backend renders reaches its environment through the `App`.
    #[test]
    fn building_an_app_installs_the_video_realization() {
        let env = Environment::new();
        assert!(env.get::<Hook<waterui_video::VideoConfig>>().is_none());
        let app = App::new(|| text("hello"), env);
        assert!(app.env.get::<Hook<waterui_video::VideoConfig>>().is_some());
    }
}
