//! The `AppKit` source list: an `NSTableView` of rows in a scroll view, in
//! the platform's sidebar style.
//!
//! The list shows one section of rows — icon, label, optional badge — with
//! the source-list row style `NSTableView` gives sidebars. Selection is
//! two-way: `select` moves the highlight and `on_select` reports the row the
//! user picked.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSTableView` subclass that is its own data
//! source and delegate, and calls `objc2`/`AppKit` bindings marked unsafe
//! because `AppKit` table APIs are main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSControlTextEditingDelegate, NSImage, NSScrollView, NSTableCellView, NSTableColumn,
    NSTableView, NSTableViewDataSource, NSTableViewDelegate, NSTableViewRowSizeStyle,
    NSTableViewStyle, NSView,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSIndexSet, NSNotification, NSObjectProtocol, NSString};

use crate::appkit::segmented::Segment;

/// Called with the selected row's index.
type SelectHandler = Rc<dyn Fn(usize)>;

/// The source list's rows and selection.
pub struct SourceListIvars {
    /// The rows' content.
    segments: RefCell<Vec<Segment>>,
    /// Called with the selected row's index.
    select: RefCell<Option<SelectHandler>>,
}

impl fmt::Debug for SourceListIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceListIvars")
            .field("segments", &self.segments.borrow())
            .field("has_select", &self.select.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSTableView`'s designated initializer is `initWithFrame:`,
    // which `SourceList::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSTableView))]
    #[name = "CocoaUiSourceListTable"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SourceListIvars]
    #[derive(Debug)]
    /// An `NSTableView` that is its own data source and delegate.
    pub struct SourceListTable;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSTableView`.
    unsafe impl NSObjectProtocol for SourceListTable {}

    impl SourceListTable {
        // SAFETY: `tableViewSelectionDidChange:` is the delegate's declared
        // signature.
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn table_view_selection_did_change(&self, _notification: &NSNotification) {
            let row = self.selectedRow();
            if row < 0 {
                return;
            }
            let handler = self.ivars().select.borrow().clone();
            if let Some(handler) = handler {
                handler(row.cast_unsigned());
            }
        }
    }

    // SAFETY: the data-source methods carry `NSTableViewDataSource`'s
    // signatures.
    unsafe impl NSTableViewDataSource for SourceListTable {
        // SAFETY: see the module safety note.
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows_in_table_view(&self, _table_view: &NSTableView) -> isize {
            self.ivars().segments.borrow().len().cast_signed()
        }
    }

    // SAFETY: the delegate methods carry `NSTableViewDelegate`'s signatures.
    unsafe impl NSControlTextEditingDelegate for SourceListTable {}

    // SAFETY: the delegate methods carry `NSTableViewDelegate`'s signatures.
    unsafe impl NSTableViewDelegate for SourceListTable {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn table_view_view_for_table_column_row(
            &self,
            _table_view: &NSTableView,
            _column: &NSTableColumn,
            row: isize,
        ) -> Option<Retained<NSView>> {
            let cell = NSTableCellView::new(self.mtm());
            let segments = self.ivars().segments.borrow();
            if let Some(segment) = segments.get(row.cast_unsigned()) {
                let label = segment.badge.as_ref().map_or_else(
                    || segment.label.clone(),
                    |badge| format!("{} ({badge})", segment.label),
                );
                // SAFETY: `textField` reads the cell's outlets.
                if let Some(field) = unsafe { cell.textField() } {
                    field.setStringValue(&NSString::from_str(&label));
                }
                let image = segment
                    .symbol
                    .as_ref()
                    .and_then(|symbol| {
                        NSImage::imageWithSystemSymbolName_accessibilityDescription(
                            &NSString::from_str(symbol),
                            None,
                        )
                    })
                    .or_else(|| segment.image.clone());
                // SAFETY: `imageView` reads the cell's outlets.
                if let (Some(image_view), Some(image)) = (unsafe { cell.imageView() }, image) {
                    image_view.setImage(Some(&image));
                }
            }
            Some(cell.into_super())
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(tableView:shouldSelectRow:))]
        fn table_view_should_select_row(
            &self,
            _table_view: &NSTableView,
            row: isize,
        ) -> bool {
            self.ivars()
                .segments
                .borrow()
                .get(row.cast_unsigned())
                .is_some_and(|segment| segment.enabled)
        }
    }
);

/// The source list: a table inside a scroll view, sidebar-styled.
#[derive(Debug)]
pub struct SourceList {
    /// The scroll view to embed — the list's platform view.
    scroll: Retained<NSScrollView>,
    /// The table inside it.
    table: Retained<SourceListTable>,
}

impl SourceList {
    /// An empty source list.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let table: Retained<SourceListTable> = {
            let this = SourceListTable::alloc(mtm).set_ivars(SourceListIvars {
                segments: RefCell::new(Vec::new()),
                select: RefCell::new(None),
            });
            // SAFETY: `initWithFrame:` is `NSTableView`'s designated
            // initializer.
            unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] }
        };
        let column = NSTableColumn::new(mtm);
        table.addTableColumn(&column);
        // SAFETY: `setHeaderView:` hides the column header.
        unsafe {
            let _: () = objc2::msg_send![&table, setHeaderView: Option::<&NSView>::None];
        }
        table.setStyle(NSTableViewStyle::SourceList);
        table.setRowSizeStyle(NSTableViewRowSizeStyle::Default);
        // SAFETY: the table conforms to both protocols; both are assign
        // references.
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(&*table)));
            table.setDelegate(Some(ProtocolObject::from_ref(&*table)));
        }

        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&table));
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        Self { scroll, table }
    }

    /// The scroll view hosting the list.
    #[must_use]
    pub fn view(&self) -> &NSScrollView {
        &self.scroll
    }

    /// Replaces the rows.
    pub fn set_segments(&self, segments: &[Segment]) {
        self.table.ivars().segments.replace(segments.to_vec());
        self.table.reloadData();
    }

    /// The selected row; `None` when nothing is selected.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        let row = self.table.selectedRow();
        (row >= 0).then_some(row.cast_unsigned())
    }

    /// Selects `index` without firing `on_select`.
    pub fn select(&self, index: Option<usize>) {
        match index {
            Some(index) => self.table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            ),
            // SAFETY: `deselectAll:` is a main-thread selection update.
            None => unsafe { self.table.deselectAll(None) },
        }
    }

    /// Runs `handler` with the selected row's index.
    pub fn set_select_handler(&self, handler: impl Fn(usize) + 'static) {
        self.table.ivars().select.replace(Some(Rc::new(handler)));
    }
}
