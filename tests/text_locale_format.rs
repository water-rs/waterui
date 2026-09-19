//! `text!` interpolations must honor format specs on the wrapped arguments.
//!
//! `text!` shadows every captured identifier with a
//! `locale::LocalizedArgument` before `format!` runs. That adapter used to emit
//! numeric values through `write_str`, dropping width, fill and alignment, so
//! `text!("Record #{id:06}")` rendered `Record #0`; and its derived `Debug`
//! printed the wrapper itself, so `text!("{selection:?}")` rendered
//! `LocalizedArgument { value: Apple, locale: Locale(en-US) }`.

use hydrolysis_m3::install as install_m3;
use waterui::prelude::*;
use waterui_testing::{Role, UiBuilder};

#[waterui::test(theme = install_m3)]
fn text_macro_pads_numeric_arguments(ui: UiBuilder) {
    let mut app = ui.mount(|| waterui::text!("Record #{id:06}", id = 0));
    app.query()
        .role(Role::LABEL)
        .label("Record #000000")
        .assert_exists();
}

#[waterui::test(theme = install_m3)]
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
