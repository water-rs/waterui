//! Math rendering example for WaterUI.

use waterui::app::App;
use waterui::math::Math;
use waterui::prelude::*;
use waterui::preview;
use waterui::widget::RichText;

fn main_view() -> impl View {
    let markdown = "Inline markdown math: $e^{i\\pi}+1=0$ and $\\frac{a_1+b_1}{\\sqrt{x}}$.\n\nBlock markdown math:\n\n$$\\int_0^\\infty e^{-x^2}dx = \\frac{\\sqrt{\\pi}}{2}$$";

    scroll(
        vstack((
            text("WaterUI Math Formula Rendering").headline(),
            text("LaTeX").bold(),
            Math::latex(r"\frac{\alpha_1+\beta_2}{\sqrt{x^2+y^2}} = \sum_{i=0}^{n} i^2")
                .block()
                .font_size(28.0),
            text("MathML").bold(),
            Math::mathml(
                "<math><mfrac><mrow><mi>a</mi><mo>+</mo><mi>b</mi></mrow><msqrt><mi>x</mi></msqrt></mfrac></math>",
            )
            .font_size(26.0),
            text("Typst").bold(),
            Math::typst("frac(alpha + beta_1, sqrt(x^2 + y^2))")
                .font_size(26.0)
                .block(),
            text("Markdown inline formulas").bold(),
            RichText::from_markdown(markdown),
        ))
        .spacing(16.0)
        .padding(),
    )
}

#[preview]
fn math_preview() -> impl View {
    main_view()
}

pub fn app(env: Environment) -> App {
    App::new(main_view, env)
}

waterui_ffi::export!();
