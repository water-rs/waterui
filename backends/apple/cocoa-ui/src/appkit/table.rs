//! The `AppKit` table view: an `NSTableView` whose data source and delegate
//! are Rust closures, with its header band hosted manually.
//!
//! [`TableView`] is the leaf surface a grid container renders into: the
//! caller owns the column list (`NSTableColumn`s), answers row counts, row
//! heights and cell views through the three handlers, and frames both the
//! table and the header view it asks [`TableView::install_header`] for. The
//! header is an `NSTableHeaderView` added as a sibling of the table — the
//! layout that normally comes from an enclosing `NSScrollView` is the
//! caller's job.
//!
//! # Safety
//!
//! `unsafe` here subclasses `NSTableView`, conforms it to
//! `NSTableViewDataSource`/`NSTableViewDelegate`, forwards to `NSTableView`'s
//! own implementation where a method is overridden, and installs the object
//! as its own data source and delegate — both non-retained relationships on
//! a view the caller owns for the same lifetime. The class is
//! `MainThreadOnly` and every framework callback guards its handler call
//! with [`crate::callback::guarded`].

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSColor, NSControlTextEditingDelegate, NSTableColumn, NSTableHeaderView, NSTableView,
    NSTableViewColumnAutoresizingStyle, NSTableViewDataSource, NSTableViewDelegate,
    NSTableViewGridLineStyle, NSTableViewStyle, NSView,
};
use objc2_foundation::{NSObjectProtocol, NSRect, NSSize};

use crate::callback::guarded;

/// How many rows the table displays.
type RowCount = Rc<dyn Fn() -> usize>;
/// The height of the row at an index.
type RowHeight = Rc<dyn Fn(usize) -> f64>;
/// The view displayed for a column/row intersection, `None` for an empty
/// cell.
type CellView = Rc<dyn Fn(&NSTableColumn, usize) -> Option<Retained<NSView>>>;

/// The per-instance handlers [`TableView`] stores.
#[derive(Default)]
pub struct TableViewIvars {
    row_count: RefCell<Option<RowCount>>,
    row_height: RefCell<Option<RowHeight>>,
    cell_view: RefCell<Option<CellView>>,
}

impl std::fmt::Debug for TableViewIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableViewIvars").finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super(NSTableView))]
    #[name = "CocoaUiTableView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TableViewIvars]
    #[derive(Debug)]
    /// A view-based `NSTableView` that reads its contents from Rust
    /// handlers.
    pub struct TableView;

    unsafe impl NSObjectProtocol for TableView {}

    unsafe impl NSControlTextEditingDelegate for TableView {}

    unsafe impl NSTableViewDataSource for TableView {
        /// The row count the consumer reports.
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows_in_table_view(&self, _table_view: &NSTableView) -> isize {
            guarded("TableView numberOfRowsInTableView", || {
                self.ivars()
                    .row_count
                    .borrow()
                    .as_ref()
                    .map_or(0, |handler| handler().cast_signed())
            })
        }
    }

    unsafe impl NSTableViewDelegate for TableView {
        /// The row height the consumer reports.
        #[unsafe(method(tableView:heightOfRow:))]
        fn table_view_height_of_row(&self, _table_view: &NSTableView, row: isize) -> f64 {
            guarded("TableView tableView:heightOfRow:", || {
                self.ivars()
                    .row_height
                    .borrow()
                    .as_ref()
                    .map_or(0.0, |handler| handler(row.cast_unsigned()))
            })
        }

        /// The cell view the consumer answers for the intersection.
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn table_view_view_for_table_column_row(
            &self,
            _table_view: &NSTableView,
            table_column: Option<&NSTableColumn>,
            row: isize,
        ) -> Option<Retained<NSView>> {
            guarded("TableView tableView:viewForTableColumn:row:", || {
                let handler = self.ivars().cell_view.borrow().as_ref().cloned();
                handler.and_then(|handler| {
                    table_column.and_then(|column| handler(column, row.cast_unsigned()))
                })
            })
        }
    }
);

impl TableView {
    /// A table view configured for frame-managed content: inset style, no
    /// alternating row tint, no grid lines, zero intercell spacing, no column
    /// autoresizing, a clear background, and column reordering, resizing and
    /// selection disabled.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableViewIvars::default());
        // SAFETY: `initWithFrame:` is `NSTableView`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setStyle(NSTableViewStyle::Inset);
        this.setUsesAlternatingRowBackgroundColors(false);
        this.setGridStyleMask(NSTableViewGridLineStyle::empty());
        this.setIntercellSpacing(NSSize::ZERO);
        this.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::NoColumnAutoresizing);
        this.setBackgroundColor(&NSColor::clearColor());
        this.setAllowsColumnReordering(false);
        this.setAllowsColumnResizing(false);
        this.setAllowsColumnSelection(false);
        // SAFETY: `this` implements both protocols above; the data source and
        // delegate relationships are non-retained on a view the caller owns.
        unsafe {
            this.setDataSource(Some(ProtocolObject::from_ref(&*this)));
            this.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        }
        this
    }

    /// Creates the header view this table's header band renders through,
    /// binds it to the table and hands it to the caller to frame and attach
    /// as the table's sibling. `AppKit` only positions a header view inside
    /// an `NSScrollView`; hosted manually the caller lays it out itself.
    #[must_use]
    pub fn install_header(&self) -> Retained<NSTableHeaderView> {
        let header = NSTableHeaderView::new(self.mtm());
        self.setHeaderView(Some(&header));
        header
    }

    /// Sets the handler answering the row count, replacing the previous one.
    pub fn set_row_count_handler(&self, handler: impl Fn() -> usize + 'static) {
        self.ivars().row_count.replace(Some(Rc::new(handler)));
    }

    /// Sets the handler answering each row's height, replacing the previous
    /// one.
    pub fn set_row_height_handler(&self, handler: impl Fn(usize) -> f64 + 'static) {
        self.ivars().row_height.replace(Some(Rc::new(handler)));
    }

    /// Sets the handler answering the cell view for a column/row
    /// intersection, replacing the previous one.
    pub fn set_cell_view_handler(
        &self,
        handler: impl Fn(&NSTableColumn, usize) -> Option<Retained<NSView>> + 'static,
    ) {
        self.ivars().cell_view.replace(Some(Rc::new(handler)));
    }

    /// Makes the table's columns exactly `columns`, in order, removing any
    /// that are not listed.
    pub fn set_columns(&self, columns: &[Retained<NSTableColumn>]) {
        for column in &self.tableColumns() {
            self.removeTableColumn(&column);
        }
        for column in columns {
            self.addTableColumn(column);
        }
    }

    /// Asks the table to re-query its data source and delegate.
    pub fn reload_data(&self) {
        self.reloadData();
    }

    /// Asks `header` to repaint — the manually hosted header's equivalent of
    /// a column-title refresh.
    pub fn refresh_header(header: &NSTableHeaderView) {
        header.setNeedsDisplay(true);
    }

    /// Whether `column` is one of this table's columns, by identity.
    #[must_use]
    pub fn owns_column(&self, column: &NSTableColumn) -> bool {
        self.tableColumns()
            .iter()
            .any(|existing| ptr::eq(&raw const *existing, column))
    }

    /// Marks the view as needing layout on the next pass.
    pub fn set_needs_layout(&self) {
        self.setNeedsLayout(true);
    }
}
