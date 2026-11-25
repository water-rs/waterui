//! The `disclosure` leaf: `Native<DisclosureConfig>` realized through
//! `cocoa_ui`'s native disclosure button (`NSBezelStyle::Disclosure` on `AppKit`,
//! `chevron.right`/`chevron.down` toggle button on `UIKit`) alongside the
//! mounted header label and lazily mounted content.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use cocoa_ui::geometry::{Rect as KitRect, Size as KitSize};
use cocoa_ui::text::strip_bidi_controls;
use cocoa_ui::view::hide_from_accessibility;
use cocoa_ui::{PlatformView, Retained, view};
use waterui::Signal as _;
use waterui::widget::disclosure::{DisclosureConfig, SlotBuilder};
use waterui_core::layout::{
    LayoutDirection, LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView,
    ViewDimensions, layout_direction,
};
use waterui_interaction::interaction::Disabled;

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext, Renderer};
use crate::proposal::{self, Placement};

#[cfg(target_os = "ios")]
use crate::components::button_chrome;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::HostView;
    pub(super) use cocoa_ui::appkit::disclosure::{DisclosureButton, attach_label_click};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::HostView;
    pub(super) use cocoa_ui::uikit::disclosure::{DisclosureButton, attach_label_click};
}

use platform::{DisclosureButton, HostView, attach_label_click};

const INDICATOR_GAP: f32 = 8.0;
const MIN_HEADER_HEIGHT: f32 = 24.0;
const CONTENT_SPACING: f32 = 6.0;
const CONTENT_INDENT: f32 = 24.0;

struct DisclosureState {
    renderer: Renderer,
    host: Retained<HostView>,
    button: Retained<DisclosureButton>,
    header: Mounted,
    content_builder: SlotBuilder<Held>,
    content: Option<Mounted>,
    rtl: bool,
    last_width: Cell<f32>,
}

impl DisclosureState {
    fn sync_content(&mut self, expanded: bool) {
        if expanded && self.content.is_none() {
            let child = self.renderer.realize(self.content_builder.build());
            let mounted = child.mount(&self.host);
            view::set_translates_autoresizing(mounted.view(), true);
            self.content = Some(mounted);
            view::invalidate_layout(&self.host);
            crate::measure_memo::invalidate();
        } else if !expanded && self.content.is_some() {
            self.content = None;
            view::invalidate_layout(&self.host);
            crate::measure_memo::invalidate();
        }
    }

    fn header_metrics(&self) -> (KitSize, Size, f32, f32) {
        let btn = self.button.intrinsic_size();
        let hdr = self
            .header
            .layout()
            .measure(ProposalSize::new(None, None))
            .size;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "AppKit/UIKit intrinsic button points always fit f32"
        )]
        let btn_w = btn.width as f32;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "AppKit/UIKit intrinsic button points always fit f32"
        )]
        let btn_h = btn.height as f32;
        let header_w = btn_w + INDICATOR_GAP + hdr.width;
        let header_h = btn_h.max(hdr.height).max(MIN_HEADER_HEIGHT);
        (btn, hdr, header_w, header_h)
    }

    fn layout_subviews(&self, bounds: KitRect) {
        let width = bounds.size.width;
        let (btn, hdr, _, header_h) = self.header_metrics();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "AppKit/UIKit intrinsic button points always fit f32"
        )]
        let btn_w = btn.width as f32;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "AppKit/UIKit intrinsic button points always fit f32"
        )]
        let btn_h = btn.height as f32;

        let btn_x = if self.rtl { width - btn.width } else { 0.0 };
        let btn_y = (f64::from(header_h) - btn.height) * 0.5;
        view::set_frame(
            as_platform_view(&self.button),
            KitRect::new(btn_x.max(0.0), btn_y.max(0.0), btn.width, btn.height),
        );

        let max_hdr_w = (width - btn.width - f64::from(INDICATOR_GAP)).max(0.0);
        let hdr_w = f64::from(hdr.width).min(max_hdr_w);
        let hdr_x = if self.rtl {
            (btn_x - f64::from(INDICATOR_GAP) - hdr_w).max(0.0)
        } else {
            btn.width + f64::from(INDICATOR_GAP)
        };
        let hdr_y = (f64::from(header_h) - f64::from(hdr.height)) * 0.5;
        let hdr_rect = KitRect::new(hdr_x, hdr_y.max(0.0), hdr_w, f64::from(hdr.height));
        #[expect(
            clippy::cast_possible_truncation,
            reason = "layout coordinates fit f32"
        )]
        let hdr_w_f32 = hdr_w as f32;
        proposal::deliver(
            self.header.view(),
            Placement::platform_hosted(ProposalSize::new(Some(hdr_w_f32), Some(hdr.height))),
        );
        view::set_frame(self.header.view(), hdr_rect);

        if let Some(content) = &self.content {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "layout coordinates fit f32"
            )]
            let avail_w = ((width as f32) - CONTENT_INDENT).max(0.0);
            let content_size = content
                .layout()
                .measure(ProposalSize::new(Some(avail_w), None))
                .size;
            let content_x = if self.rtl {
                0.0
            } else {
                f64::from(CONTENT_INDENT)
            };
            let content_y = f64::from(header_h + CONTENT_SPACING);
            let content_w = f64::from(content_size.width).min(f64::from(avail_w));
            let content_h = f64::from(content_size.height);
            proposal::deliver(
                content.view(),
                Placement::platform_hosted(ProposalSize::new(
                    Some(avail_w),
                    Some(content_size.height),
                )),
            );
            view::set_frame(
                content.view(),
                KitRect::new(content_x, content_y, content_w, content_h),
            );
        }
        let _ = btn_w;
        let _ = btn_h;
    }
}

#[cfg(target_os = "macos")]
fn as_platform_view(button: &DisclosureButton) -> &PlatformView {
    button.as_view()
}

#[cfg(target_os = "ios")]
fn as_platform_view(button: &DisclosureButton) -> &PlatformView {
    button.as_view()
}

struct DisclosureSubView {
    state: Rc<RefCell<DisclosureState>>,
}

impl core::fmt::Debug for DisclosureSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisclosureSubView").finish_non_exhaustive()
    }
}

impl SubView for DisclosureSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = self.state.borrow();
        let (_, _, header_w, header_h) = state.header_metrics();
        let (intrinsic_w, total_h) =
            state
                .content
                .as_ref()
                .map_or((header_w, header_h), |content| {
                    let content_offer = proposal.width.map(|w| (w - CONTENT_INDENT).max(0.0));
                    let content_size = content
                        .layout()
                        .measure(ProposalSize::new(content_offer, None))
                        .size;
                    (
                        header_w.max(CONTENT_INDENT + content_size.width),
                        header_h + CONTENT_SPACING + content_size.height,
                    )
                });
        let width = match proposal.width {
            Some(w) if w.is_finite() => w.max(intrinsic_w),
            _ => intrinsic_w,
        };
        state.last_width.set(width);
        ViewDimensions::new(Size::new(width, total_h))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Realizes `DisclosureConfig<Held>` using the native Apple disclosure button
/// alongside the mounted header label and lazy content view.
pub fn realize_disclosure_config(
    config: DisclosureConfig<Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let (header_label, content_builder, expanded) = config.into_parts();
    let initial_expanded = expanded.snapshot();
    let accessibility_label = header_label.accessibility_label();
    let disabled = Disabled::resolve(ctx.env(), false);
    let rtl = matches!(
        layout_direction(ctx.env()).snapshot(),
        LayoutDirection::RightToLeft
    );

    let host = HostView::new(mtm, KitRect::ZERO);
    let button = DisclosureButton::new(mtm, initial_expanded);
    view::add_subview(&host, as_platform_view(&button));

    let header = ctx.realize(header_label.build()).mount(&host);
    view::set_translates_autoresizing(header.view(), true);
    hide_from_accessibility(header.view());

    let label_click = attach_label_click(header.view(), button.clone());

    let state = Rc::new(RefCell::new(DisclosureState {
        renderer: ctx.renderer(),
        host: host.clone(),
        button: button.clone(),
        header,
        content_builder,
        content: None,
        rtl,
        last_width: Cell::new(0.0),
    }));
    state.borrow_mut().sync_content(initial_expanded);

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |view| {
            state.borrow().layout_subviews(view::bounds(view));
        }
    });

    let mut leaf = NativeLeaf::new(
        &host,
        DisclosureSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(label_click);

    leaf.keep(button.on_change({
        let button = button.clone();
        let expanded = expanded.clone();
        move |_| {
            let next = button.is_expanded();
            if expanded.snapshot() != next {
                expanded.set(next);
            }
        }
    }));

    leaf.watch(&expanded, {
        let state = Rc::clone(&state);
        let button = button.clone();
        move |watch_ctx| {
            let next = *watch_ctx.value();
            button.set_expanded(next);
            state.borrow_mut().sync_content(next);
            host.layout_if_needed();
        }
    });

    leaf.bind(&disabled, {
        let button = button.clone();
        move |is_disabled| button.set_enabled(!is_disabled)
    });

    #[cfg(target_os = "ios")]
    leaf.bind(&button_chrome::tint_signal(ctx.env()), {
        let button = button.clone();
        move |color| button.set_tint(&button_chrome::platform_color(&color))
    });

    leaf.bind(&accessibility_label, move |styled| {
        let plain = strip_bidi_controls(&styled.to_plain());
        button.set_accessibility_label(&plain);
    });

    leaf
}
