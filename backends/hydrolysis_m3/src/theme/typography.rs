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

#[derive(Debug, Clone, Copy)]
pub(crate) struct BodyMedium;

impl Resolvable for BodyMedium {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(14.0, FontWeight::Normal))
    }
}

pub(crate) fn body_medium() -> Font {
    Font::new(BodyMedium)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BodySmall;

impl Resolvable for BodySmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(12.0, FontWeight::Normal))
    }
}

pub(crate) fn body_small() -> Font {
    Font::new(BodySmall)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TitleSmall;

impl Resolvable for TitleSmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(14.0, FontWeight::Medium))
    }
}

pub(crate) fn title_small() -> Font {
    Font::new(TitleSmall)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HeadlineSmall;

impl Resolvable for HeadlineSmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(24.0, FontWeight::Normal))
    }
}

pub(crate) fn headline_small() -> Font {
    Font::new(HeadlineSmall)
}

#[cfg(test)]
mod tests {
    use super::{
        MATERIAL_TYPEFACE, body_medium, body_small, headline_small, label_large, label_small,
        settings, title_small,
    };
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

    #[test]
    fn body_medium_matches_material_web_v0_192_body_medium() {
        let env = Environment::new();

        assert_material_font(body_medium().resolve(&env).get(), 14.0, FontWeight::Normal);
    }

    #[test]
    fn body_small_matches_material_web_v0_192_body_small() {
        let env = Environment::new();

        assert_material_font(body_small().resolve(&env).get(), 12.0, FontWeight::Normal);
    }

    #[test]
    fn title_small_matches_material_web_v0_192_title_small() {
        let env = Environment::new();

        assert_material_font(title_small().resolve(&env).get(), 14.0, FontWeight::Medium);
    }

    #[test]
    fn headline_small_matches_material_web_v0_192_headline_small() {
        let env = Environment::new();

        assert_material_font(
            headline_small().resolve(&env).get(),
            24.0,
            FontWeight::Normal,
        );
    }
}
