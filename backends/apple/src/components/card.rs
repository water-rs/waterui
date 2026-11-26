//! The `card` leaf: `Native<CardConfig>` realized as a native `AppKit`/`UIKit`
//! rounded container surface with Core Animation's continuous corner curve,
//! reactive semantic theme tokens (`Surface`, `SurfaceVariant`, `Border`),
//! subtle elevation for [`CardStyle::Elevated`], hairline outline for
//! [`CardStyle::Outlined`] and [`CardStyle::Automatic`], and native typography
//! for optional title and subtitle views.

use alloc::rc::Rc;
use core::fmt::{self, Debug, Formatter};

use cocoa_ui::geometry::{Rect as KitRect, Size as KitSize};
use cocoa_ui::layer;
use cocoa_ui::path::{CornerRadius, ShapeKind as CocoaShapeKind, shape_path};
use cocoa_ui::{PlatformView, Retained, view};
use waterui::graphics::color::{Color, WorkingColor};
use waterui::reactive::SignalExt as _;
use waterui::resolve::Resolvable;
use waterui::text::font::{Body, Font, Subheadline, Title};
use waterui::theme::color::{
    Border as BorderToken, MutedForeground, Surface as SurfaceToken, SurfaceVariant,
};
use waterui::theme::{ForegroundOverride, install_font_signal};
use waterui::widget::{CardConfig, CardStyle};
use waterui_core::Environment;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::plugin::Plugin;

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
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

/// Standard Apple continuous corner radius for card containers, in points.
const CARD_CORNER_RADIUS: f64 = 12.0;
/// Inner content inset on all four edges of a card, in points.
const CARD_PADDING: f32 = 16.0;
/// Vertical gap between title, subtitle, and body content, in points.
const CARD_CONTENT_SPACING: f32 = 6.0;
/// Hairline outline stroke width for outlined/automatic cards, in points.
const OUTLINED_BORDER_WIDTH: f64 = 1.0;
/// Drop shadow blur radius for [`CardStyle::Elevated`], in points.
const ELEVATED_SHADOW_RADIUS: f64 = 8.0;
/// Vertical drop shadow offset for [`CardStyle::Elevated`], in points.
const ELEVATED_SHADOW_OFFSET_Y: f64 = 2.0;
/// Drop shadow opacity for [`CardStyle::Elevated`].
const ELEVATED_SHADOW_OPACITY: f32 = 0.12;

fn platform_color(color: &WorkingColor) -> Retained<platform::PlatformColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

fn title_env(env: &Environment) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, Font::from(Title).resolve(env));
    scoped
}

fn subtitle_env(env: &Environment) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, Font::from(Subheadline).resolve(env));
    Plugin::install(
        ForegroundOverride::new(Color::new(MutedForeground)),
        &mut scoped,
    );
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

fn update_elevated_shadow_path(host: &PlatformView, bounds: KitRect) {
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return;
    }
    let path = shape_path(
        CocoaShapeKind::RoundedRect {
            radii: [CornerRadius::Points(CARD_CORNER_RADIUS); 4],
        },
        &[],
        bounds,
    );
    if let Some(host_layer) = layer::layer_of(host) {
        layer::set_shadow_path(&host_layer, Some(&path));
    }
}

struct CardState {
    title: Option<Mounted>,
    subtitle: Option<Mounted>,
    content: Mounted,
    elevated: bool,
}

impl CardState {
    fn measure_inner(&self, proposal: ProposalSize) -> Size {
        let inner_w = proposal
            .width
            .filter(|w| w.is_finite())
            .map(|w| 2.0_f32.mul_add(-CARD_PADDING, w).max(0.0));
        let inner_h = proposal
            .height
            .filter(|h| h.is_finite())
            .map(|h| 2.0_f32.mul_add(-CARD_PADDING, h).max(0.0));

        let header_proposal = ProposalSize::new(inner_w, None);
        let title_size = self
            .title
            .as_ref()
            .map(|t| t.layout().measure(header_proposal).size);
        let subtitle_size = self
            .subtitle
            .as_ref()
            .map(|s| s.layout().measure(header_proposal).size);

        let header_count = u32::from(title_size.is_some()) + u32::from(subtitle_size.is_some());
        #[expect(
            clippy::cast_precision_loss,
            reason = "header_count is at most 2, exact in f32"
        )]
        let gaps = (header_count as f32) * CARD_CONTENT_SPACING;
        let header_h =
            title_size.map_or(0.0, |s| s.height) + subtitle_size.map_or(0.0, |s| s.height) + gaps;

        let content_proposal = ProposalSize::new(inner_w, inner_h.map(|h| (h - header_h).max(0.0)));
        let content_size = self.content.layout().measure(content_proposal).size;

        let width = content_size
            .width
            .max(title_size.map_or(0.0, |s| s.width))
            .max(subtitle_size.map_or(0.0, |s| s.width));
        let height = header_h + content_size.height;
        Size::new(
            2.0_f32.mul_add(CARD_PADDING, width),
            2.0_f32.mul_add(CARD_PADDING, height),
        )
    }

    fn layout_subviews(&self, host: &PlatformView, bounds: KitRect) {
        let rtl = view::is_right_to_left(host);
        let pad = f64::from(CARD_PADDING);
        let spacing = f64::from(CARD_CONTENT_SPACING);
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
        let header_proposal = ProposalSize::new(Some(content_width_f32), None);

        let mut cursor_y = content_top;
        let place_item = |mounted: &Mounted, size: Size, top: f64, avail_h: f64| -> f64 {
            let item_w = f64::from(size.width).min(content_width);
            let item_h = f64::from(size.height).min(avail_h);
            let left = if rtl {
                content_right - item_w
            } else {
                content_left
            };
            #[expect(
                clippy::cast_possible_truncation,
                reason = "layout coordinates fit f32"
            )]
            let (w_f32, h_f32) = (item_w as f32, item_h as f32);
            proposal::deliver(
                mounted.view(),
                Placement::platform_hosted(ProposalSize::new(Some(w_f32), Some(h_f32))),
            );
            view::set_frame(mounted.view(), KitRect::new(left, top, item_w, item_h));
            item_h
        };

        if let Some(title) = &self.title {
            let size = title.layout().measure(header_proposal).size;
            let placed_h = place_item(title, size, cursor_y, (content_bottom - cursor_y).max(0.0));
            cursor_y = (cursor_y + placed_h + spacing).min(content_bottom);
        }

        if let Some(subtitle) = &self.subtitle {
            let size = subtitle.layout().measure(header_proposal).size;
            let placed_h = place_item(
                subtitle,
                size,
                cursor_y,
                (content_bottom - cursor_y).max(0.0),
            );
            cursor_y = (cursor_y + placed_h + spacing).min(content_bottom);
        }

        let rem_h = (content_bottom - cursor_y).max(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "layout coordinates fit f32"
        )]
        let rem_h_f32 = rem_h as f32;
        let content_size = self
            .content
            .layout()
            .measure(ProposalSize::new(Some(content_width_f32), Some(rem_h_f32)))
            .size;
        let _ = place_item(&self.content, content_size, cursor_y, rem_h);

        if self.elevated {
            update_elevated_shadow_path(host, bounds);
        }
    }
}

struct CardSubView {
    state: Rc<CardState>,
}

impl Debug for CardSubView {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CardSubView").finish_non_exhaustive()
    }
}

impl SubView for CardSubView {
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
        self.state.content.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.content.layout().priority()
    }
}

/// Realizes `CardConfig<Held>` as a native rounded card container surface on
/// `AppKit` and `UIKit`.
pub fn realize_card_config(config: CardConfig<Held>, ctx: &RenderContext<'_>) -> NativeLeaf {
    let CardConfig {
        title,
        subtitle,
        content,
        style,
        ..
    } = config;
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, KitRect::ZERO);
    let host_view: &PlatformView = &host;
    layer::ensure_layer(host_view);

    let elevated = matches!(style, CardStyle::Elevated);
    let outlined = matches!(style, CardStyle::Automatic | CardStyle::Outlined);
    let filled = matches!(style, CardStyle::Filled);

    if let Some(host_layer) = layer::layer_of(host_view) {
        layer::set_corner_radius(&host_layer, CARD_CORNER_RADIUS);
        layer::set_continuous_corner_curve(&host_layer);
        layer::set_masks_to_bounds(&host_layer, false);
        if outlined {
            layer::set_border(
                &host_layer,
                OUTLINED_BORDER_WIDTH,
                CARD_CORNER_RADIUS,
                false,
            );
        }
        if elevated {
            let shadow_color = platform_color(&WorkingColor::new([0.0, 0.0, 0.0, 1.0]));
            layer::set_shadow(
                &host_layer,
                KitSize::new(0.0, ELEVATED_SHADOW_OFFSET_Y),
                ELEVATED_SHADOW_RADIUS,
            );
            layer::set_shadow_color(&host_layer, Some(&platform::colors::cg(&shadow_color)));
            layer::set_shadow_opacity(&host_layer, ELEVATED_SHADOW_OPACITY);
        }
    }

    let scoped_title_env = title.as_ref().map(|_| title_env(ctx.env()));
    let mounted_title = title.map(|text| {
        let env = scoped_title_env
            .as_ref()
            .expect("title_env created alongside title");
        ctx.with_env(env).render(text).mount(host_view)
    });

    let scoped_subtitle_env = subtitle.as_ref().map(|_| subtitle_env(ctx.env()));
    let mounted_subtitle = subtitle.map(|text| {
        let env = scoped_subtitle_env
            .as_ref()
            .expect("subtitle_env created alongside subtitle");
        ctx.with_env(env).render(text).mount(host_view)
    });

    let mounted_content = ctx.realize(content).mount(host_view);
    crate::primary_content::forward(&host, mounted_content.view());

    let state = Rc::new(CardState {
        title: mounted_title,
        subtitle: mounted_subtitle,
        content: mounted_content,
        elevated,
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
        CardSubView {
            state: Rc::clone(&state),
        },
    );

    let fill_signal = if filled {
        SurfaceVariant.resolve(ctx.env()).computed()
    } else {
        SurfaceToken.resolve(ctx.env()).computed()
    };
    leaf.bind(&fill_signal, {
        let host = Retained::clone(&host);
        move |color| apply_background_color(&color, &host)
    });

    if outlined {
        let border_signal = BorderToken.resolve(ctx.env()).computed();
        leaf.bind(&border_signal, {
            let host = Retained::clone(&host);
            move |color| apply_border_color(&color, &host)
        });
    }

    if let Some(env) = scoped_title_env {
        leaf.keep(env);
    }
    if let Some(env) = scoped_subtitle_env {
        leaf.keep(env);
    }
    leaf.keep(state);
    leaf
}
