//! The `AppKit` table: an `NSScrollView` wrapping an `NSTableView` whose
//! data source and delegate are the scroll view itself.
//!
//! [`TableView`] is the leaf surface a list renders into. The wrapped
//! [`NSTableView`] (`table_view()`) holds a single `content` column and
//! answers the `NSTableViewDataSource`/`NSTableViewDelegate` callbacks by
//! forwarding to the [`TableSource`] the consumer installs; the scroll view
//! owns the scroll chrome (vertical scroller, autohide, top content inset)
//! and is flipped so document space is top-left origin.
//!
//! [`TableRowView`] is the `NSTableRowView` that draws a hairline separator
//! under a row when its consumer installs a separator handler, and answers
//! `isEmphasized` from the key state of the window it is in — pointer
//! clicks on a table's rows go to the row content, not to first responder,
//! so selection emphasis has to read the window directly.
//!
//! [`RowContainer`] is the per-row cell view: it mounts one content view
//! inside insets and can present an inline delete button.
//!
//! `TableView` answers `cocoaUiIsSidebarContent` as a KVC-read/write pair
//! (`setCocoaUiIsSidebarContent:`) so a Swift-side owner can tell the table
//! it draws a sidebar's contents — the translucent material behind it means
//! the table must paint no background and take the source-list row chrome.
//!
//! # Safety
//!
//! `unsafe` here subclasses `NSScrollView`/`NSTableRowView`/`NSView`, calls
//! `super`, forwards `AppKit` callbacks into the stored `Rc` handlers, and
//! builds `NSPasteboardItem`s for drags. The superclasses are main-thread
//! classes and every class is marked `MainThreadOnly`; every override guards
//! the handler call with [`crate::callback::guarded`].

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSColor, NSControlTextEditingDelegate, NSDragOperation, NSDraggingInfo,
    NSFont, NSLayoutConstraint, NSPasteboardItem, NSPasteboardType, NSPasteboardWriting,
    NSRectFill, NSScrollView, NSTableColumn, NSTableRowView, NSTableView,
    NSTableViewAnimationOptions, NSTableViewDataSource, NSTableViewDelegate,
    NSTableViewDropOperation, NSTableViewSelectionHighlightStyle, NSTableViewStyle, NSTextField,
    NSView,
};
use objc2_foundation::{
    NSArray, NSEdgeInsets, NSIndexSet, NSInteger, NSMutableIndexSet, NSNotFound, NSNotification,
    NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use crate::callback::guarded;
use crate::geometry::EdgeInsets;

/// The table's content-bearing column identifier.
const COLUMN_IDENTIFIER: &str = "content";

/// The data the wrapped `NSTableView` reads rows from — the list's model.
///
/// All methods are called on the main thread. `rows`, `view_for_row`,
/// `row_view` and `row_height` are asked by `AppKit` during tiling and must
/// answer cheaply; `view_for_row` is where a row's view materializes.
pub trait TableSource {
    /// The number of rows (flat entries) the table currently presents.
    fn rows(&self, table: &TableView) -> usize;

    /// The cell view for `row`, or `None` for an empty row.
    fn view_for_row(&self, table: &TableView, row: usize) -> Option<Retained<NSView>>;

    /// The row view (background/separator chrome) for `row`; `None` takes
    /// `AppKit`'s default.
    fn row_view(&self, table: &TableView, row: usize) -> Option<Retained<TableRowView>> {
        let _ = (table, row);
        None
    }

    /// The height `row` reports.
    fn row_height(&self, table: &TableView, row: usize) -> f64;

    /// Whether `row` draws as a group row.
    fn is_group_row(&self, table: &TableView, row: usize) -> bool {
        let _ = (table, row);
        false
    }

    /// Whether a pointer click may select `row`.
    fn should_select(&self, table: &TableView, row: usize) -> bool {
        let _ = (table, row);
        true
    }

    /// The table's selection changed.
    fn selection_did_change(&self, table: &TableView) {
        let _ = table;
    }

    /// The drag payload `row` contributes, or `None` when the row is not
    /// draggable. The string is written to the pasteboard under the first
    /// registered drag type.
    fn dragged_payload(&self, table: &TableView, row: usize) -> Option<String> {
        let _ = (table, row);
        None
    }

    /// Whether a drop landing above `row` (`on_row == false`) or on it is
    /// acceptable.
    fn validate_drop(&self, table: &TableView, row: usize, on_row: bool) -> bool {
        let _ = (table, row, on_row);
        false
    }

    /// The drop landed on `row` carrying `payload`; return whether it was
    /// accepted.
    fn accept_drop(&self, table: &TableView, row: usize, payload: &str) -> bool {
        let _ = (table, row, payload);
        false
    }
}

/// The handler [`TableView`] calls after `AppKit` lays it out.
type LayoutHandler = Rc<dyn Fn(&TableView)>;
/// The handler [`TableView`] calls when it moves between windows.
type WindowHandler = Rc<dyn Fn(&TableView)>;

/// The per-instance state [`TableView`] stores.
#[derive(Default)]
pub struct TableViewIvars {
    /// The installed row provider.
    source: RefCell<Option<Rc<dyn TableSource>>>,
    /// The wrapped `NSTableView`.
    table: RefCell<Option<Retained<NSTableView>>>,
    /// The drag types `set_drag_types` registered, in order.
    drag_types: RefCell<Vec<Retained<NSString>>>,
    /// Post-layout hook (column-width tracking).
    layout: RefCell<Option<LayoutHandler>>,
    /// Window-attach hook (key-state observation).
    window: RefCell<Option<WindowHandler>>,
    /// Whether the table draws sidebar chrome.
    sidebar: Cell<bool>,
}

impl fmt::Debug for TableViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableViewIvars")
            .field("source", &self.source.borrow().is_some())
            .field("sidebar", &self.sidebar.get())
            .finish_non_exhaustive()
    }
}

impl TableViewIvars {
    /// The installed source, if any.
    fn source(&self) -> Option<Rc<dyn TableSource>> {
        self.source.borrow().as_ref().cloned()
    }
}

define_class!(
    // SAFETY: `NSScrollView`'s designated initializer is `initWithFrame:`,
    // which `TableView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSScrollView))]
    #[name = "CocoaUiListTableView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TableViewIvars]
    /// A scroll view hosting a single-column `NSTableView` that sources its
    /// rows from an installed [`TableSource`].
    pub struct TableView;

    unsafe impl NSObjectProtocol for TableView {}

    // SAFETY: every `NSControlTextEditingDelegate` method is optional; the
    // table has no editable text cells.
    unsafe impl NSControlTextEditingDelegate for TableView {}

    // SAFETY: `NSTableViewDataSource`'s required `numberOfRowsInTableView`
    // is answered from the installed source; the optional drag methods are
    // answered the same way, and the pasteboard item is built locally.
    unsafe impl NSTableViewDataSource for TableView {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table_view: &NSTableView) -> NSInteger {
            self.ivars()
                .source()
                .map_or(0, |source| source.rows(self).cast_signed())
        }

        #[unsafe(method_id(tableView:pasteboardWriterForRow:))]
        fn pasteboard_writer(
            &self,
            _table_view: &NSTableView,
            row: NSInteger,
        ) -> Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> {
            let item = self
                .ivars()
                .source()
                .and_then(|source| source.dragged_payload(self, row.cast_unsigned()))
                .and_then(|payload| {
                    let drag_types = self.ivars().drag_types.borrow();
                    drag_types.first().map(|drag_type| {
                        let item = NSPasteboardItem::new();
                        item.setString_forType(&NSString::from_str(&payload), drag_type);
                        item
                    })
                });
            item.map(ProtocolObject::from_retained)
        }

        #[unsafe(method(tableView:validateDrop:proposedRow:proposedDropOperation:))]
        fn validate_drop(
            &self,
            table_view: &NSTableView,
            _info: &ProtocolObject<dyn NSDraggingInfo>,
            row: NSInteger,
            drop_operation: NSTableViewDropOperation,
        ) -> NSDragOperation {
            let Some(source) = self.ivars().source() else {
                return NSDragOperation::None;
            };
            let on_row = drop_operation == NSTableViewDropOperation::On;
            if source.validate_drop(self, row.cast_unsigned(), on_row) {
                NSDragOperation::Move
            } else {
                NSDragOperation::None
            }
        }

        #[unsafe(method(tableView:acceptDrop:row:dropOperation:))]
        fn accept_drop(
            &self,
            _table_view: &NSTableView,
            info: &ProtocolObject<dyn NSDraggingInfo>,
            row: NSInteger,
            _drop_operation: NSTableViewDropOperation,
        ) -> Bool {
            let Some(source) = self.ivars().source() else {
                return Bool::NO;
            };
            let payload = (|| {
                let drag_types = self.ivars().drag_types.borrow();
                let drag_type = drag_types.first()?;
                let items = info.draggingPasteboard().pasteboardItems()?;
                items.firstObject()?.stringForType(drag_type)
            })();
            let Some(payload) = payload else {
                return Bool::NO;
            };
            Bool::new(source.accept_drop(self, row.cast_unsigned(), &payload.to_string()))
        }
    }

    // SAFETY: `NSTableViewDelegate` methods only forward to the installed
    // source; the returned views are the ones the source built.
    unsafe impl NSTableViewDelegate for TableView {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            _table_view: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            self.ivars()
                .source()
                .and_then(|source| source.view_for_row(self, row.cast_unsigned()))
        }

        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view_for_row(
            &self,
            _table_view: &NSTableView,
            row: NSInteger,
        ) -> Option<Retained<NSTableRowView>> {
            self.ivars().source().and_then(|source| {
                source
                    .row_view(self, row.cast_unsigned())
                    .map(Retained::into_super)
            })
        }

        #[unsafe(method(tableView:heightOfRow:))]
        fn height_of_row(&self, _table_view: &NSTableView, row: NSInteger) -> f64 {
            self.ivars()
                .source()
                .map_or(0.0, |source| source.row_height(self, row.cast_unsigned()))
        }

        #[unsafe(method(tableView:isGroupRow:))]
        fn is_group_row(&self, _table_view: &NSTableView, row: NSInteger) -> bool {
            self.ivars()
                .source()
                .is_some_and(|source| source.is_group_row(self, row.cast_unsigned()))
        }

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select_row(&self, _table_view: &NSTableView, row: NSInteger) -> bool {
            self.ivars()
                .source()
                .is_some_and(|source| source.should_select(self, row.cast_unsigned()))
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, _notification: &NSNotification) {
            guarded("TableView selectionDidChange", || {
                if let Some(source) = self.ivars().source() {
                    source.selection_did_change(self);
                }
            });
        }
    }

    impl TableView {
        /// Flipped document space: the table's origin is its top edge.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }

        /// Keeps `AppKit`'s layout, then reports it so the consumer can
        /// track the content width onto the column.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            guarded("TableView layout", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layout] };
                if let Some(handler) = self.ivars().layout.borrow().as_ref().cloned() {
                    handler(self);
                }
            });
        }

        /// Window changes swap the consumer's key-state observation.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            guarded("TableView viewDidMoveToWindow", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
                if let Some(handler) = self.ivars().window.borrow().as_ref().cloned() {
                    handler(self);
                }
            });
        }

        // SAFETY: exposed as a KVC pair so a Swift owner can restyle the
        // table for sidebar contents without knowing this class.
        #[unsafe(method(cocoaUiIsSidebarContent))]
        fn cocoa_ui_is_sidebar_content(&self) -> bool {
            self.ivars().sidebar.get()
        }

        // SAFETY: see above — `setValue:forKey:` resolves this setter for
        // the `cocoaUiIsSidebarContent` key.
        #[unsafe(method(setCocoaUiIsSidebarContent:))]
        fn set_cocoa_ui_is_sidebar_content(&self, sidebar: bool) {
            self.set_sidebar(sidebar);
        }
    }
);

impl TableView {
    /// A list-shaped table: one `content` column, no header, `fullWidth`
    /// rows, zero intercell spacing, text-background fill, autohiding
    /// vertical scroller, and a 10pt top content inset that scrolls away.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableViewIvars::default());
        // SAFETY: standard `NSScrollView` init on a main-thread class.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };

        let table = NSTableView::new(mtm);
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(mtm),
            &NSString::from_str(COLUMN_IDENTIFIER),
        );
        table.addTableColumn(&column);
        table.setHeaderView(None);
        // SAFETY: `this` implements both protocols; the table stores its
        // data source and delegate weakly, as `AppKit` documents.
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(&*this)));
            table.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        }
        table.setStyle(NSTableViewStyle::FullWidth);
        table.setIntercellSpacing(NSSize::ZERO);
        table.setBackgroundColor(&NSColor::textBackgroundColor());
        table.setSelectionHighlightStyle(NSTableViewSelectionHighlightStyle::Regular);

        this.setDocumentView(Some(&table));
        this.setHasVerticalScroller(true);
        this.setAutohidesScrollers(true);
        this.setDrawsBackground(false);
        this.setAutomaticallyAdjustsContentInsets(false);
        this.setContentInsets(NSEdgeInsets {
            top: 10.0,
            left: 0.0,
            bottom: 0.0,
            right: 0.0,
        });
        this.ivars().table.replace(Some(table));
        this
    }

    /// The wrapped `NSTableView` — selection, row reload and drop-target
    /// calls go through it.
    ///
    /// # Panics
    ///
    /// Never after construction; the table is installed by `new`.
    #[must_use]
    pub fn table_view(&self) -> Retained<NSTableView> {
        self.ivars()
            .table
            .borrow()
            .as_ref()
            .expect("TableView table installed at construction")
            .clone()
    }

    /// Installs the row provider the delegate methods forward to.
    pub fn set_source(&self, source: Rc<dyn TableSource>) {
        self.ivars().source.replace(Some(source));
    }

    /// Registers `types` as drop targets and makes `types[0]` the pasteboard
    /// type drag payloads are written under; also enables gap feedback.
    pub fn set_drag_types(&self, types: &[&str]) {
        let types: Vec<Retained<NSString>> = types.iter().map(|t| NSString::from_str(t)).collect();
        let pasteboard_types: Vec<Retained<NSPasteboardType>> = types.clone();
        let array = NSArray::from_retained_slice(&pasteboard_types);
        self.table_view().registerForDraggedTypes(&array);
        self.ivars().drag_types.replace(types);
    }

    /// Whether the table draws sidebar chrome (`clear` background,
    /// `sourceList` style). Readable from KVC as `cocoaUiIsSidebarContent`.
    pub fn set_sidebar(&self, sidebar: bool) {
        self.ivars().sidebar.set(sidebar);
        let table = self.table_view();
        if sidebar {
            table.setBackgroundColor(&NSColor::clearColor());
            table.setStyle(NSTableViewStyle::SourceList);
        } else {
            table.setBackgroundColor(&NSColor::textBackgroundColor());
            table.setStyle(NSTableViewStyle::FullWidth);
        }
        self.setDrawsBackground(false);
        table.reloadData();
    }

    /// The size `AppKit` reports this view wants — the `fittingSize` the
    /// leaf measures through.
    #[must_use]
    pub fn fitting(&self) -> crate::Size {
        self.fittingSize().into()
    }

    /// Whether the table is drawing sidebar chrome.
    #[must_use]
    pub fn is_sidebar(&self) -> bool {
        self.ivars().sidebar.get()
    }

    /// Reloads every row's data and layout.
    pub fn reload_data(&self) {
        self.table_view().reloadData();
    }

    /// Runs `handler` after every `AppKit` layout pass, replacing the
    /// previous handler.
    pub fn set_layout_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().layout.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` each time the view moves between windows, replacing
    /// the previous handler.
    pub fn set_window_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().window.replace(Some(Rc::new(handler)));
    }

    /// Marks the view as needing layout on the next pass.
    pub fn set_needs_layout(&self) {
        self.setNeedsLayout(true);
    }

    /// Runs any pending layout immediately.
    pub fn layout_if_needed(&self) {
        self.layoutSubtreeIfNeeded();
    }
}

/// The handler [`TableRowView`] asks for the separator's horizontal span.
type SeparatorHandler = Rc<dyn Fn(&TableRowView) -> (f64, f64)>;

/// The per-instance state [`TableRowView`] stores.
#[derive(Default)]
pub struct TableRowViewIvars {
    /// `(leading, trailing)` insets for the hairline; `None` draws none.
    separator: RefCell<Option<SeparatorHandler>>,
}

impl fmt::Debug for TableRowViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableRowViewIvars")
            .field("separator", &self.separator.borrow().is_some())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSTableRowView`'s designated initializer is
    // `initWithFrame:`, which `TableRowView::new` calls, and the class does
    // not implement `Drop`.
    #[unsafe(super(NSTableRowView))]
    #[name = "CocoaUiTableRowView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TableRowViewIvars]
    #[derive(Debug)]
    /// A table row view that draws a hairline separator along its bottom
    /// edge when a separator handler is installed.
    pub struct TableRowView;

    unsafe impl NSObjectProtocol for TableRowView {}

    impl TableRowView {
        /// Selection emphasis follows the window's key state: the table
        /// never takes first responder (clicks go to the row content), yet
        /// a selected row in a key window shows the accent highlight.
        #[unsafe(method(isEmphasized))]
        fn is_emphasized(&self) -> bool {
            self.window().is_some_and(|window| window.isKeyWindow())
        }

        /// Keeps `AppKit`'s row fill, then paints the hairline inside the
        /// span the separator handler reports.
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty_rect: NSRect) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), drawRect: dirty_rect] };
            let Some(handler) = self.ivars().separator.borrow().as_ref().cloned() else {
                return;
            };
            let (leading, trailing) = handler(self);
            let bounds = self.bounds();
            let hairline = 1.0 / self.window().map_or(1.0, |w| w.backingScaleFactor());
            let inset = leading.min(bounds.size.width);
            let width = (bounds.size.width - inset - trailing).max(0.0);
            // The row view is flipped: maxY is the row's bottom edge, which
            // is where a boundary separator belongs.
            let rect = NSRect::new(
                NSPoint::new(inset, bounds.origin.y + bounds.size.height - hairline),
                NSSize::new(width, hairline),
            );
            NSColor::separatorColor().setFill();
            NSRectFill(rect);
        }
    }
);

impl TableRowView {
    /// An empty row view.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableRowViewIvars::default());
        // SAFETY: standard `NSTableRowView` init on a main-thread class.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// Installs the handler `drawRect:` asks for the separator's `(leading,
    /// trailing)` insets; installing one draws the hairline, removing it
    /// draws none.
    pub fn set_separator_handler(&self, handler: Option<SeparatorHandler>) {
        self.ivars().separator.replace(handler);
        self.setNeedsDisplay(true);
    }
}

/// An inline delete button a [`RowContainer`] presents: its title and the
/// closure a press fires.
pub struct DeleteButton {
    /// The button's title.
    pub title: String,
    /// What a press fires.
    pub handler: Box<dyn Fn()>,
}

impl fmt::Debug for DeleteButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeleteButton")
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// The handler [`RowContainer`] runs after every `AppKit` layout pass.
type RowLayoutHandler = Rc<dyn Fn(&RowContainer)>;

/// The per-instance state [`RowContainer`] stores.
#[derive(Default)]
pub struct RowContainerIvars {
    /// The mounted content view.
    content: RefCell<Option<Retained<NSView>>>,
    /// The constraints pinning the content (and button) into the row.
    constraints: RefCell<Vec<Retained<NSLayoutConstraint>>>,
    /// The delete button while it is shown.
    delete_button: RefCell<Option<Retained<NSButton>>>,
    /// The button's action target; lives as long as the button does.
    delete_target: RefCell<Option<crate::ActionTarget>>,
    /// Called after every `AppKit` layout pass on the row.
    layout: RefCell<Option<RowLayoutHandler>>,
    /// Whatever the consumer keeps alive with the row (mounted leaf,
    /// watcher guards).
    payload: RefCell<Option<Box<dyn Any>>>,
}

impl fmt::Debug for RowContainerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowContainerIvars")
            .field("content", &self.content.borrow().is_some())
            .field("delete_button", &self.delete_button.borrow().is_some())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `RowContainer::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiTableRowContainer"]
    #[thread_kind = MainThreadOnly]
    #[ivars = RowContainerIvars]
    #[derive(Debug)]
    /// A row's cell view: one content view pinned inside insets, optionally
    /// followed by an inline delete button.
    pub struct RowContainer;

    unsafe impl NSObjectProtocol for RowContainer {}

    impl RowContainer {
        /// A resize marks the row for a fresh layout pass, where the
        /// layout handler sees the constraint-resolved content slot.
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, new_size: NSSize) {
            guarded("RowContainer setFrameSize", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setFrameSize: new_size] };
                self.setNeedsLayout(true);
            });
        }

        /// Keeps `AppKit`'s layout, then reports it so the consumer can
        /// track the content slot's width.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            guarded("RowContainer layout", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layout] };
                if let Some(handler) = self.ivars().layout.borrow().as_ref().cloned() {
                    handler(self);
                }
            });
        }
    }
);

impl RowContainer {
    /// An empty row container.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RowContainerIvars::default());
        // SAFETY: standard `NSView` init on a main-thread class.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// Mounts `content` pinned inside `insets`. When `delete` is `Some`, an
    /// inline delete button with its `title` sits after the content (which
    /// then takes no trailing inset) and `handler` fires on press.
    pub fn configure(
        &self,
        mtm: MainThreadMarker,
        content: &NSView,
        insets: EdgeInsets,
        delete: Option<DeleteButton>,
    ) {
        let ivars = self.ivars();
        if let Some(old) = ivars.content.borrow_mut().take() {
            old.removeFromSuperview();
        }
        if let Some(old) = ivars.delete_button.borrow_mut().take() {
            old.removeFromSuperview();
        }
        ivars.delete_target.replace(None);
        NSLayoutConstraint::deactivateConstraints(&NSArray::from_retained_slice(
            &ivars.constraints.replace(Vec::new()),
        ));

        content.setTranslatesAutoresizingMaskIntoConstraints(false);
        self.addSubview(content);
        ivars.content.replace(Some(content.retain()));

        let mut constraints = vec![
            content
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&self.leadingAnchor(), insets.left),
            content
                .topAnchor()
                .constraintEqualToAnchor_constant(&self.topAnchor(), insets.top),
            content
                .bottomAnchor()
                .constraintEqualToAnchor_constant(&self.bottomAnchor(), -insets.bottom),
        ];

        if let Some(delete) = delete {
            let DeleteButton { title, handler } = delete;
            // SAFETY: `target`/`action` are `None`, so the button stores no
            // pointer that could dangle.
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&title),
                    None,
                    None,
                    mtm,
                )
            };
            // SAFETY: `.inline` is deprecated in favor of accessory bar styles, but
            // it is the bezel a list row's delete control takes.
            #[allow(deprecated)]
            button.setBezelStyle(NSBezelStyle::Inline);
            button.setTranslatesAutoresizingMaskIntoConstraints(false);
            self.addSubview(&button);
            ivars.delete_button.replace(Some(button.clone()));
            constraints.extend([
                content
                    .trailingAnchor()
                    .constraintEqualToAnchor_constant(&button.leadingAnchor(), -8.0),
                button
                    .trailingAnchor()
                    .constraintEqualToAnchor_constant(&self.trailingAnchor(), -8.0),
                button
                    .centerYAnchor()
                    .constraintEqualToAnchor(&self.centerYAnchor()),
            ]);
            ivars
                .delete_target
                .replace(Some(crate::ActionTarget::new(&button, move |_| handler())));
        } else {
            constraints.push(
                content
                    .trailingAnchor()
                    .constraintEqualToAnchor_constant(&self.trailingAnchor(), -insets.right),
            );
        }

        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&constraints));
        ivars.constraints.replace(constraints);
    }

    /// Runs `handler` after every `AppKit` layout pass, replacing the
    /// previous handler.
    pub fn set_layout_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().layout.replace(Some(Rc::new(handler)));
    }

    /// Stores `value` in the row; the previous payload is dropped.
    pub fn set_payload(&self, value: Box<dyn Any>) {
        self.ivars().payload.replace(Some(value));
    }

    /// The content view currently mounted, if any.
    #[must_use]
    pub fn content(&self) -> Option<Retained<NSView>> {
        self.ivars().content.borrow().as_ref().cloned()
    }
}

fn index_set(indexes: &[usize]) -> Retained<NSIndexSet> {
    let set = NSMutableIndexSet::new();
    for &index in indexes {
        set.addIndex(index);
    }
    set.into_super()
}

impl TableView {
    /// Applies `deletes` and `inserts` as one update block — `.effectFade`
    /// when `animated`, no animation otherwise.
    pub fn apply_row_updates(&self, deletes: &[usize], inserts: &[usize], animated: bool) {
        let animation = if animated {
            NSTableViewAnimationOptions::EffectFade
        } else {
            NSTableViewAnimationOptions::EffectNone
        };
        let table = self.table_view();
        table.beginUpdates();
        table.removeRowsAtIndexes_withAnimation(&index_set(deletes), animation);
        table.insertRowsAtIndexes_withAnimation(&index_set(inserts), animation);
        table.endUpdates();
    }

    /// The flat row indexes `AppKit` reports selected, ascending.
    #[must_use]
    pub fn selected_rows(&self) -> Vec<usize> {
        let indexes = self.table_view().selectedRowIndexes();
        let mut rows = Vec::with_capacity(indexes.count());
        let mut index = indexes.firstIndex();
        while index != NSNotFound.cast_unsigned() {
            rows.push(index);
            index = indexes.indexGreaterThanIndex(index);
        }
        rows
    }

    /// Makes `rows` the selection: deselects each selected row not in it,
    /// then extends the selection by it.
    pub fn set_selected_rows(&self, rows: &[usize]) {
        let table = self.table_view();
        let wanted: std::collections::BTreeSet<usize> = rows.iter().copied().collect();
        for row in self.selected_rows() {
            if !wanted.contains(&row) {
                table.deselectRow(row.cast_signed());
            }
        }
        table.selectRowIndexes_byExtendingSelection(&index_set(rows), true);
    }

    /// Whether `row` is selected.
    #[must_use]
    pub fn is_row_selected(&self, row: usize) -> bool {
        self.table_view().isRowSelected(row.cast_signed())
    }

    /// How many flat rows the table holds.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.table_view().numberOfRows().cast_unsigned()
    }

    /// The frame `row` occupies in table coordinates.
    #[must_use]
    pub fn rect_of_row(&self, row: usize) -> NSRect {
        self.table_view().rectOfRow(row.cast_signed())
    }

    /// Tells the table to re-ask heights for `rows`.
    pub fn note_height_changed(&self, rows: std::ops::Range<usize>) {
        let set = NSIndexSet::indexSetWithIndexesInRange(NSRange::new(rows.start, rows.len()));
        self.table_view().noteHeightOfRowsWithIndexesChanged(&set);
    }

    /// Scrolls `row`'s top edge to the clip's top, unanimated.
    pub fn scroll_row_to_top(&self, row: usize) {
        self.layoutSubtreeIfNeeded();
        let top = self.rect_of_row(row).origin.y;
        let clip = self.contentView();
        clip.scrollToPoint(NSPoint::new(0.0, top));
        self.reflectScrolledClipView(&clip);
    }

    /// Whether the scroll view is in a window.
    #[must_use]
    pub fn in_window(&self) -> bool {
        self.window().is_some()
    }

    /// Runs `handler` once per row view currently materialized.
    pub fn enumerate_row_views(&self, handler: impl Fn(&NSTableRowView) + 'static) {
        let block = block2::RcBlock::new(
            move |row_view: NonNull<NSTableRowView>, _index: NSInteger| {
                // SAFETY: `AppKit` lends a live row view for the block's
                // duration.
                unsafe { handler(row_view.as_ref()) };
            },
        );
        self.table_view()
            .enumerateAvailableRowViewsUsingBlock(&block);
    }
}

/// The per-instance state [`SectionHeader`] stores.
#[derive(Default)]
pub struct SectionHeaderIvars {
    label: RefCell<Option<Retained<NSTextField>>>,
    constraints: RefCell<Vec<Retained<NSLayoutConstraint>>>,
    payload: RefCell<Option<Box<dyn Any>>>,
}

impl fmt::Debug for SectionHeaderIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SectionHeaderIvars")
            .field("label", &self.label.borrow().is_some())
            .finish_non_exhaustive()
    }
}

/// Which band a [`SectionHeader`] presents: a section's header or its
/// footer — the choice changes the band's vertical margins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    /// The band above a section's rows.
    Header,
    /// The band below a section's rows.
    Footer,
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `SectionHeader::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUiTableSectionHeader"]
    #[ivars = SectionHeaderIvars]
    /// A section header/footer band: an `NSTextField` label pinned with the
    /// margins a list section takes, styled by the consumer.
    pub struct SectionHeader;
);

impl SectionHeader {
    /// Creates an empty band; `kind` picks the top margin a header (14) or
    /// footer (6) draws.
    pub fn new(mtm: MainThreadMarker, kind: SectionKind) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SectionHeaderIvars::default());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setTranslatesAutoresizingMaskIntoConstraints(false);
        this.addSubview(&label);
        let top = match kind {
            SectionKind::Header => 14.0,
            SectionKind::Footer => 6.0,
        };
        let constraints = vec![
            label
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&this.leadingAnchor(), 10.0),
            label
                .topAnchor()
                .constraintEqualToAnchor_constant(&this.topAnchor(), top),
            label
                .trailingAnchor()
                .constraintLessThanOrEqualToAnchor_constant(&this.trailingAnchor(), -16.0),
            label
                .bottomAnchor()
                .constraintLessThanOrEqualToAnchor_constant(&this.bottomAnchor(), -6.0),
        ];
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&constraints));
        this.ivars().constraints.replace(constraints);
        this.ivars().label.replace(Some(label));
        this
    }

    /// Sets the band's text.
    pub fn set_text(&self, text: &NSString) {
        if let Some(label) = self.ivars().label.borrow().as_ref() {
            label.setStringValue(text);
        }
    }

    /// Sets the band's text color.
    pub fn set_text_color(&self, color: &NSColor) {
        if let Some(label) = self.ivars().label.borrow().as_ref() {
            label.setTextColor(Some(color));
        }
    }

    /// Sets the band's font.
    pub fn set_font(&self, font: &NSFont) {
        if let Some(label) = self.ivars().label.borrow().as_ref() {
            label.setFont(Some(font));
        }
    }

    /// Stores `value` in the view; the previous payload is dropped, so
    /// watchers the consumer stores here release with the band.
    pub fn set_payload(&self, value: Box<dyn Any>) {
        *self.ivars().payload.borrow_mut() = Some(value);
    }
}
