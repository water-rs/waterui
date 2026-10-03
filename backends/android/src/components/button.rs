//! `Native<ButtonConfig>` — a `FrameLayout` shell that carries the
//! theme's button chrome directly and hosts the `WaterUI` label as its
//! child.
//!
//! `?attr/buttonStyle` resolves at runtime to the style a platform
//! `Button` would use; its `background` (shape and press ripple) and
//! `stateListAnimator` (the pressed lift) apply to the shell, so the
//! rendered `Label` subtree — a child of the raised view — draws above
//! the chrome by construction, no elevation arithmetic against an
//! animated `translationZ`. The inset is the background drawable's own
//! `getPadding`, read through the same `TypedArray`.
//!
//! `ButtonStyle` shares the `UIKit` color table: prominent styles draw
//! `AccentForeground` on the accent chrome; `plain`/glass draw plain
//! `Foreground`; the rest draw `Accent` text. Borderless chrome itself
//! (`?attr/borderlessButtonStyle`) is a platform style the follow-up port
//! applies — the skeleton keeps the one chrome and notes the split.

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
        let handler =
            unsafe { &*crate::handle::jlong_to_pointer::<ClickHandler>(handle).cast_const() };
        handler.action.borrow_mut()(&handler.env);
        Ok(())
    });
    outcome.resolve::<crate::policy::ThrowRuntimeExAndDefault>();
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
    /// `(horizontal, vertical)` total chrome inset in dp — the
    /// background drawable's own padding the style declares.
    inset: (f32, f32),
}

impl SubView for ButtonSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let (horizontal, vertical) = self.inset;
        let offer = ProposalSize::new(
            proposal.width.map(|w| (w - horizontal).max(0.0)),
            proposal.height.map(|h| (h - vertical).max(0.0)),
        );
        let measured = self.child.layout().measure(offer);
        ViewDimensions::new(Size::new(
            horizontal + measured.size.width,
            vertical + measured.size.height,
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

/// Builds the button: a `FrameLayout` shell that carries the theme's
/// `?attr/buttonStyle` background and `stateListAnimator` itself, holding
/// the `FrameLayout` that hosts the rendered label. Answers
/// `(shell, label_container, chrome padding px)`.
fn assemble_shell(platform: &Platform) -> (PlatformView, PlatformView, [jint; 4]) {
    jvm::with_env(|env| {
        let bindings = platform.bindings();
        let shell = platform
            .new_frame_layout(env)
            .expect("a FrameLayout constructs against the host context");
        let label_container = platform
            .new_frame_layout(env)
            .expect("a FrameLayout constructs against the host context");
        // The raised chrome is the shell's own background and animator —
        // what a `Button` built in this theme would have drawn — so the
        // label child draws above it by construction.
        let padding: [jint; 4] = platform
            .install_button_chrome(env, shell.as_ref())
            .expect("the theme must resolve ?attr/buttonStyle to a style");
        let shell = jvm::retain(&shell);
        let label_container = jvm::retain(&label_container);
        // `View.setBackground` applies the drawable's own `getPadding` as
        // the shell's view padding, which `FrameLayout` already subtracts
        // for a match-parent child — a plain fill puts the label exactly
        // inside the chrome's content area. Setting the same padding as
        // margins on top would double the inset and collapse the label.
        fill_parent(env, &label_container, platform);
        bindings
            .set_clickable(env, label_container.as_ref(), false)
            .and_then(|()| {
                bindings.set_important_for_accessibility(
                    env,
                    label_container.as_ref(),
                    bindings.important_for_accessibility_no_hide_descendants(),
                )
            })
            .and_then(|()| {
                bindings.set_important_for_accessibility(
                    env,
                    shell.as_ref(),
                    bindings.important_for_accessibility_yes(),
                )
            })
            // The shell is the one interactive element: keyboard focus
            // and the click target are its, the label subtree neither.
            .and_then(|()| bindings.set_focusable(env, shell.as_ref(), true))
            .and_then(|()| bindings.add_view(env, shell.as_ref(), label_container.as_ref()))
            .expect("button shell assembly must not throw");
        (shell, label_container, padding)
    })
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

        let (shell, label_container, chrome_padding_px) = assemble_shell(platform);

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
                inset: (
                    platform.px_to_dp(chrome_padding_px[0] + chrome_padding_px[2]),
                    platform.px_to_dp(chrome_padding_px[1] + chrome_padding_px[3]),
                ),
            },
            platform,
        );
        leaf.keep(label_env);
        leaf.keep(label_container);

        let target = jvm::retain(&shell);
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
                .new_click_listener(env, crate::handle::pointer_to_jlong(handler_ptr))
                .expect("a RustOnClickListener constructs");
            bindings
                .set_on_click_listener(env, shell.as_ref(), Some(listener.as_ref()))
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
