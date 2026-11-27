//! The `list_row` leaf: `Native<ListRowConfig>` realized directly in the
//! Apple backend without adding duplicate outer insets on top of the enclosing
//! `List` cell's row padding.

use alloc::rc::Rc;
use core::fmt::{self, Debug, Formatter};

use cocoa_ui::geometry::Rect as KitRect;
use cocoa_ui::view;
use nami::Signal as _;
use waterui_controls::list::{ListRowConfig, RowLayout};
use waterui_controls::theme::ForegroundOverride;
use waterui_core::contract_violation;
use waterui_core::env::Installed;
use waterui_core::layout::{
    LayoutDirection, LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView,
    ViewDimensions, layout_direction,
};

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::proposal::{self, Placement};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// Horizontal gap between label and value in [`RowLayout::Inline`], matching
/// Apple inset-grouped key/value rows.
const INLINE_SPACING: f32 = 12.0;
/// Vertical gap between label and value in [`RowLayout::Detail`], matching
/// Apple detail rows.
const DETAIL_SPACING: f32 = 6.0;

struct ListRowState {
    label: Mounted,
    value: Mounted,
    layout: RowLayout,
    rtl: bool,
}

fn inline_proposals(
    available: f32,
    spacing: f32,
    label_natural: f32,
    value_natural: f32,
) -> (ProposalSize, ProposalSize) {
    let remaining = (available - spacing).max(0.0);
    if label_natural + value_natural <= remaining {
        return (ProposalSize::UNSPECIFIED, ProposalSize::UNSPECIFIED);
    }
    let half = remaining * 0.5;
    let (label_max, value_max) = if label_natural <= half {
        (label_natural, (remaining - label_natural).max(0.0))
    } else if value_natural <= half {
        ((remaining - value_natural).max(0.0), value_natural)
    } else {
        (half, half)
    };
    (
        ProposalSize::new(Some(label_max), None),
        ProposalSize::new(Some(value_max), None),
    )
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "AppKit/UIKit layout coordinates fit f32"
)]
const fn f64_as_f32(value: f64) -> f32 {
    value as f32
}

impl ListRowState {
    fn layout_inline(&self, bounds: KitRect, width_f32: f32) {
        let width = bounds.size.width;
        let height = bounds.size.height;
        let label_nat = self.label.layout().measure(ProposalSize::UNSPECIFIED).size;
        let value_nat = self.value.layout().measure(ProposalSize::UNSPECIFIED).size;
        let spacing = if label_nat.width > 0.0 && value_nat.width > 0.0 {
            INLINE_SPACING
        } else {
            0.0
        };
        let (label_prop, value_prop) =
            inline_proposals(width_f32, spacing, label_nat.width, value_nat.width);
        let label_size = if label_prop == ProposalSize::UNSPECIFIED {
            label_nat
        } else {
            self.label.layout().measure(label_prop).size
        };
        let value_size = if value_prop == ProposalSize::UNSPECIFIED {
            value_nat
        } else {
            self.value.layout().measure(value_prop).size
        };

        let spacing_f64 = f64::from(spacing);
        let label_w = f64::from(label_size.width).min(width);
        let label_h = f64::from(label_size.height).min(height);
        let label_y = ((height - label_h) * 0.5).max(0.0);
        let label_x = if self.rtl {
            (width - label_w).max(0.0)
        } else {
            0.0
        };

        let max_value_w = (width - label_w - spacing_f64).max(0.0);
        let value_w = f64::from(value_size.width).min(max_value_w);
        let value_h = f64::from(value_size.height).min(height);
        let value_y = ((height - value_h) * 0.5).max(0.0);
        let value_x = if self.rtl {
            (label_x - spacing_f64 - value_w).max(0.0)
        } else {
            (width - value_w).max(label_w + spacing_f64).min(width)
        };

        proposal::deliver(
            self.label.view(),
            Placement::platform_hosted(ProposalSize::new(
                Some(f64_as_f32(label_w)),
                Some(label_size.height),
            )),
        );
        view::set_frame(
            self.label.view(),
            KitRect::new(
                bounds.origin.x + label_x,
                bounds.origin.y + label_y,
                label_w,
                label_h,
            ),
        );

        proposal::deliver(
            self.value.view(),
            Placement::platform_hosted(ProposalSize::new(
                Some(f64_as_f32(value_w)),
                Some(value_size.height),
            )),
        );
        view::set_frame(
            self.value.view(),
            KitRect::new(
                bounds.origin.x + value_x,
                bounds.origin.y + value_y,
                value_w,
                value_h,
            ),
        );
    }

    fn layout_detail(&self, bounds: KitRect, width_f32: f32) {
        let width = bounds.size.width;
        let height = bounds.size.height;
        let child_prop = ProposalSize::new(Some(width_f32), None);
        let label_size = self.label.layout().measure(child_prop).size;
        let value_size = self.value.layout().measure(child_prop).size;

        let label_w = f64::from(label_size.width).min(width);
        let label_h = f64::from(label_size.height).min(height);
        let label_x = if self.rtl {
            (width - label_w).max(0.0)
        } else {
            0.0
        };

        let spacing = if label_size.height > 0.0 && value_size.height > 0.0 {
            f64::from(DETAIL_SPACING)
        } else {
            0.0
        };
        let value_w = f64::from(value_size.width).min(width);
        let value_x = if self.rtl {
            (width - value_w).max(0.0)
        } else {
            0.0
        };
        let value_y = (label_h + spacing).min(height);
        let value_h = f64::from(value_size.height).min((height - value_y).max(0.0));

        proposal::deliver(
            self.label.view(),
            Placement::platform_hosted(ProposalSize::new(
                Some(f64_as_f32(label_w)),
                Some(label_size.height),
            )),
        );
        view::set_frame(
            self.label.view(),
            KitRect::new(bounds.origin.x + label_x, bounds.origin.y, label_w, label_h),
        );

        proposal::deliver(
            self.value.view(),
            Placement::platform_hosted(ProposalSize::new(
                Some(f64_as_f32(value_w)),
                Some(value_size.height),
            )),
        );
        view::set_frame(
            self.value.view(),
            KitRect::new(
                bounds.origin.x + value_x,
                bounds.origin.y + value_y,
                value_w,
                value_h,
            ),
        );
    }

    fn layout_subviews(&self, bounds: KitRect) {
        let width = bounds.size.width;
        let height = bounds.size.height;
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let width_f32 = f64_as_f32(width);

        match self.layout {
            RowLayout::Inline => self.layout_inline(bounds, width_f32),
            RowLayout::Detail => self.layout_detail(bounds, width_f32),
            _ => {
                contract_violation!(
                    "waterui-apple: unsupported RowLayout variant in ListRowConfig"
                );
            }
        }
    }
}

struct ListRowSubView {
    state: Rc<ListRowState>,
}

impl Debug for ListRowSubView {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListRowSubView").finish_non_exhaustive()
    }
}

impl SubView for ListRowSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let proposed_width = proposal.width.filter(|w| w.is_finite());

        match self.state.layout {
            RowLayout::Inline => {
                let label_nat = self
                    .state
                    .label
                    .layout()
                    .measure(ProposalSize::UNSPECIFIED)
                    .size;
                let value_nat = self
                    .state
                    .value
                    .layout()
                    .measure(ProposalSize::UNSPECIFIED)
                    .size;
                let spacing = if label_nat.width > 0.0 && value_nat.width > 0.0 {
                    INLINE_SPACING
                } else {
                    0.0
                };
                let Some(avail) = proposed_width else {
                    return ViewDimensions::new(Size::new(
                        label_nat.width + spacing + value_nat.width,
                        label_nat.height.max(value_nat.height),
                    ));
                };
                let (label_prop, value_prop) =
                    inline_proposals(avail, spacing, label_nat.width, value_nat.width);
                let label_size = if label_prop == ProposalSize::UNSPECIFIED {
                    label_nat
                } else {
                    self.state.label.layout().measure(label_prop).size
                };
                let value_size = if value_prop == ProposalSize::UNSPECIFIED {
                    value_nat
                } else {
                    self.state.value.layout().measure(value_prop).size
                };
                let intrinsic_w = label_size.width + spacing + value_size.width;
                ViewDimensions::new(Size::new(
                    avail.max(intrinsic_w),
                    label_size.height.max(value_size.height),
                ))
            }
            RowLayout::Detail => {
                let child_prop = proposed_width.map_or(ProposalSize::UNSPECIFIED, |w| {
                    ProposalSize::new(Some(w), None)
                });
                let label_size = self.state.label.layout().measure(child_prop).size;
                let value_size = self.state.value.layout().measure(child_prop).size;
                let intrinsic_w = label_size.width.max(value_size.width);
                let width = proposed_width.map_or(intrinsic_w, |w| w.max(intrinsic_w));
                let spacing = if label_size.height > 0.0 && value_size.height > 0.0 {
                    DETAIL_SPACING
                } else {
                    0.0
                };
                ViewDimensions::new(Size::new(
                    width,
                    label_size.height + spacing + value_size.height,
                ))
            }
            _ => {
                contract_violation!(
                    "waterui-apple: unsupported RowLayout variant in ListRowConfig"
                );
                ViewDimensions::new(Size::zero())
            }
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Realizes `ListRowConfig<Held>` by mounting the label and value subviews
/// inside a borderless `HostView` with zero extra outer padding.
pub fn realize_list_row_config(config: ListRowConfig<Held>, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let (label_control, value_text, layout, value_color) = config.into_parts();
    let rtl = matches!(
        layout_direction(ctx.env()).snapshot(),
        LayoutDirection::RightToLeft
    );

    let host = HostView::new(mtm, KitRect::ZERO);
    let label = ctx.realize(label_control.build()).mount(&host);
    view::set_translates_autoresizing(label.view(), true);

    let value_leaf = match value_color {
        Some(color) => ctx.render(Installed::new(value_text, ForegroundOverride::new(color))),
        None => ctx.render(value_text),
    };
    let value = value_leaf.mount(&host);
    view::set_translates_autoresizing(value.view(), true);

    let state = Rc::new(ListRowState {
        label,
        value,
        layout,
        rtl,
    });
    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |view| {
            state.layout_subviews(view::bounds(view));
        }
    });

    NativeLeaf::new(&host, ListRowSubView { state })
}
