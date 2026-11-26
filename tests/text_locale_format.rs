//! `text!` interpolations must honor format specs on the wrapped arguments.
//!
//! `text!` shadows every captured identifier with a
//! `locale::LocalizedArgument` before `format!` runs. That adapter used to emit
//! numeric values through `write_str`, dropping width, fill and alignment, so
//! `text!("Record #{id:06}")` rendered `Record #0`; and its derived `Debug`
//! printed the wrapper itself, so `text!("{selection:?}")` rendered
//! `LocalizedArgument { value: Apple, locale: Locale(en-US) }`.

use core::num::{NonZeroU32, Saturating, Wrapping};

use waterui::locale::{Locale, locales};
use waterui::prelude::*;
use waterui::reactive::constant;
use waterui_testing::{Role, UiBuilder};

#[waterui::test()]
fn text_macro_pads_numeric_arguments(ui: UiBuilder) {
    let mut app = ui.mount(|| waterui::text!("Record #{id:06}", id = 0));
    app.query()
        .role(Role::LABEL)
        .label("Record #000000")
        .assert_exists();
}

#[waterui::test()]
fn text_macro_debug_formats_the_wrapped_value(ui: UiBuilder) {
    #[derive(Debug, Clone, PartialEq)]
    enum Fruit {
        Apple,
    }

    let selection: Binding<Fruit> = binding(Fruit::Apple);
    let mut app = ui.mount(move || waterui::text!("Selected: {selection:?}"));
    app.query()
        .role(Role::LABEL)
        .label("Selected: Apple")
        .assert_exists();
}

#[waterui::test()]
fn text_macro_localizes_nonzero_wrapping_and_saturating_numbers_per_locale(ui: UiBuilder) {
    let ar_eg: Locale = "ar-EG".parse().unwrap();
    let nz = NonZeroU32::new(1234).unwrap();
    let wr = Wrapping(1234_u32);
    let sat = Saturating(1234_u32);

    let cases = [
        ("ar-EG", "[ar-EG] ١٬٢٣٤ items"),
        ("de", "[de] 1.234 items"),
        ("en-US", "[en-US] 1,234 items"),
        ("zh-CN", "[zh-CN] 1,234 items"),
        ("ja", "[ja] 1,234 items"),
        ("ko", "[ko] 1,234 items"),
    ];

    let mut app = ui.mount(move || {
        let ar_eg = ar_eg.clone();
        vstack((
            vstack((
                waterui::text!("[ar-EG] {n} items", n = 1234_u32).with(ar_eg.clone()),
                waterui::text!("[ar-EG] {n} items", n = constant(nz)).with(ar_eg.clone()),
                waterui::text!("[ar-EG] {n} items", n = constant(wr)).with(ar_eg.clone()),
                waterui::text!("[ar-EG] {n} items", n = constant(sat)).with(ar_eg),
            )),
            vstack((
                waterui::text!("[de] {n} items", n = 1234_u32).with(locales::DE),
                waterui::text!("[de] {n} items", n = constant(nz)).with(locales::DE),
                waterui::text!("[de] {n} items", n = constant(wr)).with(locales::DE),
                waterui::text!("[de] {n} items", n = constant(sat)).with(locales::DE),
            )),
            vstack((
                waterui::text!("[en-US] {n} items", n = 1234_u32).with(locales::EN_US),
                waterui::text!("[en-US] {n} items", n = constant(nz)).with(locales::EN_US),
                waterui::text!("[en-US] {n} items", n = constant(wr)).with(locales::EN_US),
                waterui::text!("[en-US] {n} items", n = constant(sat)).with(locales::EN_US),
            )),
            vstack((
                waterui::text!("[zh-CN] {n} items", n = 1234_u32).with(locales::ZH_CN),
                waterui::text!("[zh-CN] {n} items", n = constant(nz)).with(locales::ZH_CN),
                waterui::text!("[zh-CN] {n} items", n = constant(wr)).with(locales::ZH_CN),
                waterui::text!("[zh-CN] {n} items", n = constant(sat)).with(locales::ZH_CN),
            )),
            vstack((
                waterui::text!("[ja] {n} items", n = 1234_u32).with(locales::JA),
                waterui::text!("[ja] {n} items", n = constant(nz)).with(locales::JA),
                waterui::text!("[ja] {n} items", n = constant(wr)).with(locales::JA),
                waterui::text!("[ja] {n} items", n = constant(sat)).with(locales::JA),
            )),
            vstack((
                waterui::text!("[ko] {n} items", n = 1234_u32).with(locales::KO),
                waterui::text!("[ko] {n} items", n = constant(nz)).with(locales::KO),
                waterui::text!("[ko] {n} items", n = constant(wr)).with(locales::KO),
                waterui::text!("[ko] {n} items", n = constant(sat)).with(locales::KO),
            )),
        ))
    });

    for (tag, expected) in cases {
        let matches = app.query().role(Role::LABEL).label(expected).all();
        assert_eq!(
            matches.len(),
            4,
            "expected 4 localized labels `{expected}` for `{tag}`"
        );
    }
}
