//! `AppKit`, the user-interface framework of macOS.
//!
//! A macOS application is one [`Application`] run with
//! [`ApplicationHandlers`]: when it finishes launching, the handler creates
//! [`Window`]s and puts a [`HostView`] in each. The main menu is a [`Menu`] of
//! [`MenuItem`]s whose [`MenuAction`]s travel the responder chain.
//!
//! ```no_run
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! use cocoa_ui::appkit::{
//!     Application, ApplicationHandlers, HostView, Window, WindowStyle,
//! };
//! use cocoa_ui::{MainThreadMarker, Rect};
//!
//! let mtm = MainThreadMarker::new().expect("main runs on the main thread");
//! let windows = Rc::new(RefCell::new(Vec::new()));
//! let launched = Rc::clone(&windows);
//! Application::shared(mtm).run(
//!     ApplicationHandlers::new()
//!         .did_finish_launching(move |mtm| {
//!             let window = Window::new(mtm, Rect::new(0.0, 0.0, 800.0, 600.0), WindowStyle::all());
//!             window.set_title("Hello");
//!             window.set_content_view(&HostView::new(mtm, window.content_rect()));
//!             window.center();
//!             window.make_key_and_order_front();
//!             launched.borrow_mut().push(window);
//!         })
//!         .should_terminate_after_last_window_closed(|_| true),
//! );
//! ```

mod animate;
mod appearance;
mod application;
mod badge;
pub mod button;
mod color_view;
mod color_well;
pub mod colors;
mod context_menu;
pub mod control;
mod cursor;
mod date_picker;
pub mod drag_drop;
mod effect_view;
pub mod event;
pub mod gesture;
mod host_view;
pub mod image;
pub mod input_view;
mod list_table;
mod menu;
mod panel;
mod popup_button;
mod search_field;
mod segmented;
mod source_list;
mod split;
pub mod surface_view;
mod table;
mod toolbar;
mod view_controller;
mod window;

pub use animate::{run_animation, set_animated_alpha};
pub use appearance::ColorSchemeObservation;
pub use application::{
    ActivationPolicy, Application, ApplicationHandlers, AttentionRequest, AttentionRequestToken,
};
pub use badge::BadgeView;
pub use button::Button;
pub use color_view::ColorView;
pub use color_well::ColorWell;
pub use colors::AppColor;
pub use context_menu::{AccessoryPanel, ContextMenu};
pub use control::{activate, first_button, first_control};
pub use cursor::Cursor;
pub use date_picker::{DatePicker, DatePickerElements};
pub use effect_view::{header_material_view, set_material_background};
pub use gesture::GestureAttachment;
pub use host_view::{HitTest, HostView};
pub use image::{ImageView, first_symbol_view, symbol_image};
pub mod label;
mod picker;
pub use picker::Picker;
pub mod toggle;
pub use label::Label;
mod slider;
pub use crate::menu::{Command, KeyModifiers, MenuTreeNode};
pub use menu::{Menu, MenuAction, MenuButton, MenuItem};
pub use panel::{
    Panel, content_bounds, convert_to_screen, did_resize_notification,
    view_frame_did_change_notification, window_of,
};
pub use slider::Slider;
mod progress;
pub use progress::Progress;
mod scroll;
pub use scroll::ScrollView;
mod stepper;
pub use list_table::{
    DeleteButton, RowContainer, SectionHeader, SectionKind, TableRowView, TableSource,
    TableView as ListTableView,
};
pub use stepper::Stepper;
pub use table::TableView;
pub mod text_field;
pub use popup_button::PopUpButton;
pub use search_field::SearchField;
pub use segmented::{Segment, SegmentedControl};
pub use source_list::SourceList;
pub use split::{Column, ColumnWidth, SplitViewController};
pub use text_field::{SecureField, TextField};
pub use toolbar::{HostedItem, HostedSearch, ToolbarChild, ToolbarContent, WindowToolbar};
pub use view_controller::ViewController;
pub use window::{
    Window, WindowLevel, WindowStyle, is_visible, main_screen_scale, watch_occlusion,
};
