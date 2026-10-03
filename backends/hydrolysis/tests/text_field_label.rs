//! Regression coverage for water-rs/hydrolysis-m3#85: an empty, unfocused
//! Material 3 filled text field centres its label vertically in the 56 dp
//! container, and the label floats to the top only while the field is
//! focused or carries content. A field that hides its label view draws the
//! prompt in the label slot under the same resting/floating geometry, and a
//! single-line field with no inside label at all centres its input text.

use core::time::Duration;

use waterui::View;
use waterui::ViewExt as _;
use waterui::{Binding, Str};
use waterui_controls::field;
use waterui_testing::{OffscreenApp, Role};

/// The issue's reproduction: a filled field whose label is hidden, carrying
/// only a prompt.
fn hidden_label_prompt_field() -> impl View {
    let value = Binding::container(Str::from(""));
    field("Search chats", &value)
        .hide_label()
        .prompt("Search chats")
        .size(280.0, 56.0)
}

/// A filled field with a visible label: the resting position the prompt
/// case must match.
fn labeled_field() -> impl View {
    let value = Binding::container(Str::from(""));
    field("Search chats", &value).size(280.0, 56.0)
}

/// A filled field with neither a visible label nor a prompt: M3 centres the
/// input text vertically in the container.
fn label_less_field() -> impl View {
    let value = Binding::container(Str::from(""));
    field("", &value).hide_label().size(280.0, 56.0)
}

/// Contiguous bands of text-ink rows inside the field's container, in
/// field-relative y coordinates. Rows thinner than a glyph stroke (the 1 px
/// caret) and rows wider than half the container (the bottom active
/// indicator) are not text and are dropped.
fn text_ink_runs(app: &mut OffscreenApp) -> (Vec<(f64, f64)>, f64) {
    let bounds = app.query().role(Role::TEXT_INPUT).single().bounds();
    let snapshot = app.snapshot();
    let x0 = bounds.x() as usize;
    let y0 = bounds.y() as usize;
    let x1 = (bounds.x() + bounds.width()) as usize;
    let y1 = (bounds.y() + bounds.height()) as usize;
    let pixel = |x: usize, y: usize| {
        let i = (y * snapshot.width as usize + x) * 4;
        &snapshot.rgba8[i..i + 4]
    };
    let background = pixel(x1 - 8, (y0 + y1) / 2);
    let mut runs: Vec<(f64, f64)> = Vec::new();
    let mut run_start = None;
    for y in y0..(y1 - 4) {
        let mut ink = 0usize;
        for x in (x0 + 6)..(x1 - 6) {
            let difference: i32 = pixel(x, y)
                .iter()
                .zip(background)
                .map(|(a, b)| (*a as i32 - *b as i32).abs())
                .sum();
            if difference > 24 {
                ink += 1;
            }
        }
        let is_text = ink >= 4 && ink < (x1 - x0) / 2;
        match (is_text, run_start) {
            (true, None) => run_start = Some(y - y0),
            (false, Some(start)) => {
                runs.push((start as f64, (y - y0) as f64));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        runs.push((start as f64, (y1 - 4 - y0) as f64));
    }
    // Antialiased stroke edges can drop under the ink threshold for a row or
    // two inside one line of text; merge runs separated by that much.
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for run in runs {
        if let Some(last) = merged.last_mut()
            && run.0 - last.1 <= 2.0
        {
            last.1 = run.1;
            continue;
        }
        merged.push(run);
    }
    (merged, (y1 - y0) as f64)
}

fn assert_run_centre_near(
    runs: &[(f64, f64)],
    index: usize,
    centre: f64,
    epsilon: f64,
    context: &str,
) {
    let run = runs
        .get(index)
        .unwrap_or_else(|| panic!("{context}: missing ink run {index} in {runs:?}"));
    let measured = (run.0 + run.1) * 0.5;
    assert!(
        (measured - centre).abs() <= epsilon,
        "{context}: ink run {run:?} centred at {measured}, expected within {epsilon} px of {centre}"
    );
}

#[waterui::test(hidden_label_prompt_field, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (320, 120))]
fn empty_unfocused_prompt_label_is_vertically_centered(app: &mut OffscreenApp) {
    let (runs, height) = text_ink_runs(app);
    assert!(
        runs.len() == 1,
        "an empty unfocused field should draw only the label: {runs:?}"
    );
    // M3 centres the resting label's line box in the 56 dp container; the
    // face's ink rides about a pixel above that box's centre.
    assert_run_centre_near(&runs, 0, height * 0.5, 1.5, "resting prompt label");
}

#[waterui::test(hidden_label_prompt_field, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (320, 120))]
fn focused_prompt_label_floats_to_the_top(app: &mut OffscreenApp) {
    let field = app.query().role(Role::TEXT_INPUT).single();
    field.focus(app);
    app.pump_for(Duration::from_millis(600));
    let (runs, _height) = text_ink_runs(app);
    let floating_label = runs
        .first()
        .unwrap_or_else(|| panic!("focused field should still draw the label: {runs:?}"));
    assert!(
        floating_label.0 <= 8.0 && floating_label.1 <= 20.0,
        "the floating label should sit at the top of the container: {runs:?}"
    );
    assert!(
        runs.iter().all(|&(y0, _)| !(20.0..36.0).contains(&y0)),
        "nothing should remain at the resting position once the label floats: {runs:?}"
    );
}

#[waterui::test(hidden_label_prompt_field, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (320, 120))]
fn filled_prompt_label_floats_to_the_top(app: &mut OffscreenApp) {
    let field = app.query().role(Role::TEXT_INPUT).single();
    field.set_text(app, "lorem ipsum");
    app.pump_for(Duration::from_millis(600));
    app.clear_ui_focus();
    app.pump_for(Duration::from_millis(600));
    let (runs, _height) = text_ink_runs(app);
    assert!(
        runs.len() >= 2,
        "a filled field draws the floated label and the input text: {runs:?}"
    );
    assert!(
        runs[0].0 <= 8.0 && runs[0].1 <= 20.0,
        "the floated label should sit at the top: {runs:?}"
    );
    let input = runs[1];
    assert!(
        input.0 >= 24.0,
        "the input text should sit under the floated label, not centred: {runs:?}"
    );
}

#[waterui::test(labeled_field, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (320, 120))]
fn empty_unfocused_label_is_vertically_centered(app: &mut OffscreenApp) {
    let (runs, height) = text_ink_runs(app);
    assert!(
        runs.len() == 1,
        "an empty unfocused field should draw only the label: {runs:?}"
    );
    assert_run_centre_near(&runs, 0, height * 0.5, 2.5, "resting label");
}

#[waterui::test(labeled_field, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (320, 120))]
fn focused_label_floats_to_the_top(app: &mut OffscreenApp) {
    let field = app.query().role(Role::TEXT_INPUT).single();
    field.focus(app);
    app.pump_for(Duration::from_millis(600));
    let (runs, _height) = text_ink_runs(app);
    let floating_label = runs
        .first()
        .unwrap_or_else(|| panic!("focused field should still draw the label: {runs:?}"));
    assert!(
        floating_label.0 <= 8.0 && floating_label.1 <= 20.0,
        "the floating label should sit at the top of the container: {runs:?}"
    );
}

#[waterui::test(label_less_field, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (320, 120))]
fn label_less_field_centres_its_input_text(app: &mut OffscreenApp) {
    let field = app.query().role(Role::TEXT_INPUT).single();
    field.set_text(app, "lorem ipsum");
    app.pump_for(Duration::from_millis(600));
    app.clear_ui_focus();
    app.pump_for(Duration::from_millis(600));
    let (runs, height) = text_ink_runs(app);
    assert!(
        runs.len() == 1,
        "a label-less field draws only the input text: {runs:?}"
    );
    assert_run_centre_near(&runs, 0, height * 0.5, 2.5, "label-less input text");
}

/// Snapshot acceptance: the prompt-as-label field in its three states under
/// both schemes. Ignored by default — it writes PNGs for direct image
/// review.
#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(hidden_label_prompt_field, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (320, 120))]
fn prompt_label_states_light(app: &mut OffscreenApp) {
    capture_states(app, "light");
}

/// Snapshot acceptance, dark scheme.
#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(
    hidden_label_prompt_field,
    theme = hydrolysis_m3::Material3::with_colors(hydrolysis_m3::MaterialColorScheme::baseline_dark()),
    offscreen,
    viewport = (320, 120)
)]
fn prompt_label_states_dark(app: &mut OffscreenApp) {
    capture_states(app, "dark");
}

fn capture_states(app: &mut OffscreenApp, scheme: &str) {
    let _ = app.capture_snapshot("text-field", "label-placement", format!("{scheme}-empty"));
    let field = app.query().role(Role::TEXT_INPUT).single();
    field.focus(app);
    app.pump_for(Duration::from_millis(600));
    let _ = app.capture_snapshot("text-field", "label-placement", format!("{scheme}-focused"));
    let field = app.query().role(Role::TEXT_INPUT).single();
    field.set_text(app, "lorem ipsum");
    app.pump_for(Duration::from_millis(600));
    app.clear_ui_focus();
    app.pump_for(Duration::from_millis(600));
    let _ = app.capture_snapshot("text-field", "label-placement", format!("{scheme}-filled"));
}
