//! The `code` leaf: `Native<CodeConfig>` realized as a native `AppKit`/`UIKit`
//! rounded surface container with a language caption header, a native Copy
//! button, and syntax-highlighted monospaced text that follows the installed
//! `Body` font slot.

use alloc::rc::Rc;
use core::fmt::{self, Debug, Formatter};

use cocoa_ui::geometry::Rect as KitRect;
use cocoa_ui::layer;
use cocoa_ui::{PlatformView, Retained, view};
use nami::SignalExt as _;
use waterui_controls::button::{ButtonStyle, button};
use waterui_controls::theme::color::{
    Accent, Border as BorderToken, MutedForeground, SurfaceVariant,
};
use waterui_controls::theme::{ForegroundOverride, install_font_signal};
use waterui_core::Environment;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::plugin::Plugin;
use waterui_core::resolve::Resolvable;
use waterui_graphics::color::{Color, WorkingColor};
use waterui_text::code::CodeConfig;
use waterui_text::font::{Body, Caption, Font};
use waterui_text::text;

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::proposal::{self, Placement};

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HostView, colors};
    pub(super) use cocoa_ui::objc2_app_kit::NSColor as PlatformColor;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor as PlatformColor;
    pub(super) use cocoa_ui::uikit::{HostView, colors};
}

use platform::HostView;

/// Continuous corner radius for code block containers, in points.
const CODE_CORNER_RADIUS: f64 = 12.0;
/// Inner content inset on all four edges of a code block, in points.
const CODE_PADDING: f32 = 14.0;
/// Vertical gap between the header row and the code body, in points.
const CODE_HEADER_SPACING: f32 = 8.0;
/// Minimum horizontal gap between the language label and Copy button, in points.
const CODE_HEADER_ITEM_GAP: f32 = 8.0;
/// Hairline outline stroke width for code block containers, in points.
const CODE_BORDER_WIDTH: f64 = 0.5;

fn platform_color(color: &WorkingColor) -> Retained<platform::PlatformColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

fn header_env(env: &Environment) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, Font::from(Caption).resolve(env));
    Plugin::install(
        ForegroundOverride::new(Color::new(MutedForeground)),
        &mut scoped,
    );
    scoped
}

fn copy_env(env: &Environment) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, Font::from(Caption).resolve(env));
    Plugin::install(ForegroundOverride::new(Color::new(Accent)), &mut scoped);
    scoped
}

fn body_env(env: &Environment) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, Font::from(Body).monospaced(true).resolve(env));
    scoped
}

fn apply_background_color(color: &WorkingColor, host: &PlatformView) {
    let native = platform_color(color);
    let cg = platform::colors::cg(&native);
    if let Some(host_layer) = layer::layer_of(host) {
        layer::set_background_color(&host_layer, Some(&cg));
    }
    view::invalidate_captured_rendering(host);
}

fn apply_border_color(color: &WorkingColor, host: &PlatformView) {
    let native = platform_color(color);
    let cg = platform::colors::cg(&native);
    if let Some(host_layer) = layer::layer_of(host) {
        layer::set_border_color(&host_layer, Some(&cg));
    }
    view::invalidate_captured_rendering(host);
}

struct CodeState {
    header: Mounted,
    copy_button: Mounted,
    body: Mounted,
}

impl CodeState {
    fn measure_inner(&self, proposal: ProposalSize) -> Size {
        let inner_w = proposal
            .width
            .filter(|w| w.is_finite())
            .map(|w| 2.0_f32.mul_add(-CODE_PADDING, w).max(0.0));
        let inner_h = proposal
            .height
            .filter(|h| h.is_finite())
            .map(|h| 2.0_f32.mul_add(-CODE_PADDING, h).max(0.0));

        let copy_size = self
            .copy_button
            .layout()
            .measure(ProposalSize::new(inner_w, None))
            .size;
        let header_max_w = inner_w.map(|w| (w - copy_size.width - CODE_HEADER_ITEM_GAP).max(0.0));
        let header_size = self
            .header
            .layout()
            .measure(ProposalSize::new(header_max_w, None))
            .size;

        let header_row_w = header_size.width + CODE_HEADER_ITEM_GAP + copy_size.width;
        let header_row_h = header_size.height.max(copy_size.height);

        let body_proposal = ProposalSize::new(
            inner_w,
            inner_h.map(|h| (h - header_row_h - CODE_HEADER_SPACING).max(0.0)),
        );
        let body_size = self.body.layout().measure(body_proposal).size;

        let natural_w = header_row_w.max(body_size.width);
        let width = inner_w.map_or(natural_w, |proposed| proposed.max(natural_w));
        let height = header_row_h + CODE_HEADER_SPACING + body_size.height;
        Size::new(
            2.0_f32.mul_add(CODE_PADDING, width),
            2.0_f32.mul_add(CODE_PADDING, height),
        )
    }

    fn layout_subviews(&self, host: &PlatformView, bounds: KitRect) {
        let rtl = view::is_right_to_left(host);
        let pad = f64::from(CODE_PADDING);
        let spacing = f64::from(CODE_HEADER_SPACING);
        let gap = f64::from(CODE_HEADER_ITEM_GAP);
        let content_left = bounds.origin.x + pad;
        let content_right = (bounds.origin.x + bounds.size.width - pad).max(content_left);
        let content_top = bounds.origin.y + pad;
        let content_bottom = (bounds.origin.y + bounds.size.height - pad).max(content_top);
        let content_width = (content_right - content_left).max(0.0);

        #[expect(
            clippy::cast_possible_truncation,
            reason = "layout coordinates fit f32"
        )]
        let content_width_f32 = content_width as f32;

        let copy_size = self
            .copy_button
            .layout()
            .measure(ProposalSize::new(Some(content_width_f32), None))
            .size;
        let header_avail_w = (content_width - f64::from(copy_size.width) - gap).max(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "layout coordinates fit f32"
        )]
        let header_avail_w_f32 = header_avail_w as f32;
        let header_size = self
            .header
            .layout()
            .measure(ProposalSize::new(Some(header_avail_w_f32), None))
            .size;

        let avail_h = (content_bottom - content_top).max(0.0);
        let header_row_h = f64::from(header_size.height.max(copy_size.height)).min(avail_h);

        let place_at = |mounted: &Mounted, left: f64, top: f64, width: f64, height: f64| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "layout coordinates fit f32"
            )]
            let (w_f32, h_f32) = (width as f32, height as f32);
            proposal::deliver(
                mounted.view(),
                Placement::platform_hosted(ProposalSize::new(Some(w_f32), Some(h_f32))),
            );
            view::set_frame(mounted.view(), KitRect::new(left, top, width, height));
        };

        let header_w = f64::from(header_size.width).min(header_avail_w);
        let header_h = f64::from(header_size.height).min(header_row_h);
        let header_y = (header_row_h - header_h).mul_add(0.5, content_top);
        let header_x = if rtl {
            content_right - header_w
        } else {
            content_left
        };
        place_at(&self.header, header_x, header_y, header_w, header_h);

        let copy_w = f64::from(copy_size.width).min(content_width);
        let copy_h = f64::from(copy_size.height).min(header_row_h);
        let copy_y = (header_row_h - copy_h).mul_add(0.5, content_top);
        let copy_x = if rtl {
            content_left
        } else {
            content_right - copy_w
        };
        place_at(&self.copy_button, copy_x, copy_y, copy_w, copy_h);

        let body_top = (content_top + header_row_h + spacing).min(content_bottom);
        let rem_h = (content_bottom - body_top).max(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "layout coordinates fit f32"
        )]
        let rem_h_f32 = rem_h as f32;
        let body_size = self
            .body
            .layout()
            .measure(ProposalSize::new(Some(content_width_f32), Some(rem_h_f32)))
            .size;
        let body_w = f64::from(body_size.width).min(content_width);
        let body_h = f64::from(body_size.height).min(rem_h);
        let body_x = if rtl {
            content_right - body_w
        } else {
            content_left
        };
        place_at(&self.body, body_x, body_top, body_w, body_h);
    }
}

struct CodeSubView {
    state: Rc<CodeState>,
}

impl Debug for CodeSubView {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeSubView").finish_non_exhaustive()
    }
}

impl SubView for CodeSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(self.state.measure_inner(proposal))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Realizes `CodeConfig` as a native rounded code block surface on `AppKit` and
/// `UIKit`, with a language caption header, native Copy button, and monospaced
/// `Body`-scaled syntax-highlighted source text.
#[expect(
    clippy::needless_pass_by_value,
    reason = "NativeBackend hands each payload by value"
)]
pub fn realize_code_config(config: CodeConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, KitRect::ZERO);
    let host_view: &PlatformView = &host;
    layer::ensure_layer(host_view);

    if let Some(host_layer) = layer::layer_of(host_view) {
        layer::set_corner_radius(&host_layer, CODE_CORNER_RADIUS);
        layer::set_continuous_corner_curve(&host_layer);
        layer::set_masks_to_bounds(&host_layer, false);
        layer::set_border(&host_layer, CODE_BORDER_WIDTH, CODE_CORNER_RADIUS, false);
    }

    let scoped_header_env = header_env(ctx.env());
    let mounted_header = ctx
        .with_env(&scoped_header_env)
        .render(text(config.header_label()).bold(true))
        .mount(host_view);

    let scoped_copy_env = copy_env(ctx.env());
    let copy_config = config.clone();
    let mounted_copy =
        ctx.with_env(&scoped_copy_env)
            .render(button("Copy").style(ButtonStyle::Borderless).action(
                move |env: Environment| {
                    copy_config.copy_to_clipboard(&env);
                },
            ))
            .mount(host_view);

    let scoped_body_env = body_env(ctx.env());
    let highlighted = config.highlighted(ctx.env());
    let mounted_body = ctx
        .with_env(&scoped_body_env)
        .render(text(highlighted))
        .mount(host_view);

    let state = Rc::new(CodeState {
        header: mounted_header,
        copy_button: mounted_copy,
        body: mounted_body,
    });

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host_view| {
            let bounds = view::bounds(host_view);
            state.layout_subviews(host_view, bounds);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        CodeSubView {
            state: Rc::clone(&state),
        },
    );

    let fill_signal = SurfaceVariant.resolve(ctx.env()).computed();
    leaf.bind(&fill_signal, {
        let host = Retained::clone(&host);
        move |color| apply_background_color(&color, &host)
    });

    let border_signal = BorderToken.resolve(ctx.env()).computed();
    leaf.bind(&border_signal, {
        let host = Retained::clone(&host);
        move |color| apply_border_color(&color, &host)
    });

    leaf.keep(scoped_header_env);
    leaf.keep(scoped_copy_env);
    leaf.keep(scoped_body_env);
    leaf.keep(state);
    leaf
}
