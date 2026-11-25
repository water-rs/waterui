//! The disclosure look of the theme contract: what a theme returns for a
//! [`DisclosureSpec`] (`DisclosureLook`).
//!
//! A disclosure draws a focusable header row whose [`Chrome`] handles hover,
//! press and keyboard focus feedback, a [`ToggleControl`] indicator that
//! reflects `expanded`, and layout spacing between the indicator, header
//! label and expanded content.

use core::time::Duration;

use cherenkov_record::{Draw as _, FillRule, Paint, ShapeData, WorkingColor};
use kurbo::{BezPath, Cap, Join, Rect, Size, Stroke};
use waterui::Environment;
use waterui::animation::Animation;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;

use super::look::SpecTint;
use super::{
    Chrome, CornerShape, Custom, Driven, FocusIndicator, HitRegion, Inputs, Motion, PressFeedback,
    StateLayer, Surface, ToggleControl, ValueMotion,
};

/// Structural inputs Hydrolysis passes to [`WidgetTheme::disclosure`](super::WidgetTheme::disclosure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosureSpec {
    direction: LayoutDirection,
    tint: SpecTint,
}

impl DisclosureSpec {
    /// Creates a spec for a disclosure laid out in `direction` with `tint`.
    #[must_use]
    pub const fn new(direction: LayoutDirection, tint: SpecTint) -> Self {
        Self { direction, tint }
    }

    /// The layout direction the disclosure is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The resolved tint colour for interactive focus/accent highlights.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }
}

/// Layout metrics for a disclosure control in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisclosureLayout {
    /// Minimum height of the header row.
    pub min_header_height: f64,
    /// Horizontal inset inside the header row.
    pub horizontal_inset: f64,
    /// Vertical inset inside the header row.
    pub vertical_inset: f64,
    /// Vertical gap between the header row and expanded content.
    pub content_spacing: f64,
    /// Leading indent applied to expanded content (mirrored in RTL).
    pub content_indent: f64,
}

/// Visual look and layout metrics of a disclosure control.
#[derive(Debug, Clone)]
pub struct DisclosureLook {
    control: ToggleControl,
    chrome: Chrome,
    layout: DisclosureLayout,
}

impl DisclosureLook {
    /// Creates a disclosure look from an indicator `control`, header row
    /// `chrome`, and `layout` metrics.
    #[must_use]
    pub const fn new(control: ToggleControl, chrome: Chrome, layout: DisclosureLayout) -> Self {
        Self {
            control,
            chrome,
            layout,
        }
    }

    /// The indicator control drawn beside the header label.
    #[must_use]
    pub const fn control(&self) -> &ToggleControl {
        &self.control
    }

    /// The header row's interactive chrome.
    #[must_use]
    pub const fn chrome(&self) -> &Chrome {
        &self.chrome
    }

    /// Layout metrics for header padding and expanded content placement.
    #[must_use]
    pub const fn layout(&self) -> DisclosureLayout {
        self.layout
    }
}

fn chevron_path(rect: Rect, expanded: bool, direction: LayoutDirection) -> ShapeData {
    let center = rect.center();
    let half_w = rect.width() * 0.22;
    let half_h = rect.height() * 0.22;
    let dx = half_w * 0.55;
    let dy = half_h * 0.55;
    let mut path = BezPath::new();
    if expanded {
        path.move_to((center.x - half_w, center.y - dy));
        path.line_to((center.x, center.y + dy));
        path.line_to((center.x + half_w, center.y - dy));
    } else if matches!(direction, LayoutDirection::RightToLeft) {
        path.move_to((center.x + dx, center.y - half_h));
        path.line_to((center.x - dx, center.y));
        path.line_to((center.x + dx, center.y + half_h));
    } else {
        path.move_to((center.x - dx, center.y - half_h));
        path.line_to((center.x + dx, center.y));
        path.line_to((center.x - dx, center.y + half_h));
    }
    ShapeData::Path {
        elements: path.elements().into(),
        rule: FillRule::NonZero,
    }
}

fn chevron_part(
    direction: LayoutDirection,
    expanded: bool,
    color: &Color,
    env: &Environment,
    value_motion: ValueMotion,
) -> Custom<bool, (Driven<Paint, bool>,)> {
    let opacity = Driven::new(move |inputs: &Inputs<'_, bool>| {
        if *inputs.value == expanded {
            1.0_f32
        } else {
            0.0_f32
        }
    })
    .value_motion(move |_| value_motion.clone());
    Custom::new(
        (Driven::follow(
            color.resolve(env),
            |resolved: &WorkingColor, _| Paint::from(*resolved),
        ),),
        move |recorder, rect, (live_paint,)| {
            let stroke = Stroke::new(1.5)
                .with_caps(Cap::Round)
                .with_join(Join::Round);
            recorder.stroke(chevron_path(rect, expanded, direction), stroke, live_paint);
        },
    )
    .opacity(opacity)
}

/// Default framework disclosure look used until a style package overrides
/// [`WidgetTheme::disclosure`](super::WidgetTheme::disclosure).
#[must_use]
pub fn default_disclosure_look(spec: &DisclosureSpec, env: &Environment) -> DisclosureLook {
    let duration = Duration::from_millis(120);
    let motion = Motion::uniform(Animation::linear(duration));
    let value_motion = ValueMotion::Follow(Animation::linear(duration));
    let direction = spec.direction();
    let indicator_color = Color::new(waterui::theme::color::MutedForeground);
    let control = ToggleControl::new(Size::new(16.0, 16.0), motion.clone())
        .label_spacing(8.0)
        .part(chevron_part(
            direction,
            false,
            &indicator_color,
            env,
            value_motion.clone(),
        ))
        .part(chevron_part(
            direction,
            true,
            &indicator_color,
            env,
            value_motion,
        ));
    let chrome = Chrome::new(
        Surface::clear(CornerShape::radius(6.0)),
        StateLayer::new(Color::new(waterui::theme::color::Foreground)),
        PressFeedback::new(),
        FocusIndicator::Tint,
        motion,
    )
    .hit(HitRegion::NONE);
    DisclosureLook::new(
        control,
        chrome,
        DisclosureLayout {
            min_header_height: 32.0,
            horizontal_inset: 6.0,
            vertical_inset: 4.0,
            content_spacing: 6.0,
            content_indent: 24.0,
        },
    )
}
