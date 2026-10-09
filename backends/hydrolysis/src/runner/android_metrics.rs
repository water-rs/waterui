//! The metrics value the Android host's window-insets regions derive
//! their logical values from — compiled on Android for `set_metrics`'s
//! change detection and on host for its tests; dead elsewhere.
//!
//! `nativeSetMetrics` pushes size, density, font scale, refresh and both
//! system-insets regions as one coherent snapshot. A region's logical
//! insets derive from two of those values — the region's physical-pixel
//! edges AND the density — so the change detection compares that whole
//! value between pushes. Comparing the pixel edges alone is a subset
//! that leaves the published `WindowSafeArea`/`WindowKeyboardArea`
//! bindings stale on a density-only push (water-rs/waterui#2295).

use waterui_layout::padding::EdgeInsets;

/// The metrics one window-insets region's logical value derives from:
/// the region's physical-pixel edges plus the display density that
/// converts them. `AndroidSession::set_metrics` compares this whole
/// value between pushes — a density-only change flags both regions even
/// though no pixel edge moved.
#[derive(Clone, Copy, Debug)]
pub struct InsetsMetrics {
    /// Region edges in physical px: `[left, top, right, bottom]` — the
    /// layout order the JNI push already delivers.
    px: [i32; 4],
    /// Physical pixels per logical unit — the platform scale factor.
    density: f64,
}

impl InsetsMetrics {
    /// Pairs a pushed region's pixel edges with the snapshot's density.
    pub const fn new(px: [i32; 4], density: f64) -> Self {
        Self { px, density }
    }

    /// The logical insets the session publishes for the region — the
    /// pixel edges in this density's logical units.
    pub const fn logical(self) -> EdgeInsets {
        let density = crate::num_cast::f64_as_f32(self.density);
        let [leading, top, trailing, bottom] = self.px;
        EdgeInsets::new(
            crate::num_cast::i32_as_f32(top) / density,
            crate::num_cast::i32_as_f32(bottom) / density,
            crate::num_cast::i32_as_f32(leading) / density,
            crate::num_cast::i32_as_f32(trailing) / density,
        )
    }
}

impl PartialEq for InsetsMetrics {
    /// Bitwise on the density, like the size check `set_metrics` runs on
    /// the same push — a push that differs only in representation still
    /// flags the regions.
    fn eq(&self, other: &Self) -> bool {
        self.px == other.px && self.density.to_bits() == other.density.to_bits()
    }
}

#[cfg(test)]
mod tests {
    use waterui_layout::padding::EdgeInsets;

    use super::InsetsMetrics;

    /// A push that moves only the density — a display-size change or a
    /// window moved to another-density display — still flags both
    /// regions and re-derives their logical insets in the new units
    /// (water-rs/waterui#2295).
    #[test]
    fn a_density_only_change_recomputes_the_logical_insets() {
        let container_before = InsetsMetrics::new([0, 96, 0, 48], 2.0);
        let keyboard_before = InsetsMetrics::new([0, 0, 0, 640], 2.0);
        let container_after = InsetsMetrics::new([0, 96, 0, 48], 3.0);
        let keyboard_after = InsetsMetrics::new([0, 0, 0, 640], 3.0);
        assert_ne!(container_before, container_after);
        assert_ne!(keyboard_before, keyboard_after);
        assert_eq!(
            container_after.logical(),
            EdgeInsets::new(32.0, 16.0, 0.0, 0.0)
        );
        assert_eq!(
            keyboard_after.logical(),
            EdgeInsets::new(0.0, 640.0 / 3.0, 0.0, 0.0)
        );
    }

    /// The compare is also the dedupe that keeps an IME animation's
    /// repeated pushes quiet: an unchanged region must not flag, or every
    /// progress frame would republish and re-lay out.
    #[test]
    fn an_identical_push_flags_no_change() {
        let metrics = InsetsMetrics::new([0, 48, 0, 24], 2.0);
        assert_eq!(metrics, InsetsMetrics::new([0, 48, 0, 24], 2.0));
    }

    /// A pixel-edge move still flags the region — the common push, such
    /// as an IME animation frame carrying a new keyboard band.
    #[test]
    fn a_pixel_edge_change_flags_the_region() {
        let before = InsetsMetrics::new([0, 0, 0, 320], 2.0);
        assert_ne!(before, InsetsMetrics::new([0, 0, 0, 640], 2.0));
    }
}
