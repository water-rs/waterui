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
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! button("Click me").action(|| {});
//! ```
//!
//! ## State Injection
//!
//! Use `.state()` on the resulting view to inject cloneable state into the
//! environment, then extract it inside `action(...)` with `State<T>`:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! let counter: Binding<i32> = binding(0);
//!
//! button("Increment")
//!     .action(|State(count): State<Binding<i32>>| {
//!         *count.get_mut() += 1;
//!     })
//!     .state(&counter);
//!
//! button("Go")
//!     .action(
//!         |State(wv): State<WebView>, State(addr): State<Binding<Url>>| {
//!         wv.go_to(addr.get().as_str());
//!         },
//!     )
//!     .state(&webview)
//!     .state(&address);
//! ```
//!
//! ## Environment Extraction
//!
//! Any extractor type can be used directly as an `action(...)` parameter:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! button("Apply Theme")
//!     .action(|theme: Theme| {
//!         apply_theme(theme);
//!     });
//!
//! button("Save")
//!     .action(|db: DatabaseConnection, user: CurrentUser| {
//!         db.save_user_preferences(user);
//!     });
//! ```
//!
//! ## Combined State and Extraction
//!
//! State and custom extractors compose in a single handler:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! button("Submit")
//!     .action(|State(data): State<FormData>, client: ApiClient| {
//!         client.submit(data.get());
//!     })
//!     .state(&form_data);
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
//!     .action_async(|State(result): State<ResultBinding>| async move {
//!         let data = fetch_from_server().await;
//!         result.set(data);
//!     })
//!     .state(&result_binding);
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

use core::fmt;
use core::future::Future;
use fmt::Debug;

use alloc::boxed::Box;
use executor_core::spawn_local;
use waterui_core::handler::{BoxedAction, Handler};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, Native, NativeView, View, impl_debug};
use waterui_text::IntoText;

use crate::label::{IntoLabel, Label, LabelDisplayMode};

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

/// How large a button is drawn.
///
/// Material 3 Expressive scales a button through five sizes that change height,
/// padding, icon size and corner shape together — not just its height — so the
/// size is a semantic choice, like the style, rather than a frame the caller
/// imposes from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ButtonSize {
    /// The most compact button, for dense rows of actions.
    ExtraSmall,
    /// The default size.
    #[default]
    Small,
    /// A roomier button for a screen's main action.
    Medium,
    /// A prominent button, large enough to anchor a section.
    Large,
    /// The largest button, for hero actions on big surfaces.
    ExtraLarge,
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
/// It is produced by [`ConfigurableView::config`] and consumed by the native backend.
///
/// # Layout Behavior
///
/// Buttons size themselves to fit their label content and never stretch to fill
/// extra space. In a stack, they take only the space they need.
#[non_exhaustive]
pub struct ButtonConfig {
    /// The semantic label displayed on the button.
    ///
    /// [`Label`] carries both the visual chrome (text + icon + display mode)
    /// and the spoken text used by assistive technology. Hooks receiving a
    /// `ButtonConfig` get a typed `Label`, so they can wrap or restyle it but
    /// cannot strip away the semantic identity.
    pub label: Label,
    /// The action to execute when the button is clicked.
    pub action: BoxedAction<()>,
    /// The visual style of the button.
    pub style: ButtonStyle,
    /// How large the button is drawn.
    pub size: ButtonSize,
}

impl_debug!(ButtonConfig);

impl NativeView for ButtonConfig {}

impl ButtonConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        // `Automatic` means "whatever this context calls for". A surrounding
        // scope can say what that is — a list row wants row-style buttons, not
        // the filled containers a screen's primary action gets — while an
        // explicit style on the button still wins.
        if matches!(self.style, ButtonStyle::Automatic)
            && let Some(ambient) = env.get::<ButtonStyle>()
        {
            self.style = *ambient;
        }
        self
    }
}

fn render_button_config<Action>(button: Button<Action>) -> ButtonConfig
where
    Action: FnMut(&Environment) + 'static,
{
    ButtonConfig {
        // A button label stays one truncated line — the SwiftUI and Material
        // baseline — instead of folding into a paragraph under compression.
        // Applied here so themed hooks and native backends agree, while an
        // explicit `Text::line_limit` from the application still wins.
        label: button
            .label
            .default_text_line_limit(core::num::NonZeroUsize::MIN),
        action: Box::new(button.action),
        style: button.style,
        size: button.size,
    }
}

impl<Action> View for Button<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let config = render_button_config(self);
        if let Some(hook) = env.get::<Hook<ButtonConfig>>() {
            AnyView::new(hook.apply(env, config))
        } else {
            AnyView::new(Native::new(config.resolve(env)))
        }
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

impl ViewConfiguration for ButtonConfig {
    type View = Button<BoxedAction<()>>;

    fn render(self) -> Self::View {
        Button {
            label: self.label,
            action: self.action,
            style: self.style,
            size: self.size,
        }
    }
}

impl<Action> ConfigurableView for Button<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    type Config = ButtonConfig;

    fn config(self) -> Self::Config {
        ButtonConfig {
            label: self.label,
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
pub struct Button<Action> {
    label: Label,
    action: Action,
    style: ButtonStyle,
    size: ButtonSize,
}

impl<Action> Button<Action> {
    /// Returns the semantic label displayed by this button.
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// Returns the visual style requested for this button.
    #[must_use]
    pub const fn button_style(&self) -> ButtonStyle {
        self.style
    }

    /// Returns the size this button is drawn at.
    #[must_use]
    pub const fn button_size(&self) -> ButtonSize {
        self.size
    }

    /// Sets how large the button is drawn.
    #[must_use]
    pub const fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }
}

impl<Action> Debug for Button<Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Button")
            .field("label", &self.label)
            .field("style", &self.style)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<Action: Clone> Clone for Button<Action> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            action: self.action.clone(),
            style: self.style,
            size: self.size,
        }
    }
}

impl Button<fn(&Environment)> {
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
    /// ```rust,ignore
    /// let btn = Button::new(label("Click me")).action(|| {});
    /// ```
    #[must_use]
    pub fn new(label: Label) -> Self {
        Self {
            label,
            action: noop,
            style: ButtonStyle::Automatic,
            size: ButtonSize::default(),
        }
    }
}

impl<Action> Button<Action> {
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
    /// ```rust,ignore
    /// button("Click me").action(|| {});
    /// ```
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> Button<impl FnMut(&Environment)>
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
    /// ```rust,ignore
    /// button("Fetch").action_async(|| async {
    ///     let data = api::fetch_data().await;
    ///     process(data);
    /// });
    /// ```
    pub fn action_async<F, Fut, Args>(self, action: F) -> Button<impl FnMut(&Environment)>
    where
        F: Handler<Args, Fut> + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let mut action = action;
        self.action(move |env: Environment| {
            spawn_local(action.call(&env)).detach();
        })
    }
}

/// No-op action for buttons without a configured action.
const fn noop(_env: &Environment) {}

impl<Action> From<Button<Action>> for crate::menu::Command
where
    Action: FnMut(&Environment) + 'static,
{
    fn from(value: Button<Action>) -> Self {
        let Button {
            label,
            mut action,
            style: _,
            size: _,
        } = value;
        Self::builder(label).action(move |env: Environment| action(&env))
    }
}

impl<Action> From<Button<Action>> for crate::menu::MenuItem
where
    Action: FnMut(&Environment) + 'static,
{
    fn from(value: Button<Action>) -> Self {
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
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// button("Click me").action(|| {});
/// ```
pub fn button(label: impl IntoLabel) -> Button<fn(&Environment)> {
    Button::new(label.into_label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::Signal;
    use waterui_core::view::{ConfigurableView, ViewConfiguration};

    #[test]
    fn menu_command_preserves_label_through_config_render_roundtrip() {
        crate::init_test_executor();
        let rendered =
            button(crate::label::label("Edit").icon(waterui_icon::system_icon::search()))
                .action(|| {})
                .config()
                .render();
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label
                .semantic_text()
                .resolve(&Environment::default())
                .content
                .get()
                .to_plain()
                .as_str(),
            "Edit"
        );
        let icon = command
            .label
            .semantic_icon()
            .expect("semantic label icon should be preserved");
        assert_eq!(icon.name.as_str(), "magnifyingglass");
    }

    #[test]
    fn accessibility_label_overrides_text_only() {
        crate::init_test_executor();
        // Calling .accessibility_label("X") must rewrite the spoken text
        // while preserving the visible text and icon attached at construction time.
        let rendered =
            button(crate::label::label("Edit").icon(waterui_icon::system_icon::search()))
                .accessibility_label("Edit document")
                .action(|| {})
                .config()
                .render();
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label
                .semantic_text()
                .resolve(&Environment::default())
                .content
                .get()
                .to_plain()
                .as_str(),
            "Edit"
        );
        assert_eq!(
            command
                .label
                .clone()
                .resolve(&Environment::default())
                .accessibility_label()
                .get()
                .to_plain()
                .as_str(),
            "Edit document"
        );
        let icon = command
            .label
            .semantic_icon()
            .expect("icon should be preserved when only the spoken text is overridden");
        assert_eq!(icon.name.as_str(), "magnifyingglass");
    }
}
