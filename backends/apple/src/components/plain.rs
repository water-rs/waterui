//! The `plain` leaf: `Native<Str>` rendered through a kit [`Label`].
//!
//! Mirrors `WuiPlain`: body text drawn in the theme's body font and
//! foreground color, rendered through the same attributed form as styled
//! text so the slot's line pitch and letter spacing reach it. The label's
//! measurement answers the leaf's `sizeThatFits`, baseline guides included.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::Retained;
use waterui::Str;
use waterui::animation::Animation;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata;
use waterui::resolve::Resolvable;
use waterui::text::font::{Body, FontDesign, FontWeight, ResolvedFont};
use waterui::theme::color::Foreground;
use waterui_core::layout::{
    ProposalSize, Size, StretchAxis, SubView, VerticalAlignment, ViewDimensions,
};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub use cocoa_ui::appkit::Label;
}
#[cfg(target_os = "ios")]
mod platform {
    pub use cocoa_ui::uikit::Label;
}
use platform::Label;

/// The label as its platform view, for animations.
fn as_view(label: &Label) -> &cocoa_ui::PlatformView {
    label
}

/// The cross-dissolve duration the metadata calls for, in seconds — the
/// same table `components::text` uses.
fn cross_dissolve_duration(metadata: &Metadata) -> Option<f64> {
    match metadata.try_get::<Animation>() {
        None => None,
        Some(Animation::Default) => Some(0.25),
        Some(Animation::Bezier { duration, .. }) => Some(duration.as_secs_f64()),
        Some(Animation::Spring { .. }) => Some(0.15),
    }
}

/// The platform weight of a `FontWeight` on the `UIFont`/`NSFont` scale.
const fn platform_weight(weight: FontWeight) -> f64 {
    use cocoa_ui::font::weight;
    match weight {
        FontWeight::Thin => weight::THIN,
        FontWeight::UltraLight => weight::ULTRA_LIGHT,
        FontWeight::Light => weight::LIGHT,
        FontWeight::Normal => weight::REGULAR,
        FontWeight::Medium => weight::MEDIUM,
        FontWeight::SemiBold => weight::SEMI_BOLD,
        FontWeight::Bold => weight::BOLD,
        FontWeight::UltraBold => weight::HEAVY,
        FontWeight::Black => weight::BLACK,
    }
}

/// The comma-separated candidates of a CSS-style family list, trimmed with
/// the empties dropped.
fn family_candidates(family: &str) -> impl Iterator<Item = &str> {
    family
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
}

/// The platform face a resolved font names.
///
/// # Panics
///
/// When the family names no installed font and no generic — the same
/// `fatalError` the Swift port raises.
fn platform_font(
    mtm: cocoa_ui::MainThreadMarker,
    resolved: &ResolvedFont,
) -> Retained<cocoa_ui::Font> {
    let size = f64::from(resolved.size);
    let weight = platform_weight(resolved.weight);
    match resolved.family.as_deref() {
        Some(family) if !family.is_empty() => {
            let mut resolved_font = None;
            for candidate in family_candidates(family) {
                resolved_font = match candidate {
                    "system" | "sans-serif" => Some(cocoa_ui::font::system(mtm, size, weight)),
                    _ => cocoa_ui::font::named(candidate, size),
                };
                if resolved_font.is_some() {
                    break;
                }
            }
            resolved_font.unwrap_or_else(|| {
                panic!(
                    "WaterUI: font family '{family}' not found. Ensure the font is bundled and registered."
                )
            })
        }
        _ => match resolved.design {
            FontDesign::Default => cocoa_ui::font::system(mtm, size, weight),
            FontDesign::Monospaced => cocoa_ui::font::monospaced(mtm, size, weight),
        },
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_ui_kit::UIColor> {
    {
        let [red, green, blue, alpha] = color.components;
        cocoa_ui::uikit::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object, with HDR
/// headroom applied as a content-headroom multiplier.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_app_kit::NSColor> {
    {
        let [red, green, blue, alpha] = color.components;
        cocoa_ui::appkit::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The leaf's live state: the content plus the latest resolved theme
/// values, rebuilt into the label on any change.
struct PlainState {
    /// Proof of the main thread for kit calls inside watchers.
    mtm: cocoa_ui::MainThreadMarker,
    /// The platform label.
    label: Retained<Label>,
    /// The plain string.
    text: Str,
    /// The latest resolved body font.
    font: ResolvedFont,
    /// The latest resolved theme foreground.
    foreground: WorkingColor,
}

impl core::fmt::Debug for PlainState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PlainState").finish_non_exhaustive()
    }
}

/// `render()`: the one attributed run — body font, theme foreground — onto
/// the label, then intrinsic-size invalidation (`invalidateCapturedRendering`).
fn render(state: &PlainState, duration: Option<f64>) {
    let font = platform_font(state.mtm, &state.font);
    let foreground = platform_color(&state.foreground);
    let run = cocoa_ui::text::TextRun {
        text: state.text.as_str(),
        font: &font,
        foreground: Some(&*foreground),
        background: None,
        underline: false,
        strikethrough: false,
        letter_spacing: f64::from(state.font.letter_spacing),
        line_height: state.font.line_height.map_or(0.0, f64::from),
    };
    let attributed = cocoa_ui::text::build(state.mtm, &[run]);
    let label = state.label.clone();
    match duration {
        Some(seconds) => {
            let view = as_view(&state.label);
            cocoa_ui::core_animation::cross_dissolve(view, seconds, move || {
                label.set_attributed_text(&attributed);
            });
        }
        None => state.label.set_attributed_text(&attributed),
    }
    cocoa_ui::view::invalidate_layout(&state.label);
    crate::measure_memo::invalidate();
}

/// The label's layout face: intrinsic, baseline-aware, non-stretching.
struct PlainSubView {
    /// The label whose laid-out text the measure reports.
    label: Retained<Label>,
}

impl core::fmt::Debug for PlainSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PlainSubView").finish_non_exhaustive()
    }
}

impl SubView for PlainSubView {
    // `TextMetrics` measures in f64; `ViewDimensions` speaks f32 — the
    // narrowing is the layout contract.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let wrap = match proposal.width {
            None => cocoa_ui::text::WrapWidth::Free,
            Some(width) if width > 0.0 => cocoa_ui::text::WrapWidth::Fixed(f64::from(width)),
            Some(_) => cocoa_ui::text::WrapWidth::Unbreakable,
        };
        let metrics = self.label.measure(wrap);
        let mut dimensions = ViewDimensions::new(Size::new(
            metrics.size.width as f32,
            metrics.size.height as f32,
        ));
        if let Some(first) = metrics.first_baseline {
            dimensions = dimensions.with_vertical(VerticalAlignment::FirstBaseline, first as f32);
        }
        if let Some(last) = metrics.last_baseline {
            dimensions = dimensions.with_vertical(VerticalAlignment::LastBaseline, last as f32);
        }
        dimensions
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `plain` handler on the dispatcher: `Native<Str>` maps to a
/// kit label whose attributed string rebuilds when the body font slot or
/// foreground slot changes.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Str>(|text, ctx| {
        let mtm = ctx.mtm();
        let label = Label::new(mtm);
        label.set_line_limit(0);

        let body = Body.resolve(ctx.env());
        let foreground = Foreground.resolve(ctx.env());
        let state = Rc::new(RefCell::new(PlainState {
            mtm,
            label: label.clone(),
            text,
            font: body.snapshot(),
            foreground: foreground.snapshot(),
        }));
        render(&state.borrow(), None);

        let mut leaf = NativeLeaf::new(
            as_view(&label),
            PlainSubView {
                label: label.clone(),
            },
        );

        leaf.watch(&body, {
            let state = Rc::clone(&state);
            move |ctx| {
                let duration = cross_dissolve_duration(ctx.metadata());
                state.borrow_mut().font = ctx.into_value();
                render(&state.borrow(), duration);
            }
        });
        leaf.watch(&foreground, {
            let state = Rc::clone(&state);
            move |ctx| {
                let duration = cross_dissolve_duration(ctx.metadata());
                state.borrow_mut().foreground = ctx.into_value();
                render(&state.borrow(), duration);
            }
        });
        leaf.keep(state);
        leaf
    });
}
