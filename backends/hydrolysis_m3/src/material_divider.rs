//! Material Design 3 divider composed from the WaterUI divider primitive.

use waterui::widget::Divider;

/// A Material Design 3 divider.
///
/// Divider orientation follows the surrounding stack axis, matching the
/// `WaterUI` divider primitive. The Material theme supplies MD3 outline-variant
/// color and 1 logical-unit thickness through the Hydrolysis widget contract.
pub type MaterialDivider = Divider;

/// Creates a Material Design 3 divider.
#[must_use]
pub const fn material_divider() -> MaterialDivider {
    Divider
}

#[cfg(test)]
mod tests {
    use crate::layout::divider::metrics;

    #[test]
    fn material_divider_tokens_match_material_web_v0_192() {
        let metrics = metrics();

        assert_eq!(metrics.thickness, 1.0);
    }
}
