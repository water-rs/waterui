//! `Native<ButtonConfig>` — a platform `android.widget.Button` carrying a
//! `WaterUI` label laid out by Rust.
//!
//! The shell is a `FrameLayout` holding two siblings: the `Button` (the
//! platform chrome — ripple, pressed feedback, enabled state) filling it,
//! and a second `FrameLayout` hosting the rendered `Label` subtree on top,
//! inset by the chrome's padding. The label view is marked not-clickable so
//! touches land on the chrome.
//!
//! `ButtonStyle` shares the `UIKit` color table: prominent styles draw
//! `AccentForeground` on the accent chrome; `plain`/glass draw plain
//! `Foreground`; the rest draw `Accent` text. Borderless chrome itself
//! (`?attr/borderlessButtonStyle`) is a platform style the follow-up port
//! applies — the skeleton keeps the one `Button` and notes the split.

use alloc::boxed::Box;
use core::cell::RefCell;

use jni::objects::{Global, JObject, JString};
use jni::sys::{jint, jlong};
use waterui::component::button::{ButtonConfig, ButtonStyle};
use waterui::graphics::color::WorkingColor;
use waterui::reactive::{Computed, SignalExt};
use waterui::theme::color::{Accent, AccentForeground, Foreground};
use waterui::theme::install_color_signal;
use waterui_backend_core::{AnyView, Environment};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_core::resolve::Resolvable;

use crate::contract::{Mounted, NativeLeaf, PlatformView};
use crate::dispatch::Dispatcher;
use crate::jvm::{self, Platform};

/// `View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS` — the chrome
/// speaks its own content description; the label subtree must not also
/// talk.
const IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS: i32 = 8;

/// `View.IMPORTANT_FOR_ACCESSIBILITY_YES` — the chrome is the accessible
/// element, regardless of what its text would imply.
const IMPORTANT_FOR_ACCESSIBILITY_YES: i32 = 1;

/// Material-button inset: the room the chrome leaves its label — 8dp sides,
/// 4dp vertical, the compact face a default `Button` draws.
const CHROME_PADDING: (f32, f32) = (8.0, 4.0);

/// The state behind `RustOnClickListener.nativeOnClick`: the action and the
/// environment it runs against, owned by the leaf through [`ClickHandle`].
struct ClickHandler {
    action: RefCell<waterui_core::handler::BoxedAction>,
    env: Environment,
}

/// `RustOnClickListener.onClick`: run the action against the button's own
/// environment — the one `Button<Action>` extracted state into.
///
/// # Safety
///
/// `handle` is the `ClickHandler` the listener was constructed with, live
/// for the listener's — and its leaf's — life.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_RustOnClickListener_nativeOnClick<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _this: JObject<'caller>,
    handle: jlong,
) {
    let outcome = unowned_env.with_env(|_env| -> jni::errors::Result<()> {
        // SAFETY: the handler is freed only when the leaf that mounted the
        // button drops; a detached button cannot be clicked.
        let handler = unsafe {
            &*(usize::try_from(handle).expect("a click handle is a `Box::into_raw` pointer")
                as *const ClickHandler)
        };
        handler.action.borrow_mut()(&handler.env);
        Ok(())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// The `UIKit` color table, minus liquid glass: prominent styles read
/// `AccentForeground` on accent chrome; plain and glass read `Foreground`;
/// the rest read `Accent`.
fn label_foreground(style: ButtonStyle, env: &Environment) -> Computed<WorkingColor> {
    match style {
        ButtonStyle::BorderedProminent | ButtonStyle::GlassProminent => {
            AccentForeground.resolve(env).computed()
        }
        ButtonStyle::Plain | ButtonStyle::Glass => Foreground.resolve(env).computed(),
        ButtonStyle::Automatic
        | ButtonStyle::Link
        | ButtonStyle::Borderless
        | ButtonStyle::Bordered => Accent.resolve(env).computed(),
        _ => panic!("unsupported WaterUI button style: {style:?}"),
    }
}

/// The button's `SubView`: measure the label under the offer minus the
/// chrome padding, then add the padding back.
struct ButtonSubView {
    child: Mounted,
    /// `(horizontal, vertical)` chrome padding in dp.
    padding: (f32, f32),
}

impl SubView for ButtonSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let (horizontal, vertical) = self.padding;
        let offer = ProposalSize::new(
            proposal
                .width
                .map(|w| f32::mul_add(horizontal, -2.0, w).max(0.0)),
            proposal
                .height
                .map(|h| f32::mul_add(vertical, -2.0, h).max(0.0)),
        );
        let measured = self.child.layout().measure(offer);
        ViewDimensions::new(Size::new(
            horizontal.mul_add(2.0, measured.size.width),
            vertical.mul_add(2.0, measured.size.height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// `setLayoutParams(MATCH_PARENT, MATCH_PARENT)` — a `FrameLayout` child
/// that fills its parent.
fn fill_parent(env: &mut jni::Env, view: &PlatformView, platform: &Platform) {
    let bindings = platform.bindings();
    let params = bindings
        .new_match_parent_params(env)
        .expect("LayoutParams construct");
    bindings
        .set_layout_params(env, view.as_ref(), params.as_ref())
        .expect("setLayoutParams must not throw");
}

/// `setLayoutParams(MATCH_PARENT, MATCH_PARENT)` with `setMargins` — the
/// label container's place inside the chrome's padding, in px.
fn fill_parent_inset(env: &mut jni::Env, view: &PlatformView, platform: &Platform, margin: jint) {
    let bindings = platform.bindings();
    let params = bindings
        .new_match_parent_params(env)
        .expect("LayoutParams construct");
    bindings
        .set_margins(env, params.as_ref(), margin, margin, margin, margin)
        .and_then(|()| bindings.set_layout_params(env, view.as_ref(), params.as_ref()))
        .expect("LayoutParams writes must not throw");
}

/// Claims `Native<ButtonConfig>`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ButtonConfig>(|config, ctx| {
        let style = config.style;
        let platform = ctx.platform();

        // The label subtree resolves `Foreground` from the color the style
        // prescribes.
        let mut label_env = ctx.env().clone();
        install_color_signal::<Foreground>(&mut label_env, label_foreground(style, ctx.env()));

        let (shell, button, label_container) = jvm::with_env(|env| {
            let bindings = platform.bindings();
            let shell = platform
                .new_frame_layout(env)
                .expect("a FrameLayout constructs against the host context");
            let button = platform
                .new_button(env)
                .expect("a Button constructs against the host context");
            let label_container = platform
                .new_frame_layout(env)
                .expect("a FrameLayout constructs against the host context");
            let shell = jvm::retain(&shell);
            let button = jvm::retain(&button);
            let label_container = jvm::retain(&label_container);
            // The chrome fills the shell; the label container fills it minus
            // the chrome's padding — `FrameLayout` honors margins on
            // match-parent children.
            fill_parent(env, &button, platform);
            let margin = platform.dp_to_px(CHROME_PADDING.0.max(CHROME_PADDING.1));
            fill_parent_inset(env, &label_container, platform, margin);
            bindings
                .set_clickable(env, label_container.as_ref(), false)
                .and_then(|()| {
                    bindings.set_important_for_accessibility(
                        env,
                        label_container.as_ref(),
                        IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS,
                    )
                })
                .and_then(|()| {
                    bindings.set_important_for_accessibility(
                        env,
                        button.as_ref(),
                        IMPORTANT_FOR_ACCESSIBILITY_YES,
                    )
                })
                .and_then(|()| bindings.add_view(env, shell.as_ref(), button.as_ref()))
                .and_then(|()| bindings.add_view(env, shell.as_ref(), label_container.as_ref()))
                .expect("button shell assembly must not throw");
            (shell, button, label_container)
        });

        // The spoken label: `Label`'s own accessibility text or its
        // content's plain string, tracked reactively — resolved before the
        // label moves into the rendered leaf.
        let accessibility = config.label.accessibility_label();

        let label_leaf = ctx.with_env(&label_env).render(AnyView::new(config.label));
        let mounted = label_leaf.mount(&label_container);
        jvm::with_env(|env| fill_parent(env, mounted.view(), platform));

        let mut leaf = NativeLeaf::new(
            jvm::retain(&shell),
            ButtonSubView {
                child: mounted,
                padding: CHROME_PADDING,
            },
            platform,
        );
        leaf.keep(label_env);
        leaf.keep(ButtonChrome {
            _shell: shell,
            _button: jvm::retain(&button),
            _label_container: label_container,
        });

        let target = jvm::retain(&button);
        let accessibility_platform = platform.clone();
        leaf.bind(&accessibility, move |styled| {
            let text = styled.to_plain();
            jvm::with_env(|env| {
                let value = JString::from_str(env, text.as_str())
                    .expect("a JString allocation is infallible");
                accessibility_platform
                    .bindings()
                    .set_content_description(env, target.as_ref(), &value)
                    .expect("setContentDescription must not throw");
            });
        });

        // The click listener: the platform object holds the handler by
        // `jlong`; the leaf owns the handler behind it.
        let handler = Box::new(ClickHandler {
            action: RefCell::new(config.action),
            env: ctx.env().clone(),
        });
        let handler_ptr = Box::into_raw(handler);
        let listener = jvm::with_env(|env| {
            let bindings = platform.bindings();
            let listener = bindings
                .new_click_listener(env, handler_ptr as jlong)
                .expect("a RustOnClickListener constructs");
            bindings
                .set_on_click_listener(env, button.as_ref(), Some(listener.as_ref()))
                .expect("setOnClickListener must not throw");
            listener
        });
        leaf.keep(ClickHandle {
            _listener: listener,
            handler_ptr,
        });
        leaf
    });
}

/// The shell and its pieces — kept so the assembly drops in leaf order and
/// `Global`s release evenly.
struct ButtonChrome {
    _shell: PlatformView,
    _button: PlatformView,
    _label_container: PlatformView,
}

/// The click path: the platform listener plus the raw handler behind it.
struct ClickHandle {
    _listener: Global<JObject<'static>>,
    handler_ptr: *mut ClickHandler,
}

impl Drop for ClickHandle {
    fn drop(&mut self) {
        // SAFETY: minted from `Box::into_raw` above; freed exactly once here.
        drop(unsafe { Box::from_raw(self.handler_ptr) });
    }
}
