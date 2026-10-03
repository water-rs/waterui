//! `Native<Text>` and `Native<Str>` — the reading half of the port.
//!
//! Both ride `android.widget.TextView`: the plain object is the view; the
//! content's plain string is its text; the resolved `Body` font is its
//! `textSize`/`typeface`; `Foreground` is its text color; the paragraph
//! alignment is its gravity plus text alignment. Per-chunk styling (fonts,
//! colors, links, spans) is what `android.text.Spanned` is for and lands
//! with the text port that needs it — the skeleton reads every chunk as its
//! characters, and `line_limit` caps the laid-out lines.

use jni::objects::JString;
use waterui::Str;
use waterui::resolve::Resolvable;
use waterui::text::StyledStr;
use waterui::text::font::{Body, FontDesign, FontWeight, ResolvedFont};
use waterui::text::{Text, TextConfig};
use waterui::theme::color::Foreground;
use waterui_backend_core::Environment;
use waterui_core::layout::HorizontalAlignment;

use crate::contract::{NativeLeaf, PlatformView, RenderContext};
use crate::dispatch::Dispatcher;
use crate::jvm;
use crate::native_layout::ViewSubView;

/// Gravity's horizontal half — vertical centering is fixed by the frame the
/// layout gives the view, never by the label's gravity.
mod gravity {
    pub(crate) const START: i32 = 0x0080_0003;
    pub(crate) const CENTER_HORIZONTAL: i32 = 1;
    pub(crate) const END: i32 = 0x0080_0005;
    pub(crate) const CENTER_VERTICAL: i32 = 0x10;
}

/// `View.TEXT_ALIGNMENT_*` — the layout-direction-aware alignment.
mod text_alignment {
    pub(crate) const TEXT_START: i32 = 5;
    pub(crate) const CENTER: i32 = 4;
    pub(crate) const TEXT_END: i32 = 6;
}

/// Claims `Native<Text>` and `Native<Str>`.
pub(crate) fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Text>(|text, ctx| {
        render_text(ctx, text.resolve(&ctx.env()))
    });
    dispatcher.register_native::<Str>(|content, ctx| render_str(ctx, content));
}

/// A label, fresh off the host context.
fn new_label() -> PlatformView {
    jvm::with_env(|env| {
        jvm::globals()
            .bindings()
            .new_text_view(env)
            .expect("a TextView constructs against the host context")
    })
}

/// Pushes a string onto `label`.
fn set_label_text(label: &PlatformView, text: &StyledStr) {
    let plain = text.to_plain();
    jvm::with_env(|env| {
        let text = JString::from_str(env, plain.as_str());
        jvm::globals()
            .bindings()
            .set_text(env, label.as_ref(), &text)
            .expect("TextView.setText must not throw")
    });
}

/// `Native<Text>`: resolve the semantic text into its config — content,
/// alignment and line limit already flat signals — then render.
fn render_text(ctx: &RenderContext, config: TextConfig) -> NativeLeaf {
    let env = ctx.env();
    let label = new_label();
    let mut leaf = NativeLeaf::new(jvm::retain(&label), ViewSubView::new(label));

    // Content: the plain string is the view's text, watched per write.
    let view = jvm::retain(leaf.view());
    leaf.bind(&config.content, move |styled| set_label_text(&view, &styled));

    // Alignment: gravity places the laid-out text in the frame, text
    // alignment picks the edge. Both start/end so RTL follows the locale.
    let view = jvm::retain(leaf.view());
    leaf.bind(&config.paragraph_alignment, move |alignment| {
        let (gravity, alignment) = if alignment == HorizontalAlignment::Leading {
            (
                gravity::START | gravity::CENTER_VERTICAL,
                text_alignment::TEXT_START,
            )
        } else if alignment == HorizontalAlignment::Trailing {
            (
                gravity::END | gravity::CENTER_VERTICAL,
                text_alignment::TEXT_END,
            )
        } else {
            (
                gravity::CENTER_HORIZONTAL | gravity::CENTER_VERTICAL,
                text_alignment::CENTER,
            )
        };
        jvm::with_env(|env| {
            let bindings = jvm::globals().bindings();
            bindings
                .set_gravity(env, view.as_ref(), gravity)
                .and_then(|_| bindings.set_text_alignment(env, view.as_ref(), alignment))
                .expect("alignment setters must not throw")
        });
    });

    if let Some(limit) = config.line_limit {
        let lines = i32::try_from(limit.get()).unwrap_or(i32::MAX);
        jvm::with_env(|env| {
            jvm::globals()
                .bindings()
                .set_max_lines(env, leaf.view().as_ref(), lines)
                .expect("TextView.setMaxLines must not throw")
        });
    }

    apply_type(env, &mut leaf);
    leaf
}

/// `Native<Str>` — a bare string is a label with the resolved body font:
/// the same face as `text()` minus the config's own fields.
fn render_str(ctx: &RenderContext, content: Str) -> NativeLeaf {
    let env = ctx.env();
    let label = new_label();
    jvm::with_env(|env| {
        let text = JString::from_str(env, content.as_str());
        jvm::globals()
            .bindings()
            .set_text(env, label.as_ref(), &text)
            .expect("TextView.setText must not throw")
    });
    let mut leaf = NativeLeaf::new(jvm::retain(&label), ViewSubView::new(label));
    apply_type(env, &mut leaf);
    leaf
}

/// The font and color feed both renders share: `Body` resolves to
/// `textSize`/`typeface`, `Foreground` to `textColor`.
fn apply_type(env: &Environment, leaf: &mut NativeLeaf) {
    let view = jvm::retain(leaf.view());
    let font = Body.resolve(env);
    leaf.bind(&font, move |font| apply_font(&view, font));

    let view = jvm::retain(leaf.view());
    let color = Foreground.resolve(env);
    leaf.bind(&color, move |color| {
        jvm::with_env(|env| {
            jvm::globals()
                .bindings()
                .set_text_color(env, view.as_ref(), crate::theme::working_to_argb(color))
                .expect("TextView.setTextColor must not throw")
        });
    });
}

/// `ResolvedFont` → `textSize` in sp plus the closest platform typeface —
/// a skeleton's serif/mono distinction is `DEFAULT`/`DEFAULT_BOLD`/
/// `MONOSPACE` while custom families wait for a font loader.
fn apply_font(label: &PlatformView, font: ResolvedFont) {
    let face = match font.design {
        FontDesign::Monospaced => jvm::Face::Monospace,
        _ if matches!(
            font.weight,
            FontWeight::SemiBold | FontWeight::Bold | FontWeight::UltraBold | FontWeight::Black
        ) =>
        {
            jvm::Face::DefaultBold
        }
        _ => jvm::Face::Default,
    };
    jvm::with_env(|env| {
        let bindings = jvm::globals().bindings();
        let face = bindings
            .typeface(env, face)
            .expect("the built-in typefaces always load");
        bindings
            .set_text_size(env, label.as_ref(), font.size)
            .and_then(|_| bindings.set_typeface(env, label.as_ref(), &face))
            .expect("font setters must not throw")
    });
}
