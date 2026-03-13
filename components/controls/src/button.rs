//! Button component for `WaterUI`
//!
//! This module provides a Button component that allows users to trigger actions
//! when clicked.
//!
//! ![Button](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/button.svg)
//!
//! # Overview
//!
//! Buttons support three patterns for handling click actions:
//!
//! 1. **Simple actions** - No parameters, just execute code
//! 2. **State-based actions** - Capture state at button creation time
//! 3. **Environment extraction** - Extract values from the environment at click time
//!
//! # Examples
//!
//! ## Simple Action
//!
//! The simplest form takes a closure with no parameters:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! button("Click me").action(|| {
//!     println!("Button clicked!");
//! });
//! ```
//!
//! ## State-Based Action
//!
//! Use `.with_state()` to capture values at button creation time. The state is
//! cloned when the button is created, not when clicked. Chain multiple
//! `.with_state()` calls to capture multiple values:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! let counter: Binding<i32> = binding(0);
//!
//! // Single state value
//! button("Increment")
//!     .with_state(&counter)
//!     .action(|count| {
//!         count.set(count.get() + 1);
//!     });
//!
//! // Multiple state values (received as nested tuple)
//! button("Go")
//!     .with_state(&webview)
//!     .with_state(&address)
//!     .action(|(wv, addr)| {
//!         wv.go_to(addr.get().as_str());
//!     });
//! ```
//!
//! ## Environment Extraction
//!
//! Use `.extract::<T>()` to extract values from the environment at click time.
//! The type parameter explicitly specifies what to extract, avoiding type
//! inference issues:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! // Extract a single value
//! button("Apply Theme")
//!     .extract::<Theme>()
//!     .action(|theme| {
//!         apply_theme(theme);
//!     });
//!
//! // Chain multiple extractions
//! button("Save")
//!     .extract::<DatabaseConnection>()
//!     .extract::<CurrentUser>()
//!     .action(|(db, user)| {
//!         db.save_user_preferences(user);
//!     });
//! ```
//!
//! ## Combined State and Extraction
//!
//! You can combine both patterns. State comes first in the tuple, followed by
//! extracted values:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! button("Submit")
//!     .with_state(&form_data)
//!     .extract::<ApiClient>()
//!     .action(|(data, client)| {
//!         client.submit(data.get());
//!     });
//! ```
//!
//! ## Async Actions
//!
//! All patterns support async variants with `action_async`:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! button("Fetch Data")
//!     .with_state(&result_binding)
//!     .action_async(|result| async move {
//!         let data = fetch_from_server().await;
//!         result.set(data);
//!     });
//! ```
//!
//! ## Button Styles
//!
//! Use `.style()` or convenience methods to change the button's visual appearance:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
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
//! ```

use core::fmt::Debug;
use core::future::Future;
use core::marker::PhantomData;

use alloc::boxed::Box;
use executor_core::spawn_local;
use nami::Computed;
use waterui_core::extract::Extractor;
use waterui_core::handler::{BoxedAction, shared_action_with_env};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, Native, NativeView, View, impl_debug};
use waterui_text::{IntoText, Text, styled::StyledStr};

use crate::label::{IntoLabel, Label as SemanticLabel};

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
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// // Primary action with prominent styling
/// button("Submit").style(ButtonStyle::BorderedProminent);
///
/// // Secondary action with subtle styling
/// button("Cancel").style(ButtonStyle::Plain);
///
/// // Navigation link
/// button("Learn More").style(ButtonStyle::Link);
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
}

// ============================================================================
// Button configuration and view implementation
// ============================================================================

/// Configuration for a button component.
///
/// This struct contains all the data needed to render a button on native platforms.
/// It is produced by [`ConfigurableView::config`] and consumed by the native backend.
///
/// # Layout Behavior
///
/// Buttons size themselves to fit their label content and never stretch to fill
/// extra space. In a stack, they take only the space they need.
#[non_exhaustive]
pub struct ButtonConfig {
    /// The label displayed on the button.
    pub label: AnyView,
    /// The action to execute when the button is clicked.
    pub action: BoxedAction<()>,
    /// The visual style of the button.
    pub style: ButtonStyle,
    /// Optional semantic accessibility label resolved from the button label.
    pub accessibility_label: Option<nami::Computed<StyledStr>>,
}

impl_debug!(ButtonConfig);

impl NativeView for ButtonConfig {}

fn render_button_config<Label, Action>(
    button: Button<Label, Action>,
    env: &Environment,
) -> ButtonConfig
where
    Label: View,
    Action: FnMut(&Environment) + 'static,
{
    ButtonConfig {
        label: AnyView::new(button.label),
        action: Box::new(button.action),
        style: button.style,
        accessibility_label: button
            .accessibility_label
            .map(|label| label.__resolve(env).content),
    }
}

impl<Label, Action> View for Button<Label, Action>
where
    Label: View,
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let config = render_button_config(self, env);
        if let Some(hook) = env.get::<Hook<ButtonConfig>>() {
            hook.apply(env, config)
        } else {
            AnyView::new(Native::new(config))
        }
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

impl ViewConfiguration for ButtonConfig {
    type View = Button<AnyView, BoxedAction<()>>;

    fn render(self) -> Self::View {
        Button {
            label: self.label,
            action: self.action,
            style: self.style,
            accessibility_label: self.accessibility_label.map(Text::computed),
        }
    }
}

impl<Label, Action> ConfigurableView for Button<Label, Action>
where
    Label: View,
    Action: FnMut(&Environment) + 'static,
{
    type Config = ButtonConfig;

    fn config(self) -> Self::Config {
        ButtonConfig {
            label: AnyView::new(self.label),
            action: Box::new(self.action),
            style: self.style,
            accessibility_label: None,
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
/// - [`with_state`](Button::with_state) - Capture state, then call action
/// - [`extract`](Button::extract) - Extract from environment, then call action
///
/// See the [module documentation](self) for detailed examples.
pub struct Button<Label, Action> {
    label: Label,
    action: Action,
    style: ButtonStyle,
    accessibility_label: Option<Text>,
}

impl<Label: Debug, Action> Debug for Button<Label, Action> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Button")
            .field("label", &self.label)
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

impl<Label: Clone, Action: Clone> Clone for Button<Label, Action> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            action: self.action.clone(),
            style: self.style,
            accessibility_label: self.accessibility_label.clone(),
        }
    }
}

impl Button<SemanticLabel, fn(&Environment)> {
    /// Creates a new button with the specified label.
    ///
    /// The button has no action by default. Use [`action`](Self::action),
    /// [`with_state`](Self::with_state), or [`extract`](Self::extract) to
    /// configure what happens when the button is clicked.
    ///
    /// # Arguments
    ///
    /// * `label` - The semantic label to display on the button
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let btn = Button::new("Click me").action(|| println!("Clicked!"));
    /// ```
    pub fn new(label: impl IntoLabel) -> Self {
        let label = label.into_label();
        let accessibility_label = Some(label.__text().clone());
        Self {
            label,
            action: noop,
            style: ButtonStyle::Automatic,
            accessibility_label,
        }
    }

    /// Starts building a button with captured state.
    ///
    /// The state value is cloned immediately when this method is called.
    /// Chain multiple `with_state` calls to capture multiple values - they
    /// accumulate as nested tuples: `(first, second)`, then `((first, second), third)`.
    ///
    /// # Arguments
    ///
    /// * `state` - A reference to the value to capture (will be cloned)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Single state
    /// button("Increment")
    ///     .with_state(&counter)
    ///     .action(|count| count.set(count.get() + 1));
    ///
    /// // Multiple states
    /// button("Navigate")
    ///     .with_state(&webview)
    ///     .with_state(&url)
    ///     .action(|(wv, url)| wv.go_to(url.get().as_str()));
    /// ```
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> ButtonBuilder<SemanticLabel, T> {
        ButtonBuilder {
            label: self.label,
            state: state.clone(),
            style: self.style,
            accessibility_label: self.accessibility_label,
        }
    }

    /// Starts building a button with environment extraction.
    ///
    /// The type parameter `E` specifies what to extract from the environment
    /// when the button is clicked. This provides explicit type information,
    /// avoiding type inference issues.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The type to extract, must implement [`Extractor`]
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Extract navigation controller
    /// button("Go Back")
    ///     .extract::<NavigationController>()
    ///     .action(|nav| nav.pop());
    ///
    /// // Chain multiple extractions
    /// button("Save")
    ///     .extract::<Database>()
    ///     .extract::<CurrentUser>()
    ///     .action(|(db, user)| db.save(user));
    /// ```
    #[must_use]
    pub fn extract<E: Extractor>(self) -> ButtonExtractBuilder<SemanticLabel, E> {
        ButtonExtractBuilder {
            label: self.label,
            style: self.style,
            accessibility_label: self.accessibility_label,
            _extract: PhantomData,
        }
    }
}

impl<Label: View, Action> Button<Label, Action> {
    /// Sets the visual style of the button.
    ///
    /// # Arguments
    ///
    /// * `style` - The [`ButtonStyle`] to apply
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// button("Submit")
    ///     .style(ButtonStyle::BorderedProminent)
    ///     .action(|| submit_form());
    /// ```
    #[must_use]
    pub const fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Overrides the semantic accessibility label announced by assistive technology.
    #[must_use]
    pub fn accessibility_label(mut self, label: impl IntoText) -> Self {
        self.accessibility_label = Some(label.into_text());
        self
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
    /// ```rust,ignore
    /// button("Click me").action(|| {
    ///     println!("Button was clicked!");
    /// });
    /// ```
    #[must_use]
    pub fn action<F>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut() + 'static,
    {
        Button {
            label: self.label,
            action: move |_env: &Environment| action(),
            style: self.style,
            accessibility_label: self.accessibility_label,
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
    /// ```rust,ignore
    /// button("Fetch").action_async(|| async {
    ///     let data = api::fetch_data().await;
    ///     process(data);
    /// });
    /// ```
    pub fn action_async<F, Fut>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut() -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        self.action(move || {
            spawn_local(action()).detach();
        })
    }
}

/// No-op action for buttons without a configured action.
fn noop(_env: &Environment) {}

fn semantic_menu_label<Label: View + 'static>(
    label: Label,
    accessibility_label: Option<Text>,
) -> SemanticLabel {
    let label = AnyView::new(label);
    if let Some(label) = label.downcast_ref::<SemanticLabel>() {
        return label.clone();
    }
    if let Some(accessibility_label) = accessibility_label {
        return SemanticLabel::new(accessibility_label);
    }
    panic!(
        "Buttons used inside Menu must either use a semantic Label or provide Button::accessibility_label(...)"
    );
}

impl<Label, Action> From<Button<Label, Action>> for crate::menu::Command
where
    Label: View + 'static,
    Action: FnMut(&Environment) + 'static,
{
    fn from(value: Button<Label, Action>) -> Self {
        let Button {
            label,
            action,
            style: _,
            accessibility_label,
        } = value;
        Self {
            label: semantic_menu_label(label, accessibility_label),
            action: shared_action_with_env(action),
            disabled: Computed::constant(false),
            selected: Computed::constant(false),
            shortcut: None,
        }
    }
}

impl<Label, Action> From<Button<Label, Action>> for crate::menu::MenuItem
where
    Label: View + 'static,
    Action: FnMut(&Environment) + 'static,
{
    fn from(value: Button<Label, Action>) -> Self {
        Self::Command(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::Signal;
    use waterui_core::view::{ConfigurableView, ViewConfiguration};

    #[test]
    fn menu_command_preserves_semantic_label_from_any_view_button() {
        let rendered = button(SemanticLabel::new("Edit").icon(waterui_icon::SystemIcon::SEARCH))
            .action(|| {})
            .config()
            .render();
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label
                .__text()
                .__resolve(&Environment::default())
                .content
                .get()
                .to_plain()
                .as_str(),
            "Edit"
        );
        let icon = command
            .label
            .__icon()
            .expect("semantic label icon should be preserved");
        assert_eq!(icon.name.as_str(), "magnifyingglass");
    }

    #[test]
    fn menu_command_falls_back_to_accessibility_label() {
        let rendered = ButtonConfig {
            label: AnyView::new(()),
            action: Box::new(|_| {}),
            style: ButtonStyle::Automatic,
            accessibility_label: Some(
                Text::new("Accessible")
                    .__resolve(&Environment::default())
                    .content,
            ),
        }
        .render();
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label
                .__text()
                .__resolve(&Environment::default())
                .content
                .get()
                .to_plain()
                .as_str(),
            "Accessible"
        );
        assert!(command.label.__icon().is_none());
    }

    #[test]
    #[should_panic(
        expected = "Buttons used inside Menu must either use a semantic Label or provide Button::accessibility_label(...)"
    )]
    fn menu_command_requires_semantic_label_or_accessibility_label() {
        let rendered = ButtonConfig {
            label: AnyView::new(()),
            action: Box::new(|_| {}),
            style: ButtonStyle::Automatic,
            accessibility_label: None,
        }
        .render();
        let _: crate::menu::Command = rendered.into();
    }
}

// ============================================================================
// ButtonBuilder - for buttons with state
// ============================================================================

/// A builder for creating buttons with captured state.
///
/// Created by calling [`Button::with_state`]. Chain multiple `with_state` calls
/// to accumulate state, then call [`action`](Self::action) to finalize.
///
/// # State Accumulation
///
/// Each `with_state` call wraps the previous state in a tuple:
/// - First call: `S`
/// - Second call: `(S, T)`
/// - Third call: `((S, T), U)`
///
/// # Example
///
/// ```rust,ignore
/// button("Update")
///     .with_state(&binding1)
///     .with_state(&binding2)
///     .action(|(b1, b2)| {
///         b1.set(b2.get());
///     });
/// ```
#[derive(Debug, Clone)]
pub struct ButtonBuilder<Label, State> {
    label: Label,
    state: State,
    style: ButtonStyle,
    accessibility_label: Option<Text>,
}

// Generate with_state for state chaining: S -> (S, T)
waterui_core::impl_stateful_builder!(ButtonBuilder<Label>; state; label, style, accessibility_label);

impl<Label: View, S: Clone + 'static> ButtonBuilder<Label, S> {
    /// Adds environment extraction to the button.
    ///
    /// After calling this, the action will receive a tuple of `(state, extracted)`.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The type to extract from the environment
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// button("Submit")
    ///     .with_state(&form_data)
    ///     .extract::<ApiClient>()
    ///     .action(|(data, client)| {
    ///         client.submit(data.get());
    ///     });
    /// ```
    #[must_use]
    pub fn extract<E: Extractor>(self) -> ButtonStateExtractBuilder<Label, S, E> {
        ButtonStateExtractBuilder {
            label: self.label,
            state: self.state,
            style: self.style,
            accessibility_label: self.accessibility_label,
            _extract: PhantomData,
        }
    }

    /// Sets the visual style of the button.
    #[must_use]
    pub const fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    impl_style_methods!();

    /// Sets the action to be performed when the button is clicked.
    ///
    /// The action receives the accumulated state as its argument. The state
    /// is cloned each time the button is clicked.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that receives the captured state
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// button("Toggle")
    ///     .with_state(&is_enabled)
    ///     .action(|enabled| enabled.set(!enabled.get()));
    /// ```
    #[must_use]
    pub fn action<F>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut(S) + 'static,
    {
        let state = self.state;
        Button {
            label: self.label,
            action: move |_env: &Environment| action(state.clone()),
            style: self.style,
            accessibility_label: self.accessibility_label,
        }
    }

    /// Sets an asynchronous action with access to captured state.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that receives state and returns a future
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// button("Save")
    ///     .with_state(&data)
    ///     .action_async(|data| async move {
    ///         api::save(data.get()).await;
    ///     });
    /// ```
    pub fn action_async<F, Fut>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut(S) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let state = self.state;
        Button {
            label: self.label,
            action: move |_env: &Environment| {
                spawn_local(action(state.clone())).detach();
            },
            style: self.style,
            accessibility_label: self.accessibility_label,
        }
    }
}

// ============================================================================
// ButtonExtractBuilder - for buttons with extraction only
// ============================================================================

/// A builder for creating buttons that extract values from the environment.
///
/// Created by calling [`Button::extract`]. The extraction type is specified
/// explicitly, providing clear type information.
///
/// # Example
///
/// ```rust,ignore
/// button("Navigate")
///     .extract::<NavigationController>()
///     .action(|nav| nav.push(DetailView::new()));
/// ```
#[derive(Debug, Clone)]
pub struct ButtonExtractBuilder<Label, Extract> {
    label: Label,
    style: ButtonStyle,
    accessibility_label: Option<Text>,
    _extract: PhantomData<Extract>,
}

impl<Label: View, E: Extractor> ButtonExtractBuilder<Label, E> {
    /// Chains another extraction type.
    ///
    /// The extracted values accumulate as nested tuples: `(first, second)`.
    ///
    /// # Type Parameters
    ///
    /// * `E2` - The additional type to extract
    #[must_use]
    pub fn extract<E2: Extractor>(self) -> ButtonExtractBuilder<Label, (E, E2)> {
        ButtonExtractBuilder {
            label: self.label,
            style: self.style,
            accessibility_label: self.accessibility_label,
            _extract: PhantomData,
        }
    }

    /// Adds state to the button.
    ///
    /// State will come first in the tuple, followed by extracted values:
    /// `(state, extracted)`.
    ///
    /// # Arguments
    ///
    /// * `state` - A reference to the value to capture
    #[must_use]
    pub fn with_state<T: Clone + 'static>(
        self,
        state: &T,
    ) -> ButtonStateExtractBuilder<Label, T, E> {
        ButtonStateExtractBuilder {
            label: self.label,
            state: state.clone(),
            style: self.style,
            accessibility_label: self.accessibility_label,
            _extract: PhantomData,
        }
    }

    /// Sets the visual style of the button.
    #[must_use]
    pub const fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    impl_style_methods!();

    /// Sets the action to be performed when the button is clicked.
    ///
    /// The action receives the extracted value as its argument. Extraction
    /// happens each time the button is clicked.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that receives the extracted value
    #[must_use]
    pub fn action<F>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut(E) + 'static,
    {
        Button {
            label: self.label,
            action: move |env: &Environment| {
                let extracted = E::extract(env).expect("failed to extract value from environment");
                action(extracted);
            },
            style: self.style,
            accessibility_label: self.accessibility_label,
        }
    }

    /// Sets an asynchronous action with environment extraction.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that receives extracted value and returns a future
    pub fn action_async<F, Fut>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut(E) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        Button {
            label: self.label,
            action: move |env: &Environment| {
                let extracted = E::extract(env).expect("failed to extract value from environment");
                spawn_local(action(extracted)).detach();
            },
            style: self.style,
            accessibility_label: self.accessibility_label,
        }
    }
}

// ============================================================================
// ButtonStateExtractBuilder - for buttons with both state and extraction
// ============================================================================

/// A builder for creating buttons with both captured state and environment extraction.
///
/// Created by calling [`ButtonBuilder::extract`] or [`ButtonExtractBuilder::with_state`].
/// The action receives a tuple of `(state, extracted)`.
///
/// # Example
///
/// ```rust,ignore
/// button("Submit Form")
///     .with_state(&form_data)
///     .extract::<ApiClient>()
///     .action(|(data, client)| {
///         client.submit(data.get());
///     });
/// ```
#[derive(Debug, Clone)]
pub struct ButtonStateExtractBuilder<Label, State, Extract> {
    label: Label,
    state: State,
    style: ButtonStyle,
    accessibility_label: Option<Text>,
    _extract: PhantomData<Extract>,
}

impl<Label: View, S: Clone + 'static, E: Extractor> ButtonStateExtractBuilder<Label, S, E> {
    /// Adds another state value.
    ///
    /// State values accumulate as nested tuples.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(
        self,
        state: &T,
    ) -> ButtonStateExtractBuilder<Label, (S, T), E> {
        ButtonStateExtractBuilder {
            label: self.label,
            state: (self.state, state.clone()),
            style: self.style,
            accessibility_label: self.accessibility_label,
            _extract: PhantomData,
        }
    }

    /// Chains another extraction type.
    ///
    /// Extracted values accumulate as nested tuples after the state.
    #[must_use]
    pub fn extract<E2: Extractor>(self) -> ButtonStateExtractBuilder<Label, S, (E, E2)> {
        ButtonStateExtractBuilder {
            label: self.label,
            state: self.state,
            style: self.style,
            accessibility_label: self.accessibility_label,
            _extract: PhantomData,
        }
    }

    /// Sets the visual style of the button.
    #[must_use]
    pub const fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    impl_style_methods!();

    /// Sets the action to be performed when the button is clicked.
    ///
    /// The action receives a tuple of `(state, extracted)`. State is cloned
    /// and extraction happens each time the button is clicked.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that receives `(state, extracted)`
    #[must_use]
    pub fn action<F>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut((S, E)) + 'static,
    {
        let state = self.state;
        Button {
            label: self.label,
            action: move |env: &Environment| {
                let extracted = E::extract(env).expect("failed to extract value from environment");
                action((state.clone(), extracted));
            },
            style: self.style,
            accessibility_label: self.accessibility_label,
        }
    }

    /// Sets an asynchronous action with both state and extraction.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that receives `(state, extracted)` and returns a future
    pub fn action_async<F, Fut>(self, mut action: F) -> Button<Label, impl FnMut(&Environment)>
    where
        F: FnMut((S, E)) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let state = self.state;
        Button {
            label: self.label,
            action: move |env: &Environment| {
                let extracted = E::extract(env).expect("failed to extract value from environment");
                spawn_local(action((state.clone(), extracted))).detach();
            },
            style: self.style,
            accessibility_label: self.accessibility_label,
        }
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
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// button("Click me").action(|| println!("Clicked!"));
/// ```
pub fn button(label: impl IntoLabel) -> Button<SemanticLabel, fn(&Environment)> {
    Button::new(label)
}
