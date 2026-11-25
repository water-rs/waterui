//! Every public capability must be reachable from a crate that depends only on
//! `waterui`.
//!
//! These are compile-time assertions: an app author should never have to add a
//! direct `waterui-core` (or `filtrate`, or `wgpu`) dependency, nor reach
//! through an inconsistent long path, to use a documented feature. Each `use`
//! below stands for one gap that used to force exactly that.

#![allow(unused_imports, reason = "these imports are the assertion")]

// Tabs sit beside the split-view navigation types instead of behind
// `navigation::tab::…`.
use waterui::navigation::{
    NavigationSplitView, Tab, Tabs, navigation_transition, split_style, tab_style,
};

// The `slider` free function joins `button`/`toggle`/`stepper`/`field`/`label`
// at the controls root.
use waterui::component::{button, field, label, slider, stepper, toggle};

// Custom extractors reach the single `State` environment channel through `Extractor` and `State`.
use waterui::extract::{Extractor, State};

// Implementing a custom resolvable theme token.
use waterui::resolve::{AnyResolvable, Resolvable};

// View metadata and lifecycle events.
use waterui::event::Event;
use waterui::metadata::{Metadata, MetadataKey, Retain};

// Implementing `Animatable` for a custom data shape.
use waterui::easing::{EasingCurve, Interpolatable};

// Declarative sensory feedback and imperative haptic triggers.
#[cfg(feature = "haptics")]
use waterui::haptic::{
    Feedback as HapticFeedback, Haptic as HapticEngine, Intensity as HapticIntensity,
};
#[cfg(feature = "haptics")]
use waterui::{Feedback, Haptic, Intensity};

// `.filter(F)` requires `F: Reach + RenderTransfer` (`Reach` covers every
// `Filter` of a colour or spatial kind) and `.effect(E)` requires
// `E: Effect + RenderTransfer`; the filter set, the `Filter`, `Reach` and
// `Effect` traits with the types an effect is written in, and the
// render-transfer trait answer at `graphics::filter_view` so an app links the
// same copies the engine runs.
#[cfg(feature = "gpu")]
use waterui::graphics::RenderTransfer;
#[cfg(feature = "gpu")]
use waterui::graphics::filter_view::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRenderResult, EffectSetupResult,
    Filter, Reach, RetainedMemory, SHADER_EFFECT_MAX_PARAMS, ShaderEffect, ShaderEffectError,
    TextureViews, filters,
};

// A `GpuContent` author's whole vocabulary answers at `graphics::gpu`: the
// `wgpu` and `bytemuck` this build links rather than a hand-added version
// that may not match, the frame timing and readback types, the
// external-frame interop and the engine backend `OffscreenRenderer` is
// generic over.
#[cfg(feature = "gpu")]
use waterui::graphics::gpu::{
    Display, FrameTime, Gpu, Offscreen, OffscreenFormat, Readback, bytemuck, interop, wgpu,
};

#[test]
fn public_capabilities_are_reachable_through_the_facade() {
    // Reaching this point means every import above resolved.
    let _ = Metadata::new(waterui::component::text("water"), Retain::new(0_u8));
}

#[test]
fn prelude_exports_app_window_widgets_and_collections_without_shadowing() {
    use std::io;
    use waterui::prelude::*;

    #[derive(Clone, Identifiable)]
    struct RowItem {
        #[id]
        id: i32,
    }

    fn sample_app(env: Environment) -> App<impl Windows, impl AppMenuBar> {
        let _ = WindowState::Normal;
        let _ = Window::new("Title", || {
            let items = StateList::from(vec![RowItem { id: 1 }, RowItem { id: 2 }]);
            let _ = items.len();
            let flag = Binding::bool(true);
            let _ = when(flag, || "ok");
            let _ = disclosure("Header", || "Body");
            let _ = error_view(io::Error::other("err"));
            let _ = ErrorView::new(io::Error::other("err"));
            let _ = Suspense::new(async { "ready" });
            let _ = SignalCollection::new(Binding::container(vec![RowItem { id: 3 }]));
            let _ = ForEach::new(items, |item| text!("{id}", id = item.id));
            let presented = Binding::bool(false);
            let _ = "Main".sheet(Sheet::new(&presented, || "Sheet"));
            "Root"
        });
        App::new(env).window(Window::new("Hello", || text!("Hello")))
    }
    let _ = sample_app;
}
