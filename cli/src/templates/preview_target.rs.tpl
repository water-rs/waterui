//! Generated {{ backend_label }} preview target for `{{ crate_name_ident }}`.

{% if expression_mode %}
pub(crate) fn load_preview_view() -> waterui::AnyView {
    use {{ crate_name_ident }}::*;
    use waterui::prelude::*;
    use waterui::prelude::picker::picker;
    use waterui as waterui;
    use waterui::binding;

    let view = { {{ preview_expression }} };
    waterui::AnyView::new(view)
}
{% else %}
fn ensure_preview_crate_is_linked() {
    let _: fn(waterui::env::Environment) -> _ = {{ crate_name_ident }}::app;
}

unsafe extern "C" {
    #[link_name = "{{ preview_symbol }}"]
    fn {{ extern_fn_name }}() -> *mut ();
}

pub(crate) fn load_preview_view() -> waterui::AnyView {
    ensure_preview_crate_is_linked();
    let ptr = unsafe { {{ extern_fn_name }}() };
    let boxed: Box<waterui::AnyView> = unsafe { Box::from_raw(ptr.cast()) };
    *boxed
}
{% endif %}
{% if let Some(style) = preview_theme_style %}
/// The style the preview runtimes are constructed with — the value `main`
/// hands to `hydrolysis::run(app, env, style)`.
pub(crate) fn preview_style(
    color_scheme: Option<waterui::theme::ColorScheme>,
) -> impl hydrolysis::Style {
    {{ style }}
}

/// The environment previews resolve under.
///
/// This goes through the application's own composition root — `app(env)` is
/// what installs the realizations and options the application configures
/// (`waterui_map_gpu::MapGpu`, provider options, fonts) — so a preview sees
/// the same environment a run would.
pub(crate) fn app_environment() -> waterui::env::Environment {
    let env = waterui::configure_environment!(waterui::env::Environment::new());
    {{ crate_name_ident }}::app(env).into_parts().env
}
{% endif -%}
{% if let Some(body) = semantic_automation_body %}
{% if offscreen_semantic_test %}
pub(crate) fn run_preview_test(
    env: waterui::env::Environment,
    width: u32,
    height: u32,
    scale: f64,
    color_scheme: Option<waterui::theme::ColorScheme>,
    video: Option<&waterui_preview_protocol::run::VideoCaptureConfig>,
    hdr: bool,
) {
    use std::cell::RefCell;
    use std::rc::Rc;

    use waterui_preview_protocol::run::{Alpha, RawFrameStreamWriter};
    use waterui_testing::ui;

    let video = video.expect("offscreen preview test requires video capture configuration");
    let mut app = ui()
        .environment(env)
        .theme(preview_style(color_scheme))
        .viewport(width, height)
        .scale_factor(scale)
        .hdr(hdr)
        .mount_offscreen(load_preview_view);
    let writer = Rc::new(RefCell::new(Some(
        if hdr {
            RawFrameStreamWriter::create_hdr(video.output(), video.fps())
        } else {
            RawFrameStreamWriter::create(video.output(), video.fps())
        }
        .unwrap_or_else(|error| panic!("hydrolysis preview test: {error}")),
    )));
    let sink_writer = Rc::clone(&writer);
    if hdr {
        app.start_hdr_frame_recording(video.fps(), move |snapshot, timestamp| {
            let (frame_w, frame_h, rgba_f32) = snapshot.into_parts();
            if let Some(writer) = sink_writer.borrow_mut().as_mut() {
                writer
                    .write_hdr_frame_f32(frame_w, frame_h, &rgba_f32, Alpha::Straight, timestamp)
                    .unwrap_or_else(|error| panic!("hydrolysis preview test: {error}"));
            }
        });
    } else {
        app.start_frame_recording(video.fps(), move |snapshot, timestamp| {
            let (frame_w, frame_h, rgba8) = snapshot.into_parts();
            if let Some(writer) = sink_writer.borrow_mut().as_mut() {
                writer
                    .write_frame(frame_w, frame_h, rgba8, Alpha::Straight, timestamp)
                    .unwrap_or_else(|error| panic!("hydrolysis preview test: {error}"));
            }
        });
    }
    run_semantic_automation(&mut app);
    app.finish_frame_recording();
    if let Some(writer) = writer.borrow_mut().take() {
        let _ = writer
            .finish()
            .unwrap_or_else(|error| panic!("hydrolysis preview test: {error}"));
    }
}

pub(crate) fn run_semantic_automation(app: &mut waterui_testing::OffscreenApp) {
    use waterui::prelude::*;
    use waterui_testing::*;

    {{ body }}
}
{% else %}
pub(crate) fn run_preview_test(
    env: waterui::env::Environment,
    width: u32,
    height: u32,
    _scale: f64,
    color_scheme: Option<waterui::theme::ColorScheme>,
    video: Option<&waterui_preview_protocol::run::VideoCaptureConfig>,
    _hdr: bool,
) {
    use waterui_testing::ui;

    assert!(
        video.is_none(),
        "hydrolysis preview test: video capture requires offscreen semantic test bindings"
    );
    let mut app = ui()
        .environment(env)
        .theme(preview_style(color_scheme))
        .viewport(width, height)
        .mount(load_preview_view);
    run_semantic_automation(&mut app);
}

pub(crate) fn run_semantic_automation(app: &mut waterui_testing::SemanticApp) {
    use waterui::prelude::*;
    use waterui_testing::*;

    {{ body }}
}
{% endif %}
{% endif %}