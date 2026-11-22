//! Declarative showcase cases, stages, viewports, and appearances.

use core::fmt;

use waterui::theme::ColorScheme;
use waterui_core::{AnyView, Environment, View};
use waterui_testing::OffscreenApp;

use crate::flip::FlipThresholds;

type ViewFactory = Box<dyn Fn() -> AnyView>;
type StageAction = Box<dyn Fn(&mut OffscreenApp)>;
type EnvSetup = Box<dyn Fn(&mut Environment)>;

/// Light or dark appearance applied to a [`RenderCase`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "matches WaterUI's closed two-variant ColorScheme (Light / Dark)"
)]
pub enum Appearance {
    /// Light mode (`ColorScheme::Light`).
    #[default]
    Light,
    /// Dark mode (`ColorScheme::Dark`).
    Dark,
}

impl Appearance {
    /// Both standard appearances in deterministic order.
    pub const ALL: [Self; 2] = [Self::Light, Self::Dark];

    /// Stable lowercase identifier used in artifact directories and reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Converts to `waterui::theme::ColorScheme`.
    #[must_use]
    pub const fn color_scheme(self) -> ColorScheme {
        match self {
            Self::Light => ColorScheme::Light,
            Self::Dark => ColorScheme::Dark,
        }
    }
}

/// Logical viewport size and pixel scale factor for a [`RenderCase`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Logical viewport width in points.
    pub width: u32,
    /// Logical viewport height in points.
    pub height: u32,
    /// Physical pixels per logical point.
    pub scale_factor: f64,
}

impl Viewport {
    /// Creates a viewport at scale factor `1.0`.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            scale_factor: 1.0,
        }
    }

    /// Sets the pixel scale factor (for example `2.0` for Retina captures).
    #[must_use]
    pub const fn with_scale_factor(mut self, scale_factor: f64) -> Self {
        self.scale_factor = scale_factor;
        self
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::new(420, 320)
    }
}

/// One deterministic capture point within a [`RenderCase`], optionally preceded
/// by interactions and semantic assertions on [`OffscreenApp`].
pub struct RenderStage {
    pub(crate) name: String,
    pub(crate) action: Option<StageAction>,
    pub(crate) allow_blank: bool,
    pub(crate) thresholds: Option<FlipThresholds>,
}

impl fmt::Debug for RenderStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderStage")
            .field("name", &self.name)
            .field("has_action", &self.action.is_some())
            .field("allow_blank", &self.allow_blank)
            .field("thresholds", &self.thresholds)
            .finish()
    }
}

impl RenderStage {
    /// Creates a stage that captures the current rendered frame with no prior
    /// interaction.
    #[must_use]
    pub fn capture(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            action: None,
            allow_blank: false,
            thresholds: None,
        }
    }

    /// Creates a stage that executes `action` (interactions and/or semantic
    /// assertions) before capturing the frame.
    #[must_use]
    pub fn interact<F>(name: impl Into<String>, action: F) -> Self
    where
        F: Fn(&mut OffscreenApp) + 'static,
    {
        Self {
            name: name.into(),
            action: Some(Box::new(action)),
            allow_blank: false,
            thresholds: None,
        }
    }

    /// Allows blank frames without failing this stage.
    #[must_use]
    pub const fn allow_blank(mut self, allow: bool) -> Self {
        self.allow_blank = allow;
        self
    }

    /// Overrides the FLIP thresholds for this stage when compared against a
    /// reference image.
    #[must_use]
    pub const fn thresholds(mut self, thresholds: FlipThresholds) -> Self {
        self.thresholds = Some(thresholds);
        self
    }

    /// Stage identifier (used as `<stage>.png`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Declarative visual and interaction conformance case.
pub struct RenderCase {
    pub(crate) name: String,
    pub(crate) viewport: Viewport,
    pub(crate) appearance: Appearance,
    pub(crate) view_factory: ViewFactory,
    pub(crate) env_setup: Option<EnvSetup>,
    pub(crate) stages: Vec<RenderStage>,
}

impl fmt::Debug for RenderCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderCase")
            .field("name", &self.name)
            .field("viewport", &self.viewport)
            .field("appearance", &self.appearance)
            .field("stages", &self.stages)
            .finish_non_exhaustive()
    }
}

impl RenderCase {
    /// Creates a new case with the default [`Viewport`] and [`Appearance::Light`].
    #[must_use]
    pub fn new<V, F>(name: impl Into<String>, view_factory: F) -> Self
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        Self {
            name: name.into(),
            viewport: Viewport::default(),
            appearance: Appearance::Light,
            view_factory: Box::new(move || AnyView::new(view_factory())),
            env_setup: None,
            stages: Vec::new(),
        }
    }

    /// Sets the logical viewport size (keeping the current scale factor).
    #[must_use]
    pub const fn viewport(mut self, width: u32, height: u32) -> Self {
        self.viewport.width = width;
        self.viewport.height = height;
        self
    }

    /// Sets the pixel scale factor.
    #[must_use]
    pub const fn scale_factor(mut self, scale_factor: f64) -> Self {
        self.viewport.scale_factor = scale_factor;
        self
    }

    /// Sets the case appearance (`Light` or `Dark`).
    #[must_use]
    pub const fn appearance(mut self, appearance: Appearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Configures extra environment entries installed before mounting.
    #[must_use]
    pub fn environment<F>(mut self, setup: F) -> Self
    where
        F: Fn(&mut Environment) + 'static,
    {
        self.env_setup = Some(Box::new(setup));
        self
    }

    /// Appends a [`RenderStage`].
    #[must_use]
    pub fn with_stage(mut self, stage: RenderStage) -> Self {
        self.stages.push(stage);
        self
    }

    /// Appends a capture-only stage named `name`.
    #[must_use]
    pub fn stage(self, name: impl Into<String>) -> Self {
        self.with_stage(RenderStage::capture(name))
    }

    /// Appends an interaction/assertion stage named `name`.
    #[must_use]
    pub fn interaction_stage<F>(self, name: impl Into<String>, action: F) -> Self
    where
        F: Fn(&mut OffscreenApp) + 'static,
    {
        self.with_stage(RenderStage::interact(name, action))
    }

    /// Case name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Configured viewport.
    #[must_use]
    pub const fn configured_viewport(&self) -> Viewport {
        self.viewport
    }

    /// Configured appearance.
    #[must_use]
    pub const fn configured_appearance(&self) -> Appearance {
        self.appearance
    }

    /// Configured stages (when empty, [`crate::GalleryRunner`] captures a
    /// single `"00_initial"` stage).
    #[must_use]
    pub fn stages(&self) -> &[RenderStage] {
        &self.stages
    }
}

/// Named collection of [`RenderCase`] definitions.
#[derive(Debug)]
pub struct ShowcaseSuite {
    pub(crate) name: String,
    pub(crate) cases: Vec<RenderCase>,
}

impl ShowcaseSuite {
    /// Creates an empty showcase suite.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cases: Vec::new(),
        }
    }

    /// Appends a [`RenderCase`] to the suite.
    #[must_use]
    pub fn with_case(mut self, case: RenderCase) -> Self {
        self.cases.push(case);
        self
    }

    /// Pushes a [`RenderCase`] into the suite in place.
    pub fn add_case(&mut self, case: RenderCase) {
        self.cases.push(case);
    }

    /// Adds both Light and Dark variants of a case named `<base_name>-light`
    /// and `<base_name>-dark`.
    #[must_use]
    pub fn with_both_appearances<F>(mut self, base_name: &str, build: F) -> Self
    where
        F: Fn(String, Appearance) -> RenderCase,
    {
        for appearance in Appearance::ALL {
            let name = format!("{base_name}-{}", appearance.as_str());
            self.cases
                .push(build(name, appearance).appearance(appearance));
        }
        self
    }

    /// Suite identifier.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Cases registered in this suite.
    #[must_use]
    pub fn cases(&self) -> &[RenderCase] {
        &self.cases
    }
}
