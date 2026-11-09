//! A text's measured width is its content's extent: aligning the line inside
//! a wider box, as a right-to-left environment does, never inflates it.

use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::layout::{LayoutDirection, ProposalSize};

use super::*;

fn measure_row(env: &Environment, proposal: ProposalSize) -> f32 {
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let view = normalize_layout_view(AnyView::new(text("Row")), env);
    measure_view_dimensions_with_proposal(&view, proposal, &mut state, env, &theme)
        .size
        .width
}

#[test]
fn rtl_text_measures_its_natural_width_under_a_roomy_proposal() {
    let ltr = test_environment();
    let mut rtl = test_environment();
    rtl.insert(LayoutDirection::RightToLeft);
    let natural = measure_row(&ltr, ProposalSize::UNSPECIFIED);
    let ltr_368 = measure_row(&ltr, ProposalSize::new(Some(368.0), None));
    let rtl_none = measure_row(&rtl, ProposalSize::UNSPECIFIED);
    let rtl_368 = measure_row(&rtl, ProposalSize::new(Some(368.0), None));
    assert!(
        (rtl_368 - natural).abs() < 1.0
            && (ltr_368 - natural).abs() < 1.0
            && (rtl_none - natural).abs() < 1.0,
        "ltr None={natural}, ltr 368={ltr_368}, rtl None={rtl_none}, rtl 368={rtl_368}"
    );
}
