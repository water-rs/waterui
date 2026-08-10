//! GTK4 Text component implementation.

use gtk4::prelude::*;
use gtk4::{Label, Widget};
use nami::Signal;
use std::fmt::Write;
use waterui_core::layout::HorizontalAlignment;
use waterui_core::{Environment, Native};
use waterui_text::TextConfig;
use waterui_text::font::{FontWeight, ResolvedFont};
use waterui_text::styled::{Style, StyledStr};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{resolved_color_to_hex, store_watcher_guards};

impl GtkComponent for Native<TextConfig> {
    /// Renders a `WaterUI` Text component as a GTK4 Label.
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        let label = Label::new(None);
        apply_styled_content(
            &label,
            config.content.get(),
            config.paragraph_alignment.get(),
            env,
        );

        // Match native behavior: read-only text should not be selection-active by default.
        label.set_selectable(false);
        // TextConfig is used for content text which may need wrapping
        label.set_wrap(true);
        label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);

        // Set up reactive updates
        let guard = config.content.watch({
            let label = label.clone();
            let env = env.clone();
            let paragraph_alignment = config.paragraph_alignment.clone();
            move |ctx| {
                let content = ctx.into_value();
                let label = label.clone();
                let env = env.clone();
                let alignment = paragraph_alignment.get();
                // Schedule update on GTK main thread
                glib::idle_add_local_once(move || {
                    apply_styled_content(&label, content, alignment, &env);
                });
            }
        });

        let alignment_guard = config.paragraph_alignment.watch({
            let label = label.clone();
            move |ctx| {
                let alignment = ctx.into_value();
                let label = label.clone();
                glib::idle_add_local_once(move || {
                    apply_paragraph_alignment(&label, alignment);
                });
            }
        });

        // Store the watcher guard to keep it alive
        store_watcher_guards(&label, vec![guard, alignment_guard]);

        label.upcast()
    }
}

fn apply_styled_content(
    label: &Label,
    content: StyledStr,
    alignment: HorizontalAlignment,
    env: &Environment,
) {
    let markup = styled_to_markup(content, env);
    label.set_markup(&markup);
    apply_paragraph_alignment(label, alignment);
}

fn apply_paragraph_alignment(label: &Label, alignment: HorizontalAlignment) {
    if alignment == HorizontalAlignment::Leading {
        label.set_justify(gtk4::Justification::Left);
        label.set_xalign(0.0);
    } else if alignment == HorizontalAlignment::Trailing {
        label.set_justify(gtk4::Justification::Right);
        label.set_xalign(1.0);
    } else {
        label.set_justify(gtk4::Justification::Center);
        label.set_xalign(0.5);
    }
}

fn styled_to_markup(content: StyledStr, env: &Environment) -> String {
    let mut markup = String::new();

    for (text, style) in content.into_chunks() {
        let escaped_text = escape_markup_text(text.as_str());
        if escaped_text.is_empty() {
            continue;
        }

        let attrs = style_to_markup_attrs(&style, env);
        if attrs.is_empty() {
            markup.push_str(&escaped_text);
            continue;
        }

        markup.push_str("<span");
        markup.push_str(&attrs);
        markup.push('>');
        markup.push_str(&escaped_text);
        markup.push_str("</span>");
    }

    markup
}

fn style_to_markup_attrs(style: &Style, env: &Environment) -> String {
    let mut attrs = String::new();
    let resolved_font: ResolvedFont = style.font.resolve(env).get();
    let font_size = resolved_font.size.max(1.0);
    let _ = write!(attrs, " size=\"{font_size:.2}pt\"");
    let _ = write!(
        attrs,
        " weight=\"{}\"",
        font_weight_to_pango_value(resolved_font.weight)
    );

    if let Some(family) = resolved_font.family {
        if !family.is_empty() {
            let family = escape_markup_attr(family.as_str());
            let _ = write!(attrs, " font_family=\"{family}\"");
        }
    }

    if style.italic {
        attrs.push_str(" style=\"italic\"");
    }

    if style.underline {
        attrs.push_str(" underline=\"single\"");
    }

    if style.strikethrough {
        attrs.push_str(" strikethrough=\"true\"");
    }

    if let Some(foreground) = &style.foreground {
        let resolved = foreground.resolve(env).get();
        let color = resolved_color_to_hex(resolved);
        let _ = write!(attrs, " foreground=\"{color}\"");
    }

    if let Some(background) = &style.background {
        let resolved = background.resolve(env).get();
        let color = resolved_color_to_hex(resolved);
        let _ = write!(attrs, " background=\"{color}\"");
    }

    attrs
}

fn font_weight_to_pango_value(weight: FontWeight) -> u16 {
    match weight {
        FontWeight::Thin => 100,
        FontWeight::UltraLight => 200,
        FontWeight::Light => 300,
        FontWeight::Normal => 400,
        FontWeight::Medium => 500,
        FontWeight::SemiBold => 600,
        FontWeight::Bold => 700,
        FontWeight::UltraBold => 800,
        FontWeight::Black => 900,
    }
}

fn escape_markup_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn escape_markup_attr(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}
