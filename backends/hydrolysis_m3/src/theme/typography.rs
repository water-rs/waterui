use waterui::reactive::{Computed, Signal};
use waterui::text::font::{Font, FontWeight, ResolvedFont};
use waterui::theme::FontSettings;
use waterui_core::{Environment, resolve::Resolvable};

const MATERIAL_TYPEFACE: &str = "Roboto";

const fn font(size: f32, weight: FontWeight) -> ResolvedFont {
    ResolvedFont::with_static_family(size, weight, MATERIAL_TYPEFACE)
}

pub(crate) fn settings() -> FontSettings {
    FontSettings::new()
        .body(font(16.0, FontWeight::Normal))
        .title(font(22.0, FontWeight::Normal))
        .headline(font(24.0, FontWeight::Normal))
        .subheadline(font(16.0, FontWeight::Medium))
        .caption(font(12.0, FontWeight::Normal))
        .footnote(font(11.0, FontWeight::Medium))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LabelLarge;

impl Resolvable for LabelLarge {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(14.0, FontWeight::Medium))
    }
}

pub(crate) fn label_large() -> Font {
    Font::new(LabelLarge)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LabelSmall;

impl Resolvable for LabelSmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(11.0, FontWeight::Medium))
    }
}

pub(crate) fn label_small() -> Font {
    Font::new(LabelSmall)
}

#[cfg(test)]
mod tests {
    use super::{MATERIAL_TYPEFACE, label_large, label_small, settings};
    use waterui::Plugin as _;
    use waterui::env::Environment;
    use waterui::reactive::Signal as _;
    use waterui::text::font::{Body, Caption, FontWeight, Footnote, Headline, Subheadline, Title};
    use waterui_core::resolve::Resolvable as _;

    fn assert_material_font(
        font: waterui::text::font::ResolvedFont,
        expected_size: f32,
        expected_weight: FontWeight,
    ) {
        assert_eq!(font.size, expected_size);
        assert_eq!(font.weight, expected_weight);
        assert_eq!(font.family.as_deref(), Some(MATERIAL_TYPEFACE));
    }

    #[test]
    fn font_slots_match_material_web_v0_192_type_scale() {
        let mut env = Environment::new();
        waterui::theme::Theme::new()
            .fonts(settings())
            .install(&mut env);

        assert_material_font(Body.resolve(&env).get(), 16.0, FontWeight::Normal);
        assert_material_font(Title.resolve(&env).get(), 22.0, FontWeight::Normal);
        assert_material_font(Headline.resolve(&env).get(), 24.0, FontWeight::Normal);
        assert_material_font(Subheadline.resolve(&env).get(), 16.0, FontWeight::Medium);
        assert_material_font(Caption.resolve(&env).get(), 12.0, FontWeight::Normal);
        assert_material_font(Footnote.resolve(&env).get(), 11.0, FontWeight::Medium);
    }

    #[test]
    fn label_large_matches_material_web_v0_192_label_large() {
        let env = Environment::new();

        assert_material_font(label_large().resolve(&env).get(), 14.0, FontWeight::Medium);
    }

    #[test]
    fn label_small_matches_material_web_v0_192_label_small() {
        let env = Environment::new();

        assert_material_font(label_small().resolve(&env).get(), 11.0, FontWeight::Medium);
    }
}
