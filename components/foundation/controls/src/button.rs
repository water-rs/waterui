//! Button component for `WaterUI`
//!
//! This module provides a Button component that allows users to trigger actions
//! when clicked.
//!
//! ![Button](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/button.svg)
//!
//! # Overview
//!
//! Buttons support two patterns for handling click actions:
//!
//! 1. **Simple actions** - No parameters, just execute code
//! 2. **Extractor-based actions** - Extract values from the environment at click time
//!
//! # Examples
//!
//! ## Simple Action
//!
//! The simplest form takes a closure with no parameters:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! # fn simple() {
//! button("Click me").action(|| {});
//! # }
//! ```
//!
//! ## State Injection
//!
//! Use `.state()` on the resulting view to inject cloneable state into the
//! environment, then extract it inside `action(...)` with `State<T>`:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::reactive::binding;
//!
//! # fn injected() {
//! let counter: Binding<i32> = binding(0);
//!
//! button("Increment")
//!     .action(|State(count): State<Binding<i32>>| {
//!         *count.get_mut() += 1;
//!     })
//!     .state(&counter);
//!
//! let query: Binding<Str> = binding("");
//! let history: Binding<Vec<Str>> = binding(Vec::new());
//!
//! button("Search")
//!     .action(
//!         |State(query): State<Binding<Str>>, State(history): State<Binding<Vec<Str>>>| {
//!             history.with_mut(|history| history.push(query.snapshot()));
//!         },
//!     )
//!     .state(&query)
//!     .state(&history);
//! # }
//! ```
//!
//! ## Environment Extraction
//!
//! Any extractor type can be used directly as an `action(...)` parameter, and
//! an owned `Clone` type becomes one with `#[state]` — injected the same way
//! as `State<T>` values, with `.state(&value)`:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! #[state]
//! #[derive(Clone)]
//! struct CurrentUser(Str);
//! # impl CurrentUser {
//! #     fn rename(&self, _name: Str) {}
//! # }
//!
//! # fn extracted(user: &CurrentUser) {
//! button("Rename")
//!     .action(|user: CurrentUser| {
//!         user.rename(Str::from("Ada"));
//!     })
//!     .state(user);
//! # }
//! ```
//!
//! [`impl_extractor!`](waterui_core::impl_extractor) is the other channel: it
//! marks a `Clone` type read straight from the environment — a value installed
//! with `env.insert(..)` or `.with(..)`, typically by `app(env)` or a backend —
//! not one injected through `.state()`:
//!
//! ```rust
//! use waterui::impl_extractor;
//! use waterui::prelude::*;
//!
//! #[derive(Clone)]
//! struct DatabaseConnection;
//! impl_extractor!(DatabaseConnection);
//! # impl DatabaseConnection {
//! #     fn save_user_preferences(&self) {}
//! # }
//!
//! # fn extracted(db: &DatabaseConnection) {
//! button("Save")
//!     .action(|db: DatabaseConnection| {
//!         db.save_user_preferences();
//!     })
//!     .with(db.clone());
//! # }
//! ```
//!
//! ## Combined State and Extraction
//!
//! State and custom extractors compose in a single handler:
//!
//! ```rust
//! use waterui::impl_extractor;
//! use waterui::prelude::*;
//!
//! # use waterui::reactive::binding;
//! # #[derive(Clone, Default)]
//! # struct FormData {
//! #     email: Str,
//! # }
//! # #[derive(Clone)]
//! # struct ApiClient;
//! # impl_extractor!(ApiClient);
//! # impl ApiClient {
//! #     fn submit(&self, _form: FormData) {}
//! # }
//! # fn combined() {
//! # let form_data: Binding<FormData> = binding(FormData::default());
//! button("Submit")
//!     .action(|State(data): State<Binding<FormData>>, client: ApiClient| {
//!         client.submit(data.snapshot());
//!     })
//!     .state(&form_data);
//! # }
//! ```
//!
//! ## Async Actions
//!
//! All patterns support async variants with `action_async`:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! # use waterui::reactive::binding;
//! # async fn fetch_from_server() -> Str {
//! #     Str::from("payload")
//! # }
//! # fn asynchronous() {
//! # let result_binding: Binding<Str> = binding("");
//! button("Fetch Data")
//!     .action_async(|State(result): State<Binding<Str>>| async move {
//!         let data = fetch_from_server().await;
//!         result.set(data);
//!     })
//!     .state(&result_binding);
//! # }
//! ```
//!
//! ## Button Styles
//!
//! Use `.style()` or convenience methods to change the button's visual appearance:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui_controls::ButtonStyle;
//!
//! # fn styled() {
//! // Using style method
//! button("Primary Action")
//!     .style(ButtonStyle::BorderedProminent)
//!     .action(|| { /* ... */ });
//!
//! // Using convenience methods
//! button("Learn More")
//!     .link()
//!     .action(|| { /* ... */ });
//!
//! button("Secondary")
//!     .bordered()
//!     .action(|| { /* ... */ });
//!
//! button("Subtle")
//!     .plain()
//!     .action(|| { /* ... */ });
//! # }
//! ```

use crate::size::ControlSize;
use core::fmt;
use core::future::Future;
use fmt::Debug;

use alloc::boxed::Box;
use executor_core::spawn_local;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
#[cfg(feature = "menu")]
use waterui_core::handler::ReportExtraction;
use waterui_core::handler::{BoxedAction, ExtractionError, Handler};
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_text::IntoText;

use crate::label::{ControlLabel, IntoLabel, Label, LabelDisplayMode, LabelKind};

// ============================================================================
// Macros for reducing boilerplate
// ============================================================================

/// Implements style convenience methods for a button builder type.
macro_rules! impl_style_methods {
    () => {
        /// Sets the button style to plain (no background or border).
        /// Suitable for low-emphasis actions or toolbar buttons.
        #[must_use]
        pub const fn plain(self) -> Self {
            self.style(ButtonStyle::Plain)
        }

        /// Sets the button style to link (hyperlink appearance).
        /// Used for URL navigation and text-based links.
        #[must_use]
        pub const fn link(self) -> Self {
            self.style(ButtonStyle::Link)
        }

        /// Sets the button style to borderless.
        /// Similar to plain but with more interactive feedback.
        #[must_use]
        pub const fn borderless(self) -> Self {
            self.style(ButtonStyle::Borderless)
        }

        /// Sets the button style to bordered.
        /// Suitable for secondary actions.
        #[must_use]
        pub const fn bordered(self) -> Self {
            self.style(ButtonStyle::Bordered)
        }

        /// Sets the button style to bordered prominent.
        /// Use for primary actions that should stand out.
        #[must_use]
        pub const fn bordered_prominent(self) -> Self {
            self.style(ButtonStyle::BorderedProminent)
        }
    };
}

// ============================================================================
// ButtonStyle
// ============================================================================

/// Visual style options for buttons.
///
/// Different button styles provide different visual emphasis and behavior.
/// The actual rendering depends on the platform's native UI toolkit.
///
/// # Examples
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui_controls::ButtonStyle;
///
/// # fn styles() {
/// // Primary action with prominent styling
/// button("Submit").style(ButtonStyle::BorderedProminent);
///
/// // Secondary action with subtle styling
/// button("Cancel").style(ButtonStyle::Plain);
///
/// // Navigation link
/// button("Learn More").style(ButtonStyle::Link);
/// # }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonStyle {
    /// The default button style, determined by the platform and context.
    /// On macOS/iOS, this typically renders as a rounded rectangle with background.
    #[default]
    Automatic,
    /// A plain text button without any background or border.
    /// Suitable for low-emphasis actions or toolbar buttons.
    Plain,
    /// A button styled as a hyperlink, typically with underlined blue text.
    /// Used for URL navigation and text-based links.
    Link,
    /// A button without a visible border, but may show hover/press effects.
    /// Similar to Plain but with more interactive feedback.
    Borderless,
    /// A button with a subtle border or background.
    /// Suitable for secondary actions.
    Bordered,
    /// A prominent button with filled background color.
    /// Use for primary actions that should stand out.
    BorderedProminent,
    /// A Liquid Glass capsule: the button floats over its content in the
    /// platform's glass, taking its highlight from what is behind it.
    ///
    /// Glass is an Apple primitive. Backends without it render this as their
    /// bordered style, since the emphasis is the same.
    Glass,
    /// A Liquid Glass capsule filled with the accent color, for the primary
    /// action among glass controls.
    ///
    /// Backends without glass render this as their bordered-prominent style.
    GlassProminent,
}

/// A button style installed on a subtree becomes the default for buttons that
/// did not pick one, the same way [`LabelDisplayMode`] adapts labels to their
/// surroundings.
impl waterui_core::plugin::Plugin for ButtonStyle {}

// ============================================================================
// Button configuration and view implementation
// ============================================================================

/// Configuration for a button component.
///
/// This struct contains all the data needed to render a button on native platforms.
/// It is produced by [`Button::config`] and consumed by the native backend.
///
/// # Layout Behavior
///
/// Buttons size themselves to fit their label content and never stretch to fill
/// extra space. In a stack, they take only the space they need.
#[non_exhaustive]
pub struct ButtonConfig<S> {
    /// The semantic label displayed on the button.
    ///
    /// [`ControlLabel`] carries both the visual chrome (text + icon + display mode)
    /// and the spoken text used by assistive technology. A hook receiving a
    /// `ButtonConfig` gets a typed [`ControlLabel`], so it can wrap or restyle it
    /// without stripping away the semantic identity.
    label: ControlLabel<S>,
    /// The action to execute when the button is clicked.
    action: BoxedAction<()>,
    /// The visual style of the button.
    style: ButtonStyle,
    /// How large the button is drawn.
    size: ControlSize,
}

impl<S> ButtonConfig<S> {
    /// The semantic label displayed on the button.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// The action to execute when the button is clicked.
    #[must_use]
    pub const fn action(&self) -> &BoxedAction<()> {
        &self.action
    }

    /// The action to execute when the button is clicked, for calling it.
    pub const fn action_mut(&mut self) -> &mut BoxedAction<()> {
        &mut self.action
    }

    /// Splits the value into its label, action, style and size.
    pub fn into_parts(self) -> (ControlLabel<S>, BoxedAction<()>, ButtonStyle, ControlSize) {
        (self.label, self.action, self.style, self.size)
    }

    /// The visual style of the button.
    #[must_use]
    pub const fn style(&self) -> ButtonStyle {
        self.style
    }

    /// How large the button is drawn.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }
}

impl<S> Debug for ButtonConfig<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ButtonConfig")
    }
}

impl<S: 'static> NativeView for ButtonConfig<S> {}

impl<S: 'static> MapSlots<S> for ButtonConfig<S> {
    type With<T: 'static> = ButtonConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ButtonConfig<T> {
        ButtonConfig {
            label: self.label.map_slots(f),
            action: self.action,
            style: self.style,
            size: self.size,
        }
    }
}

impl ButtonStyle {
    /// This style in `env`. `Automatic` means "whatever this context calls
    /// for", and a surrounding scope can say what that is — a list row wants
    /// row-style buttons, not the filled containers a screen's primary
    /// action gets — while an explicit style still wins.
    pub(crate) fn resolve(self, env: &Environment) -> Self {
        match (self, env.get::<Self>()) {
            (Self::Automatic, Some(ambient)) => *ambient,
            (style, _) => style,
        }
    }
}

fn render_button_config<L, Action, S>(
    button: Button<L, Action>,
    env: &Environment,
) -> ButtonConfig<S>
where
    L: LabelKind,
    Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    S: ChildSlot,
{
    ButtonConfig {
        // A button label stays one truncated line — the SwiftUI and Material
        // baseline — instead of folding into a paragraph under compression.
        // Applied here so themed hooks and native backends agree, while an
        // explicit `Text::line_limit` from the application still wins.
        label: button
            .label
            .default_text_line_limit(core::num::NonZeroUsize::MIN)
            .into_control_label::<S>(env),
        action: Box::new(button.action),
        style: button.style.resolve(env),
        size: button.size,
    }
}

impl<L, Action> View for Button<L, Action>
where
    L: LabelKind,
    Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        Native::new(render_button_config::<_, _, AnyView>(self, env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(render_button_config::<_, _, B::Slot>(self, env), env, cx)
    }
}

impl<S: View> From<ButtonConfig<S>> for Button<ControlLabel<S>, BoxedAction<()>> {
    fn from(value: ButtonConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            action: value.action,
            style: value.style,
            size: value.size,
        }
    }
}

impl<L, Action> Button<L, Action>
where
    L: LabelKind,
    Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
{
    /// Returns the configuration for this button.
    #[must_use]
    pub fn config(self) -> ButtonConfig<Label<L>> {
        ButtonConfig {
            label: self.label.into_unscoped_control_label(),
            action: Box::new(self.action),
            style: self.style,
            size: self.size,
        }
    }
}

// ============================================================================
// Button struct
// ============================================================================

/// A button component that triggers an action when clicked.
///
/// Create buttons using the [`button`] convenience function or [`Button::new`].
/// Configure the action using one of these patterns:
///
/// - [`action`](Button::action) - Simple closure with no parameters
/// - [`action`](Button::action) with extractor parameters - Extract from the environment at click time
/// - `ViewExt::state` - Inject local cloneable state for later extraction
///
/// See the [module documentation](self) for detailed examples.
pub struct Button<L, Action> {
    label: Label<L>,
    action: Action,
    style: ButtonStyle,
    size: ControlSize,
}

impl<L, Action> Button<L, Action> {
    /// Returns the semantic label displayed by this button.
    #[must_use]
    pub const fn label(&self) -> &Label<L> {
        &self.label
    }

    /// Returns the visual style requested for this button.
    #[must_use]
    pub const fn button_style(&self) -> ButtonStyle {
        self.style
    }

    /// Returns the size this button is drawn at.
    #[must_use]
    pub const fn button_size(&self) -> ControlSize {
        self.size
    }

    /// Sets how large the button is drawn. Buttons default to
    /// [`ControlSize::Small`].
    #[must_use]
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }
}

impl<L: Debug, Action> Debug for Button<L, Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Button")
            .field("label", &self.label)
            .field("style", &self.style)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<L: Clone, Action: Clone> Clone for Button<L, Action> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            action: self.action.clone(),
            style: self.style,
            size: self.size,
        }
    }
}

/// The action of a button that has none configured yet.
pub type NoAction = fn(&Environment) -> Result<(), ExtractionError>;

impl<L> Button<L, NoAction> {
    /// Creates a new button with the specified semantic label.
    ///
    /// The button has no action by default. Use [`action`](Self::action) or
    /// [`action_async`](Self::action_async) to configure what happens when the
    /// button is clicked.
    ///
    /// # Arguments
    ///
    /// * `label` - The semantic label to display on the button
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::{Button, label::label};
    /// # fn demo() {
    /// let btn = Button::new(label("Click me")).action(|| {});
    /// # }
    /// ```
    #[must_use]
    pub fn new(label: Label<L>) -> Self {
        Self {
            label,
            action: noop,
            style: ButtonStyle::Automatic,
            size: ControlSize::Small,
        }
    }
}

impl<L, Action> Button<L, Action> {
    /// Sets the visual style of the button.
    ///
    /// # Arguments
    ///
    /// * `style` - The [`ButtonStyle`] to apply
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::ButtonStyle;
    /// # fn submit_form() {}
    /// # fn demo() {
    /// button("Submit")
    ///     .style(ButtonStyle::BorderedProminent)
    ///     .action(|| submit_form());
    /// # }
    /// ```
    #[must_use]
    pub const fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Overrides the spoken accessibility text announced by assistive technology.
    ///
    /// This rewrites only the textual portion of the semantic label, preserving
    /// any icon attached at construction time. The visual label rendered on
    /// screen is unaffected.
    #[must_use]
    pub fn accessibility_label(mut self, label: impl IntoText) -> Self {
        self.label = self.label.accessibility_text(label);
        self
    }

    /// Sets the visual presentation mode of the button's label.
    ///
    /// The semantic identity is always preserved for assistive technology
    /// regardless of the chosen visual mode.
    #[must_use]
    pub const fn label_style(mut self, mode: LabelDisplayMode) -> Self {
        self.label.set_display_mode(mode);
        self
    }

    /// Visually hides the button's label while keeping its semantic text in
    /// the accessibility tree.
    ///
    /// Shortcut for `.label_style(LabelDisplayMode::Hidden)`.
    #[must_use]
    pub const fn hide_label(self) -> Self {
        self.label_style(LabelDisplayMode::Hidden)
    }

    impl_style_methods!();

    /// Sets the action to be performed when the button is clicked.
    ///
    /// This is the simplest action form - a closure that takes no parameters.
    /// Use this when the action doesn't need any external state or
    /// environment values.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure to execute when clicked
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # fn demo() {
    /// button("Click me").action(|| {});
    /// # }
    /// ```
    #[must_use]
    pub fn action<F, Args>(
        self,
        action: F,
    ) -> Button<L, impl FnMut(&Environment) -> Result<(), ExtractionError>>
    where
        F: Handler<Args, ()> + 'static,
    {
        Button {
            label: self.label,
            action: waterui_core::handler::boxed_action(action),
            style: self.style,
            size: self.size,
        }
    }

    /// Sets an asynchronous action to be performed when the button is clicked.
    ///
    /// The future is spawned on the local executor and detached. Use this
    /// for actions that need to perform async operations like network requests.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that returns a future
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # mod api {
    /// #     use waterui::Str;
    /// #     pub async fn fetch_data() -> Str {
    /// #         Str::from("payload")
    /// #     }
    /// # }
    /// # fn process(_data: Str) {}
    /// # fn demo() {
    /// button("Fetch").action_async(|| async {
    ///     let data = api::fetch_data().await;
    ///     process(data);
    /// });
    /// # }
    /// ```
    pub fn action_async<F, Fut, Args>(
        self,
        action: F,
    ) -> Button<L, impl FnMut(&Environment) -> Result<(), ExtractionError>>
    where
        F: Handler<Args, Fut> + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let mut action = action;
        Button {
            label: self.label,
            // A failed extraction means the action does not run: no future is
            // spawned, and the failure travels to the button's dispatcher.
            action: move |env: &Environment| {
                spawn_local(action.call(env)?).detach();
                Ok(())
            },
            style: self.style,
            size: self.size,
        }
    }
}

/// No-op action for buttons without a configured action.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the signature is the button action's"
)]
const fn noop(_env: &Environment) -> Result<(), ExtractionError> {
    Ok(())
}

#[cfg(feature = "menu")]
impl<L: LabelKind, Action> From<Button<L, Action>> for crate::menu::Command
where
    Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
{
    fn from(value: Button<L, Action>) -> Self {
        let Button {
            label,
            mut action,
            style: _,
            size: _,
        } = value;
        Self::from_semantic_label(label.into_semantic()).action(move |env: Environment| {
            action(&env).report_extraction();
        })
    }
}

#[cfg(feature = "menu")]
impl<L: LabelKind, Action> From<Button<L, Action>> for crate::menu::MenuItem
where
    Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
{
    fn from(value: Button<L, Action>) -> Self {
        Self::Command(value.into())
    }
}

// ============================================================================
// Convenience function
// ============================================================================

/// Creates a new button with the specified label.
///
/// This is a convenience function equivalent to [`Button::new`].
///
/// # Arguments
///
/// * `label` - The semantic label to display on the button
///
/// # Example
///
/// ```rust
/// use waterui::prelude::*;
///
/// # fn demo() {
/// button("Click me").action(|| {});
/// # }
/// ```
pub fn button<L: IntoLabel>(label: L) -> Button<L::Kind, NoAction> {
    Button::new(label.into_label())
}

#[cfg(all(test, feature = "menu"))]
mod tests {
    use super::*;
    use waterui_core::Signal;

    #[derive(Clone, Copy)]
    struct SearchIcon;

    impl View for SearchIcon {
        fn body(self, _env: &Environment) -> impl View {}
    }

    #[test]
    fn menu_command_preserves_label_through_config_render_roundtrip() {
        crate::init_test_executor();
        let rendered = Button::from(
            button(crate::label::label("Edit").icon(SearchIcon))
                .action(|| {})
                .config(),
        );
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label()
                .semantic_text()
                .resolve(&Environment::default())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Edit"
        );
        let icon = command
            .label()
            .icon_view()
            .expect("label icon view should be preserved");
        assert!(icon.build().downcast_ref::<SearchIcon>().is_some());
    }

    #[test]
    fn accessibility_label_overrides_text_only() {
        crate::init_test_executor();
        // Calling .accessibility_label("X") must rewrite the spoken text
        // while preserving the visible text and icon attached at construction time.
        let rendered = Button::from(
            button(crate::label::label("Edit").icon(SearchIcon))
                .accessibility_label("Edit document")
                .action(|| {})
                .config(),
        );
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label()
                .semantic_text()
                .resolve(&Environment::default())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Edit"
        );
        assert_eq!(
            command
                .label()
                .clone()
                .resolve(&Environment::default())
                .accessibility_label()
                .snapshot()
                .to_plain()
                .as_str(),
            "Edit document"
        );
        let icon = command
            .label()
            .icon_view()
            .expect("icon should be preserved when only the spoken text is overridden");
        assert!(icon.build().downcast_ref::<SearchIcon>().is_some());
    }
}
