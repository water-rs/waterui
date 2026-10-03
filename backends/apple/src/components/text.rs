//! The `text` leaf: `Native<TextConfig>` rendered through the kit's label.
//!
//! Mirrors `WuiText` + `WuiTextBase`: the `content` `Computed<StyledStr>`
//! re-resolves each chunk's font and color signals, `paragraph_alignment`
//! pushes to the platform's text alignment, and `line_limit` maps onto the
//! label's break mode. Every update is an imperative kit call inside a
//! watcher — no signal type crosses into `cocoa-ui`.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;
use core::num::{NonZero, NonZeroUsize};

use cocoa_ui::Retained;
use waterui::Str;
use waterui::animation::Animation;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata;
use waterui::resolve::Resolvable;
use waterui::text::StyledStr;
use waterui::text::TextConfig;
use waterui::text::font::{FontDesign, FontWeight, ResolvedFont};
use waterui::theme::color::Foreground;
use waterui_backend_core::Environment;
use waterui_core::layout::{
    HorizontalAlignment, ProposalSize, Size, StretchAxis, SubView, VerticalAlignment,
    ViewDimensions,
};

use crate::contract::{NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::HostView;
    pub(super) use cocoa_ui::appkit::Label;
    pub(super) use cocoa_ui::appkit::colors;
    pub(super) use cocoa_ui::objc2_app_kit::NSTextAlignment;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::NSTextAlignment;
    pub(super) use cocoa_ui::uikit::HostView;
    pub(super) use cocoa_ui::uikit::Label;
    pub(super) use cocoa_ui::uikit::colors;
}

use cocoa_ui::PlatformView;
use platform::{HostView, Label};

/// The label as its platform view, for animations and layout direction.
fn as_view(label: &Label) -> &PlatformView {
    label
}

/// A styled chunk resolved into its current values: the text plus the
/// attributes the attributed-string builder consumes.
struct Chunk {
    /// The chunk's characters.
    text: Str,
    /// Its latest resolved font.
    font: ResolvedFont,
    /// Its latest foreground — `None` draws the theme default.
    foreground: Option<WorkingColor>,
    /// Its latest background.
    background: Option<WorkingColor>,
    /// Whether the chunk is italicized.
    italic: bool,
    /// Whether the chunk is underlined.
    underline: bool,
    /// Whether the chunk is struck through.
    strikethrough: bool,
}

/// The leaf's live state: what the content watcher and the per-chunk signal
/// watchers read and mutate. `KeepAlive` holds the `Rc`; the guards inside
/// stop their subscriptions when the leaf drops.
struct TextState {
    /// Proof of the main thread for kit calls inside watchers.
    mtm: cocoa_ui::MainThreadMarker,
    /// The environment this leaf resolves signals through.
    env: Environment,
    /// The platform label.
    label: Retained<Label>,
    /// The theme default for chunks without their own foreground.
    default_foreground: WorkingColor,
    /// The resolved chunks of the current `StyledStr`.
    chunks: Vec<Chunk>,
    /// Watcher guards of the current `StyledStr`'s per-chunk signals,
    /// replaced wholesale when the content changes.
    signal_guards: Vec<Box<dyn Any>>,
    /// The theme-foreground guard — set once; the theme slot does not belong
    /// to the content, so it survives `apply_styled`. The field is never read
    /// after creation: holding it keeps the subscription alive.
    default_guard: Option<Box<dyn Any>>,
}

/// The cross-dissolve duration the metadata calls for, in seconds: `None`
/// applies the change directly — the same table `withCrossDissolveAnimation`
/// uses.
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
/// the empties dropped — `fontFamilyCandidates` in the Swift port.
fn family_candidates(family: &str) -> impl Iterator<Item = &str> {
    family
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
}

/// The platform face a resolved font names: a named family resolved through
/// its candidate list — generics first, then `fontWithName:` — or the design
/// face, italicized when the chunk asks for it.
///
/// # Panics
///
/// When the family names no installed font and no generic — the same
/// `fatalError` the Swift port raises.
fn platform_font(
    mtm: cocoa_ui::MainThreadMarker,
    resolved: &ResolvedFont,
    italic: bool,
) -> Retained<cocoa_ui::Font> {
    let size = f64::from(resolved.size);
    let weight = platform_weight(resolved.weight);
    let mut font = match resolved.family.as_deref() {
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
    };
    if italic && let Some(italic_font) = cocoa_ui::font::italic_variant(&font) {
        font = italic_font;
    }
    font
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
#[cfg(target_os = "ios")]
pub fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_ui_kit::UIColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object, with HDR
/// headroom applied as a content-headroom multiplier — the `AppKit` variant.
#[cfg(target_os = "macos")]
pub fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_app_kit::NSColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The attributed string the current chunks render into: font, colors and
/// decorations resolved per chunk, the theme default under unstyled
/// foregrounds.
fn render_attributed(state: &TextState) -> Retained<objc2_foundation::NSAttributedString> {
    let fonts: Vec<Retained<cocoa_ui::Font>> = state
        .chunks
        .iter()
        .map(|chunk| platform_font(state.mtm, &chunk.font, chunk.italic))
        .collect();
    let foregrounds: Vec<Option<Retained<_>>> = state
        .chunks
        .iter()
        .map(|chunk| {
            Some(platform_color(
                chunk
                    .foreground
                    .as_ref()
                    .unwrap_or(&state.default_foreground),
            ))
        })
        .collect();
    let backgrounds: Vec<Option<Retained<_>>> = state
        .chunks
        .iter()
        .map(|chunk| chunk.background.as_ref().map(platform_color))
        .collect();
    let runs: Vec<cocoa_ui::text::TextRun<'_>> = state
        .chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| cocoa_ui::text::TextRun {
            text: chunk.text.as_str(),
            font: &fonts[i],
            foreground: foregrounds[i].as_deref(),
            background: backgrounds[i].as_deref(),
            underline: chunk.underline,
            strikethrough: chunk.strikethrough,
            letter_spacing: f64::from(chunk.font.letter_spacing),
            line_height: chunk.font.line_height.map_or(0.0, f64::from),
        })
        .collect();
    cocoa_ui::text::build(state.mtm, &runs).into_super()
}

/// Pushes the rebuilt attributed string onto the label, animating the swap
/// when the watcher metadata asks for it.
fn rebuild(state: &TextState, duration: Option<f64>) {
    crate::measure_memo::invalidate();
    let attributed = render_attributed(state);
    match duration {
        Some(seconds) => {
            let label = state.label.clone();
            cocoa_ui::core_animation::cross_dissolve(as_view(&state.label), seconds, move || {
                label.set_attributed_text(&attributed);
            });
        }
        None => state.label.set_attributed_text(&attributed),
    }
}

/// Resolves a `StyledStr` into chunk state plus the guards that keep its
/// signals subscribed — the work `WuiStyledStrRenderer` performs.
fn apply_styled(state: &Rc<RefCell<TextState>>, styled: &StyledStr) {
    let env = state.borrow().env.clone();
    let mut chunks = Vec::with_capacity(styled.chunks().len());
    let mut guards: Vec<Box<dyn Any>> = Vec::new();
    for (index, (text, style)) in styled.chunks().iter().enumerate() {
        let font = style.font.resolve(&env);
        let foreground = style.foreground.as_ref().map(|color| color.resolve(&env));
        let background = style.background.as_ref().map(|color| color.resolve(&env));
        chunks.push(Chunk {
            text: text.clone(),
            font: font.snapshot(),
            foreground: foreground.as_ref().map(Signal::snapshot),
            background: background.as_ref().map(Signal::snapshot),
            italic: style.italic,
            underline: style.underline,
            strikethrough: style.strikethrough,
        });

        guards.push(Box::new(font.watch({
            let state = Rc::clone(state);
            move |ctx| {
                let duration = cross_dissolve_duration(ctx.metadata());
                let mut state = state.borrow_mut();
                state.chunks[index].font = ctx.into_value();
                rebuild(&state, duration);
            }
        })));
        if let Some(signal) = foreground {
            guards.push(Box::new(signal.watch({
                let state = Rc::clone(state);
                move |ctx| {
                    let duration = cross_dissolve_duration(ctx.metadata());
                    let mut state = state.borrow_mut();
                    state.chunks[index].foreground = Some(ctx.into_value());
                    rebuild(&state, duration);
                }
            })));
        }
        if let Some(signal) = background {
            guards.push(Box::new(signal.watch({
                let state = Rc::clone(state);
                move |ctx| {
                    let duration = cross_dissolve_duration(ctx.metadata());
                    let mut state = state.borrow_mut();
                    state.chunks[index].background = Some(ctx.into_value());
                    rebuild(&state, duration);
                }
            })));
        }
    }
    let mut state = state.borrow_mut();
    state.chunks = chunks;
    state.signal_guards = guards;
}

/// The label's layout face: intrinsic, baseline-aware, non-stretching.
struct TextSubView {
    /// The label whose laid-out text the measure reports.
    label: Retained<Label>,
}

impl core::fmt::Debug for TextSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextSubView").finish_non_exhaustive()
    }
}

impl SubView for TextSubView {
    // `TextMetrics` measures in f64; `ViewDimensions` speaks f32 — the
    // narrowing is the layout contract.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let metrics = self.label.measure(wrap_for(proposal.width));
        // `cocoa_ui::text::measure` reports the cell-fitting width, two
        // points past the bare text bounds; `WuiTextBase` measures the bare
        // bounds, so the inset comes back out here.
        let mut dimensions = ViewDimensions::new(Size::new(
            (metrics.size.width - 2.0).max(0.0) as f32,
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

/// The wrap mode a width proposal implies: none means natural width, a
/// positive bound wraps at it, and zero/negative width can only fit the
/// widest unbreakable run.
fn wrap_for(proposal_width: Option<f32>) -> cocoa_ui::text::WrapWidth {
    match proposal_width {
        None => cocoa_ui::text::WrapWidth::Free,
        Some(width) if width > 0.0 => cocoa_ui::text::WrapWidth::Fixed(f64::from(width)),
        Some(_) => cocoa_ui::text::WrapWidth::Unbreakable,
    }
}

/// The `HorizontalAlignment` mapping `WuiText` applies: `Center` centers,
/// `Trailing` flips with the reading direction, anything else stays natural.
fn apply_alignment(label: &Label, alignment: HorizontalAlignment) {
    use platform::NSTextAlignment;
    let alignment = if alignment == HorizontalAlignment::Center {
        NSTextAlignment::Center
    } else if alignment == HorizontalAlignment::Trailing {
        if cocoa_ui::view::is_right_to_left(as_view(label)) {
            NSTextAlignment::Left
        } else {
            NSTextAlignment::Right
        }
    } else {
        NSTextAlignment::Natural
    };
    label.set_text_alignment(alignment);
    crate::measure_memo::invalidate();
}

/// Builds a label leaf around `styled` at `line_limit`/`alignment`, sharing
/// `WuiTextBase`'s shape: the leaf is a container whose measured size is the
/// bare text bounds while the label inside keeps its cell-fitting width, so
/// text ink never clips when the two disagree by the cell's insets.
fn label_leaf(
    ctx: &RenderContext,
    line_limit: Option<NonZeroUsize>,
    styled: &StyledStr,
    alignment: HorizontalAlignment,
) -> (NativeLeaf, Rc<RefCell<TextState>>, Retained<Label>) {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, cocoa_ui::geometry::Rect::ZERO);
    // `labelWithString:` keeps the minimal cell insets of an AppKit label;
    // `Label::new` (`initWithFrame:`) keeps NSTextField's 4pt padding.
    #[cfg(target_os = "macos")]
    let label = Label::label_with_string(mtm, "");
    #[cfg(not(target_os = "macos"))]
    let label = Label::new(mtm);
    label.set_line_limit(line_limit.map_or(0, NonZero::get));
    let host_view: &PlatformView = &host;
    host.add_subview(as_view(&label));
    host.set_layout_handler({
        let label = label.clone();
        move |view| {
            let bounds = cocoa_ui::view::bounds(view);
            // A `labelWithString:` field keeps its text's alignment rect
            // inside the frame (`-[NSView alignmentRectInsets]`); expand
            // the frame by those insets so the ink lands exactly on the
            // leaf's bounds. `UILabel` draws at the frame's origin and
            // wraps at `preferredMaxLayoutWidth`, so it takes the bounds
            // unchanged.
            #[cfg(target_os = "macos")]
            let frame = {
                let insets = label.alignment_rect_insets();
                cocoa_ui::geometry::Rect::new(
                    bounds.origin.x - insets.left,
                    bounds.origin.y - insets.top,
                    bounds.size.width + insets.left + insets.right,
                    bounds.size.height + insets.top + insets.bottom,
                )
            };
            #[cfg(not(target_os = "macos"))]
            let frame = bounds;
            cocoa_ui::view::set_frame(as_view(&label), frame);
        }
    });

    let state = Rc::new(RefCell::new(TextState {
        mtm,
        env: ctx.env().clone(),
        label: label.clone(),
        default_foreground: WorkingColor::BLACK,
        chunks: Vec::new(),
        signal_guards: Vec::new(),
        default_guard: None,
    }));

    // The theme default backs every chunk without its own foreground,
    // exactly as the Swift renderer's `defaultForeground` slot does.
    let default_foreground = Foreground.resolve(ctx.env());
    let initial_foreground = default_foreground.snapshot();
    let foreground_guard = default_foreground.watch({
        let state = Rc::clone(&state);
        move |ctx| {
            let duration = cross_dissolve_duration(ctx.metadata());
            let mut state = state.borrow_mut();
            state.default_foreground = ctx.into_value();
            rebuild(&state, duration);
        }
    });
    {
        let mut state = state.borrow_mut();
        state.default_foreground = initial_foreground;
        state.default_guard = Some(Box::new(foreground_guard));
    }

    apply_styled(&state, styled);
    rebuild(&state.borrow(), None);
    apply_alignment(&label, alignment);

    let mut leaf = NativeLeaf::new(
        host_view,
        TextSubView {
            label: label.clone(),
        },
    );
    leaf.keep(Rc::clone(&state));
    (leaf, state, label)
}

/// Installs the `text` handler on the dispatcher: `Native<TextConfig>` maps
/// to a kit label with per-chunk signal watches and baseline measurement,
/// and `Native<Str>` (a bare string used as a view) maps to the same leaf
/// with static content.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<TextConfig>(|config, ctx| {
        let (mut leaf, state, label) = label_leaf(
            ctx,
            config.line_limit,
            &config.content.snapshot(),
            config.paragraph_alignment.snapshot(),
        );

        // Content changes re-resolve every chunk signal; paragraph
        // alignment pushes straight to the label inside the leaf.
        leaf.watch(&config.content, move |ctx| {
            let duration = cross_dissolve_duration(ctx.metadata());
            apply_styled(&state, ctx.value());
            rebuild(&state.borrow(), duration);
        });
        leaf.watch(&config.paragraph_alignment, move |ctx| {
            apply_alignment(&label, *ctx.value());
        });
        leaf
    });

    dispatcher.register_native::<Str>(|text, ctx| {
        let (leaf, ..) = label_leaf(
            ctx,
            None,
            &StyledStr::from(text),
            HorizontalAlignment::Leading,
        );
        leaf
    });
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;

    #[test]
    fn family_candidates_splits_trims_and_drops_empties() {
        let candidates: Vec<&str> = family_candidates("Menlo , Monaco ,,  Courier New ").collect();
        assert_eq!(candidates, ["Menlo", "Monaco", "Courier New"]);
        assert!(family_candidates("").next().is_none());
        assert!(family_candidates(" ,  ").next().is_none());
        assert_eq!(family_candidates("system").collect::<Vec<_>>(), ["system"]);
    }

    #[test]
    fn platform_weight_maps_every_weight() {
        use cocoa_ui::font::weight;
        let table: [(FontWeight, f64); 9] = [
            (FontWeight::Thin, weight::THIN),
            (FontWeight::UltraLight, weight::ULTRA_LIGHT),
            (FontWeight::Light, weight::LIGHT),
            (FontWeight::Normal, weight::REGULAR),
            (FontWeight::Medium, weight::MEDIUM),
            (FontWeight::SemiBold, weight::SEMI_BOLD),
            (FontWeight::Bold, weight::BOLD),
            (FontWeight::UltraBold, weight::HEAVY),
            (FontWeight::Black, weight::BLACK),
        ];
        for (weight, expected) in table {
            assert_eq!(platform_weight(weight).to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn cross_dissolve_duration_follows_animation_table() {
        assert_eq!(cross_dissolve_duration(&Metadata::new()), None);
        let table: [(Animation, f64); 3] = [
            (Animation::Default, 0.25),
            (
                Animation::Bezier {
                    duration: Duration::from_millis(300),
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 1.0,
                },
                0.3,
            ),
            (
                Animation::Spring {
                    stiffness: 170.0,
                    damping: 15.0,
                },
                0.15,
            ),
        ];
        for (animation, expected) in table {
            assert_eq!(
                cross_dissolve_duration(&Metadata::new().with(animation)).map(f64::to_bits),
                Some(expected.to_bits())
            );
        }
    }
}

#[cfg(test)]
mod wrap_tests {
    use cocoa_ui::text::WrapWidth;

    use super::wrap_for;

    #[test]
    fn the_width_proposal_picks_the_wrap_mode() {
        assert_eq!(wrap_for(None), WrapWidth::Free);
        assert_eq!(wrap_for(Some(120.0)), WrapWidth::Fixed(120.0));
        assert_eq!(wrap_for(Some(0.0)), WrapWidth::Unbreakable);
        assert_eq!(wrap_for(Some(-1.0)), WrapWidth::Unbreakable);
        // NaN is not a positive bound: unbreakable, like zero.
        assert_eq!(wrap_for(Some(f32::NAN)), WrapWidth::Unbreakable);
    }
}
