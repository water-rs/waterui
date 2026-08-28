//! Typography and bidirectional-layout showcase for WaterUI.

use waterui::app::App;
use waterui::env::with;
use waterui::locale::{Locale, locales};
use waterui::prelude::theme_color::{MutedForeground, SurfaceVariant};
use waterui::prelude::*;
use waterui::preview;

fn type_scale() -> impl View {
    vstack((
        text("Display / 展示 / عرض").headline(),
        text("Title / 标题 / כותרת").title(),
        text("Headline / 小标题 / عنوان").sub_headline(),
        text("Body — WaterUI shapes العربية、中文、日本語、한국어 in one paragraph.").body(),
        text("Footnote — Mixed scripts keep punctuation and numbers stable: الإصدار 2.0 (2026).")
            .footnote(),
        text("Caption — 字体 fallback follows locale and script.")
            .caption()
            .foreground(MutedForeground),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(8.0)
}

fn cjk_specimens() -> impl View {
    vstack((
        text("CJK locale-aware glyph selection").sub_headline(),
        with(text("简体中文：骨、直、门、关").body(), locales::ZH_CN),
        with(text("繁體中文：骨、直、門、關").body(), locales::ZH_TW),
        with(text("日本語：骨、直、門、関").body(), locales::JA),
        with(text("한국어: 한글과 漢字").body(), locales::KO),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(6.0)
}

fn reading_order_panel(title: &'static str, body: &'static str, arrow: &'static str) -> impl View {
    let input = Binding::container(Str::from(""));
    vstack((
        text(title).sub_headline(),
        hstack((
            text("①").headline(),
            vstack((
                text(body).body(),
                text("WaterUI 2.0 · 2026")
                    .caption()
                    .foreground(MutedForeground),
            ))
            .alignment(HorizontalAlignment::Leading),
            spacer(),
            text(arrow).headline(),
        ))
        .spacing(12.0),
        TextField::new("Name / الاسم / שם", &input).prompt("Type here / اكتب هنا / הקלידו כאן"),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(12.0)
    .padding_with(EdgeInsets::all(16.0))
    .background(SurfaceVariant)
}

fn localized_panel(
    locale: Locale,
    direction: LayoutDirection,
    title: &'static str,
    body: &'static str,
    arrow: &'static str,
) -> impl View {
    with(
        with(reading_order_panel(title, body, arrow), locale),
        direction,
    )
}

#[preview]
pub fn typography_and_rtl() -> impl View {
    scroll(
        vstack((
            text("Typography & Bidirectional Layout").headline(),
            text(
                "Semantic type styles, CJK fallback, mixed-script shaping, and logical RTL layout.",
            )
            .body()
            .foreground(MutedForeground),
            Divider,
            type_scale(),
            Divider,
            cjk_specimens(),
            Divider,
            text("Logical reading order").title(),
            localized_panel(
                locales::EN,
                LayoutDirection::LeftToRight,
                "Left-to-right",
                "A logical HStack starts here",
                "→",
            ),
            localized_panel(
                locales::AR,
                LayoutDirection::RightToLeft,
                "من اليمين إلى اليسار",
                "يبدأ الصف المنطقي من هنا",
                "←",
            ),
            localized_panel(
                "he".parse().expect("Hebrew locale must parse"),
                LayoutDirection::RightToLeft,
                "מימין לשמאל",
                "השורה הלוגית מתחילה כאן",
                "←",
            ),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(16.0)
        .padding_with(EdgeInsets::all(20.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(typography_and_rtl, env)
}

#[cfg(test)]
mod tests {
    use hydrolysis_m3::install;

    use super::typography_and_rtl;

    #[waterui::test(
        typography_and_rtl,
        theme = install,
        viewport = (720, 1280),
        offscreen
    )]
    fn typography_and_rtl_renders_in_hydrolysis(app: &mut waterui_testing::OffscreenApp) {
        app.snapshot()
            .save_png("/tmp/waterui_typography_rtl.png")
            .expect("typography and RTL snapshot must be writable");
    }
}
