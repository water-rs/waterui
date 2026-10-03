//! The registration table: every claim the backend makes, in one place.
//!
//! A component port adds a `components/<name>.rs` module exposing
//! `pub(crate) fn install(&mut Dispatcher)`, a `#[cfg(feature = "<name>")]`
//! line here, and the matching Cargo feature. This file is the shared merge
//! point the coordinator integrates.
//!
//! Entries must be sorted by the group they belong to; the comment names the
//! wave group they arrived with.

use crate::dispatch::Dispatcher;

/// Fills `dispatcher` with every claim the backend owns. Called exactly
/// once, before the first render, inside [`crate::dispatch::dispatcher`].
#[allow(clippy::too_many_lines)] // a flat registration table is meant to be long
pub fn install(dispatcher: &mut Dispatcher) {
    // Core — the unit view, always claimed: an unclaimed `Native<()>` falls
    // through the walk into a `body()` panic, which `panic = "abort"` makes fatal.
    crate::components::empty::install(dispatcher);

    // Wave A — trivial leaves.
    #[cfg(feature = "image")]
    crate::components::image::install(dispatcher);
    #[cfg(feature = "resolved_color")]
    crate::components::resolved_color::install(dispatcher);
    #[cfg(feature = "slider")]
    crate::components::slider::install(dispatcher);
    #[cfg(feature = "progress")]
    crate::components::progress::install(dispatcher);
    #[cfg(feature = "stepper")]
    crate::components::stepper::install(dispatcher);
    #[cfg(feature = "text")]
    crate::components::text::install(dispatcher);
    #[cfg(feature = "button")]
    crate::components::button::install(dispatcher);

    // Wave B — pickers.
    #[cfg(feature = "date_picker")]
    crate::components::date_picker::install(dispatcher);
    #[cfg(feature = "multi_date_picker")]
    crate::components::multi_date_picker::install(dispatcher);
    #[cfg(feature = "picker")]
    crate::components::picker::install(dispatcher);
    #[cfg(feature = "color_picker")]
    crate::components::color_picker::install(dispatcher);
    #[cfg(feature = "toggle")]
    crate::components::toggle::install(dispatcher);
    #[cfg(feature = "text_field")]
    crate::components::text_field::install(dispatcher);
    #[cfg(feature = "secure_field")]
    crate::components::secure_field::install(dispatcher);
    #[cfg(feature = "spacer")]
    crate::components::spacer::install(dispatcher);

    // Containers — the layout containers of the arrangement wave.
    #[cfg(feature = "fixed_container")]
    crate::components::fixed_container::install(dispatcher);
    #[cfg(feature = "container")]
    crate::components::container::install(dispatcher);
    #[cfg(feature = "plain")]
    crate::components::plain::install(dispatcher);
    #[cfg(feature = "dynamic")]
    crate::components::dynamic::install(dispatcher);

    // Metadata — transparent wrappers claiming `Metadata<M>` ahead of the
    // walk's `body()` expansion.
    #[cfg(feature = "anchored_overlay")]
    crate::components::anchored_overlay::install(dispatcher);
    #[cfg(feature = "draggable")]
    crate::components::draggable::install(dispatcher);
    #[cfg(feature = "drop_destination")]
    crate::components::drop_destination::install(dispatcher);
    #[cfg(feature = "focused")]
    crate::components::focused::install(dispatcher);
    #[cfg(feature = "gesture")]
    crate::components::gesture::install(dispatcher);
    #[cfg(feature = "layout_priority")]
    crate::components::layout_priority::install(dispatcher);
    #[cfg(feature = "menu")]
    crate::components::menu::install(dispatcher);

    // Visual metadata — surfaces: clip masks, dynamic-range tags, and the
    // material/glass backgrounds.
    #[cfg(feature = "clip_shape")]
    crate::components::clip_shape::install(dispatcher);
    #[cfg(feature = "dynamic_range")]
    crate::components::dynamic_range::install(dispatcher);
    #[cfg(feature = "glass_background")]
    crate::components::glass_background::install(dispatcher);
    #[cfg(feature = "material_background")]
    crate::components::material_background::install(dispatcher);

    // Visual metadata — transforms and decorations: purely visual wrappers
    // that move or decorate their content without touching layout.
    #[cfg(feature = "offset")]
    crate::components::offset::install(dispatcher);
    #[cfg(feature = "rotation")]
    crate::components::rotation::install(dispatcher);
    #[cfg(feature = "scale")]
    crate::components::scale::install(dispatcher);
    #[cfg(feature = "border")]
    crate::components::border::install(dispatcher);
    #[cfg(feature = "shadow")]
    crate::components::shadow::install(dispatcher);
    #[cfg(feature = "opacity")]
    crate::components::opacity::install(dispatcher);
    // Environment + lifecycle metadata — the environment overlay,
    // safe-area escape, capture protection, one-shot hooks and retention.
    #[cfg(feature = "ignore_safe_area")]
    crate::components::ignore_safe_area::install(dispatcher);
    #[cfg(feature = "lifecycle_hook")]
    crate::components::lifecycle_hook::install(dispatcher);
    #[cfg(feature = "retain")]
    crate::components::retain::install(dispatcher);
    #[cfg(feature = "secure")]
    crate::components::secure::install(dispatcher);
    #[cfg(feature = "with_env")]
    crate::components::with_env::install(dispatcher);

    // Interaction metadata — the non-gesture interaction leaves.
    #[cfg(feature = "accessibility_identifier")]
    crate::components::accessibility_identifier::install(dispatcher);
    #[cfg(feature = "accessibility_metadata")]
    crate::components::accessibility_metadata::install(dispatcher);
    #[cfg(feature = "context_menu")]
    crate::components::context_menu::install(dispatcher);
    #[cfg(feature = "cursor")]
    crate::components::cursor::install(dispatcher);
    #[cfg(feature = "hittable")]
    crate::components::hittable::install(dispatcher);
    #[cfg(feature = "on_event")]
    crate::components::on_event::install(dispatcher);
    #[cfg(feature = "on_key_press")]
    crate::components::on_key_press::install(dispatcher);

    // Navigation — the group: metadata claims first (they wrap any content),
    // then the container and leaf components.
    #[cfg(feature = "navigation")]
    crate::components::navigation::install(dispatcher);

    // Media — the web view leaf; its `WebViewController` is installed late
    // by `waterui_apple_install_webview`, not here.
    #[cfg(feature = "webview")]
    crate::components::webview::install(dispatcher);

    // Wave B — containers.
    #[cfg(feature = "list")]
    crate::components::list::install(dispatcher);
    #[cfg(feature = "scroll")]
    crate::components::scroll::install(dispatcher);
    #[cfg(feature = "table")]
    crate::components::table::install(dispatcher);

    // Data — the map leaf, bridging `MapConfig` onto `MKMapView`.
    #[cfg(feature = "map")]
    crate::components::map::install(dispatcher);

    // Wave C — overlays.
    #[cfg(feature = "badge")]
    crate::components::badge::install(dispatcher);

    // GPU/graphics — the surface leaf first: the filtered leaf's capture
    // resolves mounted surfaces through its registry.
    #[cfg(feature = "gpu_surface")]
    crate::components::gpu_surface::install(dispatcher);
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    crate::components::filtered::install(dispatcher);
    #[cfg(feature = "picture")]
    crate::components::picture::install(dispatcher);
    #[cfg(feature = "resolved_gradient")]
    crate::components::resolved_gradient::install(dispatcher);
    #[cfg(feature = "resolved_shape")]
    crate::components::resolved_shape::install(dispatcher);

    // Media — the AVPlayer-backed video leaves.
    #[cfg(any(feature = "video", feature = "video_player"))]
    crate::components::video::install(dispatcher);
}
