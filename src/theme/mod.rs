//! Theme tokens, settings, and environment installation helpers.

pub use waterui_controls::theme::*;

#[cfg(test)]
mod tests {
    use nami::{Binding, Signal, SignalExt};
    use waterui_core::{Environment, plugin::Plugin, resolve::Resolvable};
    use waterui_graphics::color::{Color, Working, WorkingColor};

    use super::*;

    #[test]
    fn color_modifiers_accept_tokens_palette_structs_and_signals() {
        use crate::ViewExt as _;
        use crate::background::Glass;
        use crate::border::Border;
        use crate::color::{Blue, Oklch, P3, Red, Srgb};
        use crate::component::button;
        use crate::component::list::row;
        use crate::shape::{Rectangle, ShapeExt as _};
        use crate::text::text;
        use crate::theme::color::{Accent, SurfaceVariant};
        use crate::widget::avatar;
        use waterui_icon::{Icon, IconGlyph};

        let selected = Binding::new(false);
        let signal = selected.select(Color::from(Accent), Color::from(SurfaceVariant));
        let binding_color = Binding::new(Color::from(Srgb::new(1.0, 0.0, 0.0)));
        let computed_color = binding_color.computed();

        // Palette structs and theme tokens work directly across modifiers.
        let _ = text("a").foreground(Accent);
        let _ = text("b").foreground(Red);
        let _ = text("c").foreground(Srgb::new(0.0, 0.0, 1.0));
        let _ = text("d").foreground(P3::new(0.2, 0.4, 0.8));
        let _ = text("e").foreground(Oklch::new(0.6, 0.15, 250.0));
        let _ = text("f").foreground(signal.clone());
        let _ = text("g").tint(Accent);
        let _ = text("h").tint(signal.clone());
        let _ = text("i").border(Blue, 1.0);
        let _ = text("j").border(signal.clone(), 1.0);
        let _ = Border::new(signal.clone(), 2.0);
        let _ = text("k").background(signal.clone());
        let _ = text("l").background(binding_color);
        let _ = text("m").background(computed_color);
        let _ = Glass::regular().tint(signal.clone());
        let _ = Rectangle.fill(signal.clone());
        let _ = Rectangle.fill(Red);
        let _ = button("Inbox").action(|| {}).badge(3_u32).color(Accent);
        let _ = button("Inbox")
            .action(|| {})
            .badge(3_u32)
            .color(Srgb::new(1.0, 0.0, 0.0));
        let _ = button("Inbox")
            .action(|| {})
            .badge(3_u32)
            .color(signal.clone());
        let _ = avatar("Ada").ring(Accent, 2.0).ring(signal.clone(), 2.0);
        let _ = row("Key", "Val")
            .value_color(Accent)
            .value_color(signal.clone());
        let _ = Icon::from_glyph(IconGlyph::new('\u{e88a}', "MaterialIcons-Regular"))
            .foreground(Accent)
            .foreground(signal);

        // ForegroundOverride resolves a reactive signal in place.
        let active = Binding::new(false);
        let light = WorkingColor::new([0.1, 0.2, 0.3, 1.0]);
        let dark = WorkingColor::new([0.9, 0.8, 0.7, 1.0]);
        let mut env = Environment::new();
        ForegroundOverride::new(
            active.select(Color::new(Working(dark)), Color::new(Working(light))),
        )
        .install(&mut env);
        let fg = color::Foreground.resolve(&env);
        assert_eq!(fg.snapshot().components, light.components);
        active.set(true);
        assert_eq!(fg.snapshot().components, dark.components);
    }
}
