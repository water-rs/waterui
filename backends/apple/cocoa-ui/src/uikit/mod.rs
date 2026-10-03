//! `UIKit`, the user-interface framework of iOS.
//!
//! An iOS application is started with [`run`] and a set of
//! [`ApplicationHandlers`]. The system then connects a [`WindowScene`] for the
//! application's window, and the scene handler answers with the [`Window`] to
//! show in it, usually rooted in a [`ViewController`] whose [`HostView`] fills
//! the window.
//!
//! The application's `Info.plist` must name `SceneDelegate` as the delegate
//! class of its window scene configuration: that is the class through which
//! the system reaches the scene handler.
//!
//! ```no_run
//! use cocoa_ui::MainThreadMarker;
//! use cocoa_ui::uikit::{self, ApplicationHandlers, ViewController, Window};
//!
//! let mtm = MainThreadMarker::new().expect("main runs on the main thread");
//! uikit::run(
//!     mtm,
//!     ApplicationHandlers::new(|scene| {
//!         let window = Window::new(scene);
//!         let controller = ViewController::new(scene.main_thread());
//!         controller.host_view().set_layout_handler(|_view| {
//!             // Give the subviews their frames here.
//!         });
//!         window.set_root_view_controller(&controller);
//!         window.make_key_and_visible();
//!         window
//!     }),
//! );
//! ```

mod appearance;
mod application;
mod badge;
pub mod button;
mod calendar_view;
mod color_view;
pub mod color_well;
mod context_menu;
mod date_picker;
pub mod drag_drop;
pub mod gesture;
pub use color_well::ColorWell;
pub use context_menu::{
    AccessoryOverlay, ContextMenu, ContextMenuConfiguration, ContextMenuHandlers, bounds_in_window,
    preview_controller, targeted_preview, targeted_preview_at,
};
pub mod colors;
mod host_view;
pub mod image;
pub mod input_view;
mod menu;
mod navigation;
pub use pointer::PointerInteraction;
mod search_bar;
mod split;
pub mod surface_view;
mod tabs;
pub mod view_controller;
mod window;

pub use appearance::{ColorSchemeObservation, current_scheme};
pub use application::{
    ApplicationHandlers, MenuBuilder, WindowScene, request_main_menu_rebuild, run,
};
pub use badge::BadgeView;
pub use button::Button;
pub use calendar_view::CalendarView;
pub use color_view::ColorView;
pub use colors::UiColor;
pub use date_picker::{DatePicker, DatePickerMode};
pub use gesture::GestureAttachment;
pub use host_view::{HitTest, HostView};
pub use image::ImageView;
pub mod label;
pub use menu::{Menu, MenuAction, MenuButton, MenuElement};
mod picker;
mod pointer;
pub use picker::Picker;
pub mod toggle;
pub use label::Label;
mod slider;
pub mod text_field;
pub use slider::Slider;
mod progress;
pub use progress::Progress;
mod scroll;
pub use scroll::ScrollView;
mod stepper;
pub use menu::{elements, menu, menu_with_identifier};
pub use navigation::{
    BarButton, LargeTitle, NavBar, NavContentController, NavPage, NavSearch, NavigationController,
    SearchBarPlacement, SearchUpdater, bar_item, bar_item_accessibility_label, first_button,
    first_control, image_bar_item,
};
pub use search_bar::SearchBar;
pub use split::{ColumnWidth, SplitColumnController, SplitController};
pub mod table;
pub use crate::geometry::IndexPath;
pub use stepper::Stepper;
pub use table::{SectionKind, TableCell, TableHeaderFooterView, TableSource, TableView};
pub use tabs::{TabContentController, TabSpec, TabsController};
pub use text_field::{SecureField, TextField};
pub use view_controller::ViewController;
pub use window::{Window, application_is_active, main_screen_scale, window_of};
