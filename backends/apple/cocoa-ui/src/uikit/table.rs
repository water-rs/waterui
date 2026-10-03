//! A [`UITableView`] subclass with a data-source/delegate bridge and its
//! cell and header-footer companions.
//!
//! [`TableView`] keeps the single column layout a grouped list wants —
//! `.insetGrouped` style, fixed directional margins, estimated heights, and
//! class-based reuse of [`TableCell`] and [`TableHeaderFooterView`]. All
//! `UITableView` protocol callbacks forward into a [`TableSource`] handler
//! the consumer installs, so selection, editing (commit-delete plus the
//! swipe-to-delete affordance), and row moving stay on the caller's side of
//! the abstraction.
//!
//! [`TableCell`] mounts one content view pinned inside per-row insets,
//! applies the selected background and accessibility trait itself, and fires
//! an `on_layout` hook from `layoutSubviews` so the consumer can push
//! placement proposals and resolve the separator's leading inset after
//! layout.
//!
//! # Safety
//!
//! `unsafe` here subclasses `UITableView`, `UITableViewCell`, and
//! `UITableViewHeaderFooterView`, calls `super`, and forwards `UIKit`
//! callbacks into stored `Rc` handlers. Those callbacks arrive on the main
//! thread — the classes are `MainThreadOnly` — and [`crate::callback`]
//! catches panics so none unwind into `UIKit`. Dequeued views are
//! downcast-checked rather than cast blindly, and the swipe-action block
//! copies `UIKit` arguments before use.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use block2::RcBlock;
use core::ptr::NonNull;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::runtime::ProtocolObject;
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
};
use objc2_core_foundation::{CGFloat, CGRect};
use objc2_foundation::{NSArray, NSIndexPath, NSInteger, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    NSDirectionalEdgeInsets, NSIndexPathUIKitAdditions, NSLayoutConstraint,
    NSObjectUIAccessibility, UIAccessibilityTraitSelected, UIColor, UIContextualAction,
    UIContextualActionStyle, UIEdgeInsets, UIListContentConfiguration, UIScrollViewDelegate,
    UISwipeActionsConfiguration, UITableView, UITableViewAutomaticDimension, UITableViewCell,
    UITableViewCellAccessoryType, UITableViewCellEditingStyle, UITableViewCellSelectionStyle,
    UITableViewCellStyle, UITableViewDataSource, UITableViewDelegate, UITableViewHeaderFooterView,
    UITableViewRowAnimation, UITableViewScrollPosition, UITableViewSeparatorInsetReference,
    UITableViewStyle, UIView,
};

use crate::EdgeInsets;
use crate::callback::guarded;
use crate::geometry::IndexPath;

/// Whether a header-footer view presents a section's header or footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    /// The section header shown above the rows.
    Header,
    /// The section footer shown below the rows.
    Footer,
}

/// The data source and delegate a [`TableView`] forwards `UIKit` callbacks
/// into.
///
/// Coordinates arrive as [`IndexPath`], a two-component `(section, row)`
/// pair.
pub trait TableSource: 'static {
    /// The number of sections the table shows.
    fn sections(&self, table: &TableView) -> usize;

    /// The number of rows `section` shows.
    fn rows_in_section(&self, table: &TableView, section: usize) -> usize;

    /// `UIKit` wants a configured cell for `index`.
    fn configure_cell(&self, table: &TableView, cell: &TableCell, index: IndexPath);

    /// `UIKit` wants a configured header/footer for `section`; return
    /// whether `view` should be shown.
    fn configure_header_footer(
        &self,
        table: &TableView,
        view: &TableHeaderFooterView,
        kind: SectionKind,
        section: usize,
    ) -> bool;

    /// The height `index`'s row takes.
    fn row_height(&self, table: &TableView, index: IndexPath) -> f64;

    /// The height `section`'s header takes. `f64::NAN` (the default) maps
    /// to `UITableViewAutomaticDimension`.
    fn section_header_height(&self, table: &TableView, section: usize) -> f64 {
        let _ = (table, section);
        f64::NAN
    }

    /// The height `section`'s footer takes. `f64::NAN` (the default) maps
    /// to `UITableViewAutomaticDimension`.
    fn section_footer_height(&self, table: &TableView, section: usize) -> f64 {
        let _ = (table, section);
        f64::NAN
    }

    /// Whether `index`'s row may enter the delete affordance — both the
    /// edit-mode minus control and the trailing swipe action consult it.
    /// The default reports no row deletable.
    fn is_row_deletable(&self, table: &TableView, index: IndexPath) -> bool {
        let _ = (table, index);
        false
    }

    /// `index`'s row was deleted through editing or the swipe affordance.
    fn delete_row(&self, table: &TableView, index: IndexPath) {
        let _ = (table, index);
    }

    /// Whether `index`'s row participates in move reordering. The default
    /// reports no row movable.
    fn can_move_row(&self, table: &TableView, index: IndexPath) -> bool {
        let _ = (table, index);
        false
    }

    /// `UIKit` moved the row at `from` to `to`.
    fn move_row(&self, table: &TableView, from: IndexPath, to: IndexPath) {
        let _ = (table, from, to);
    }

    /// `index`'s row became selected by the user.
    fn did_select_row(&self, table: &TableView, index: IndexPath);

    /// `index`'s row became deselected by the user.
    fn did_deselect_row(&self, table: &TableView, index: IndexPath);
}

impl fmt::Debug for dyn TableSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableSource").finish_non_exhaustive()
    }
}

/// The per-instance state [`TableView`] stores.
#[derive(Default)]
pub struct TableViewIvars {
    source: RefCell<Option<Rc<dyn TableSource>>>,
}

impl fmt::Debug for TableViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableViewIvars")
            .field("source", &self.source.borrow().is_some())
            .finish()
    }
}

impl TableViewIvars {
    fn source(&self) -> Option<Rc<dyn TableSource>> {
        self.source.borrow().clone()
    }
}

/// Translates a source's `f64::NAN` into `UITableViewAutomaticDimension`.
trait NanOrAutomatic {
    fn or_nan_to_automatic(self) -> CGFloat;
}

impl NanOrAutomatic for f64 {
    fn or_nan_to_automatic(self) -> CGFloat {
        if self.is_nan() {
            // SAFETY: reading an extern `CGFloat` constant is side-effect
            // free.
            unsafe { UITableViewAutomaticDimension }
        } else {
            self
        }
    }
}

const CELL_REUSE_IDENTIFIER: &str = "CocoaUiTableCell";
const HEADER_REUSE_IDENTIFIER: &str = "CocoaUiTableSectionHeader";
const FOOTER_REUSE_IDENTIFIER: &str = "CocoaUiTableSectionFooter";

fn index_path(index: IndexPath) -> Retained<NSIndexPath> {
    NSIndexPath::indexPathForRow_inSection(index.row.cast_signed(), index.section.cast_signed())
}

fn table_index(index_path: &NSIndexPath) -> IndexPath {
    IndexPath {
        section: index_path.section().cast_unsigned(),
        row: index_path.row().cast_unsigned(),
    }
}

define_class!(
    // SAFETY: `UITableView`'s designated initializer is
    // `initWithFrame:style:`, which `TableView::new` calls, and the class
    // does not implement `Drop`.
    #[unsafe(super(UITableView))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUiTableView"]
    #[ivars = TableViewIvars]
    /// A single-column `.insetGrouped` table whose data source and delegate
    /// forward into a [`TableSource`].
    pub struct TableView;

    unsafe impl NSObjectProtocol for TableView {}

    // SAFETY: `UITableView`'s scroll-view delegate methods are all optional.
    unsafe impl UIScrollViewDelegate for TableView {}

    // SAFETY: `UITableViewDataSource`'s required methods are implemented and
    // every supplied method forwards to the installed `TableSource`.
    unsafe impl UITableViewDataSource for TableView {
        #[unsafe(method(numberOfSectionsInTableView:))]
        fn number_of_sections(&self, _table_view: &UITableView) -> NSInteger {
            let sections = guarded("uikit::table::source.sections", || {
                self.ivars()
                    .source()
                    .map_or(0, |source| source.sections(self))
            });
            sections.cast_signed()
        }

        #[unsafe(method(tableView:numberOfRowsInSection:))]
        fn number_of_rows(&self, _table_view: &UITableView, section: NSInteger) -> NSInteger {
            let rows = guarded("uikit::table::source.rows_in_section", || {
                self.ivars().source().map_or(0, |source| {
                    source.rows_in_section(self, section.cast_unsigned())
                })
            });
            rows.cast_signed()
        }

        #[unsafe(method_id(tableView:cellForRowAtIndexPath:))]
        fn cell_for_row(
            &self,
            _table_view: &UITableView,
            index_path: &NSIndexPath,
        ) -> Retained<UITableViewCell> {
            let cell = self.dequeue_cell(index_path);
            guarded("uikit::table::source.configure_cell", || {
                if let Some(source) = self.ivars().source() {
                    source.configure_cell(self, &cell, table_index(index_path));
                }
            });
            cell.into_super()
        }

        #[unsafe(method(tableView:canEditRowAtIndexPath:))]
        fn can_edit_row(&self, _table_view: &UITableView, index_path: &NSIndexPath) -> Bool {
            let deletable = guarded("uikit::table::source.is_row_deletable", || {
                self.ivars()
                    .source()
                    .is_some_and(|source| source.is_row_deletable(self, table_index(index_path)))
            });
            Bool::new(deletable)
        }

        #[unsafe(method(tableView:canMoveRowAtIndexPath:))]
        fn can_move_row(&self, _table_view: &UITableView, index_path: &NSIndexPath) -> Bool {
            let movable = guarded("uikit::table::source.can_move_row", || {
                self.ivars()
                    .source()
                    .is_some_and(|source| source.can_move_row(self, table_index(index_path)))
            });
            Bool::new(movable)
        }

        #[unsafe(method(tableView:commitEditingStyle:forRowAtIndexPath:))]
        fn commit_editing_style(
            &self,
            _table_view: &UITableView,
            editing_style: UITableViewCellEditingStyle,
            index_path: &NSIndexPath,
        ) {
            if editing_style != UITableViewCellEditingStyle::Delete {
                return;
            }
            guarded("uikit::table::source.delete_row", || {
                if let Some(source) = self.ivars().source() {
                    source.delete_row(self, table_index(index_path));
                }
            });
        }

        #[unsafe(method(tableView:moveRowAtIndexPath:toIndexPath:))]
        fn move_row(
            &self,
            _table_view: &UITableView,
            source_index_path: &NSIndexPath,
            destination_index_path: &NSIndexPath,
        ) {
            guarded("uikit::table::source.move_row", || {
                if let Some(source) = self.ivars().source() {
                    source.move_row(
                        self,
                        table_index(source_index_path),
                        table_index(destination_index_path),
                    );
                }
            });
        }
    }

    // SAFETY: `UITableViewDelegate`'s methods are all optional; the supplied
    // ones forward to the installed `TableSource`.
    unsafe impl UITableViewDelegate for TableView {
        #[unsafe(method_id(tableView:viewForHeaderInSection:))]
        fn view_for_header(
            &self,
            _table_view: &UITableView,
            section: NSInteger,
        ) -> Option<Retained<UIView>> {
            self.header_footer(SectionKind::Header, section)
                .map(|view| view.into_super().into_super())
        }

        #[unsafe(method_id(tableView:viewForFooterInSection:))]
        fn view_for_footer(
            &self,
            _table_view: &UITableView,
            section: NSInteger,
        ) -> Option<Retained<UIView>> {
            self.header_footer(SectionKind::Footer, section)
                .map(|view| view.into_super().into_super())
        }

        #[unsafe(method(tableView:heightForRowAtIndexPath:))]
        fn height_for_row(&self, _table_view: &UITableView, index_path: &NSIndexPath) -> CGFloat {
            guarded("uikit::table::source.row_height", || {
                self.ivars().source().map_or(0.0, |source| {
                    source.row_height(self, table_index(index_path))
                })
            })
        }

        #[unsafe(method(tableView:heightForHeaderInSection:))]
        fn height_for_header(&self, _table_view: &UITableView, section: NSInteger) -> CGFloat {
            guarded("uikit::table::source.section_header_height", || {
                self.ivars()
                    .source()
                    .map_or(f64::NAN, |source| {
                        source.section_header_height(self, section.cast_unsigned())
                    })
                    .or_nan_to_automatic()
            })
        }

        #[unsafe(method(tableView:heightForFooterInSection:))]
        fn height_for_footer(&self, _table_view: &UITableView, section: NSInteger) -> CGFloat {
            guarded("uikit::table::source.section_footer_height", || {
                self.ivars()
                    .source()
                    .map_or(f64::NAN, |source| {
                        source.section_footer_height(self, section.cast_unsigned())
                    })
                    .or_nan_to_automatic()
            })
        }

        #[unsafe(method(tableView:editingStyleForRowAtIndexPath:))]
        fn editing_style_for_row(
            &self,
            _table_view: &UITableView,
            index_path: &NSIndexPath,
        ) -> UITableViewCellEditingStyle {
            let deletable = guarded("uikit::table::source.is_row_deletable", || {
                self.ivars()
                    .source()
                    .is_some_and(|source| source.is_row_deletable(self, table_index(index_path)))
            });
            if deletable {
                UITableViewCellEditingStyle::Delete
            } else {
                UITableViewCellEditingStyle::None
            }
        }

        #[unsafe(method_id(tableView:trailingSwipeActionsConfigurationForRowAtIndexPath:))]
        fn trailing_swipe_actions(
            &self,
            _table_view: &UITableView,
            index_path: &NSIndexPath,
        ) -> Option<Retained<UISwipeActionsConfiguration>> {
            let deletable = guarded("uikit::table::source.is_row_deletable", || {
                self.ivars()
                    .source()
                    .is_some_and(|source| source.is_row_deletable(self, table_index(index_path)))
            });
            if deletable {
                Some(self.swipe_configuration(table_index(index_path)))
            } else {
                None
            }
        }

        #[unsafe(method(tableView:didSelectRowAtIndexPath:))]
        fn did_select_row(&self, _table_view: &UITableView, index_path: &NSIndexPath) {
            guarded("uikit::table::source.did_select_row", || {
                if let Some(source) = self.ivars().source() {
                    source.did_select_row(self, table_index(index_path));
                }
            });
        }

        #[unsafe(method(tableView:didDeselectRowAtIndexPath:))]
        fn did_deselect_row(&self, _table_view: &UITableView, index_path: &NSIndexPath) {
            guarded("uikit::table::source.did_deselect_row", || {
                if let Some(source) = self.ivars().source() {
                    source.did_deselect_row(self, table_index(index_path));
                }
            });
        }
    }
);

impl TableView {
    /// Creates a table styled `.insetGrouped` with fixed 16-point horizontal
    /// directional margins, `fromCellEdges` separator references, and the
    /// estimated heights a plain grouped row takes.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableViewIvars::default());
        // SAFETY: `initWithFrame:style:` is the designated initializer.
        let this: Retained<Self> = unsafe {
            msg_send![super(this), initWithFrame: CGRect::ZERO, style: UITableViewStyle::InsetGrouped]
        };
        this.setCellLayoutMarginsFollowReadableWidth(false);
        let mut margins = this.directionalLayoutMargins();
        margins.leading = 16.0;
        margins.trailing = 16.0;
        this.setDirectionalLayoutMargins(margins);
        this.setSeparatorInsetReference(UITableViewSeparatorInsetReference::FromCellEdges);
        // SAFETY: `TableCell` and `TableHeaderFooterView` are the classes the
        // identifiers dequeue.
        unsafe {
            this.registerClass_forCellReuseIdentifier(
                Some(TableCell::class()),
                &NSString::from_str(CELL_REUSE_IDENTIFIER),
            );
            this.registerClass_forHeaderFooterViewReuseIdentifier(
                Some(TableHeaderFooterView::class()),
                &NSString::from_str(HEADER_REUSE_IDENTIFIER),
            );
            this.registerClass_forHeaderFooterViewReuseIdentifier(
                Some(TableHeaderFooterView::class()),
                &NSString::from_str(FOOTER_REUSE_IDENTIFIER),
            );
        }
        this.setEstimatedSectionHeaderHeight(28.0);
        this.setEstimatedSectionFooterHeight(24.0);
        this.setEstimatedRowHeight(44.0);
        this.setDataSource(Some(ProtocolObject::from_ref(&*this)));
        // SAFETY: `TableView` implements `UITableViewDelegate`.
        unsafe { this.setDelegate(Some(ProtocolObject::from_ref(&*this))) };
        this
    }

    /// Installs the data source and delegate handler.
    pub fn set_source(&self, source: Rc<dyn TableSource>) {
        self.ivars().source.replace(Some(source));
    }

    /// The row insets a stock list cell takes, from
    /// `UIListContentConfiguration.cell()`'s layout margins.
    #[must_use]
    pub fn theme_row_insets(mtm: MainThreadMarker) -> EdgeInsets {
        let margins = UIListContentConfiguration::cellConfiguration(mtm).directionalLayoutMargins();
        EdgeInsets {
            top: margins.top,
            bottom: margins.bottom,
            left: margins.leading,
            right: margins.trailing,
        }
    }

    /// The default height a stock `UITableViewCell` reports — the minimum a
    /// row takes when `min_row_height` is unset, measured the way a live
    /// table's self-sizing pass measures it: `sizeThatFits` on a detached
    /// cell reports a point less than `systemLayoutSizeFittingSize`, the
    /// measurement `UITableView` actually applies.
    #[must_use]
    pub fn stock_row_height(&self) -> f64 {
        crate::view::fitting_size(&TableCell::new(self.mtm())).height
    }

    /// The table's directional layout margins.
    pub fn directional_margins(&self) -> NSDirectionalEdgeInsets {
        self.directionalLayoutMargins()
    }

    /// The width the table's bounds offer a row.
    pub fn bounds_width(&self) -> f64 {
        self.bounds().size.width
    }

    /// Whether the table is in a window.
    pub fn in_window(&self) -> bool {
        self.window().is_some()
    }

    /// `sectionHeaderTopPadding` (iOS 15+): the padding between the
    /// adjusted top inset and the first section's header. `0` removes the
    /// `.insetGrouped` stock reserve above the first card.
    pub fn set_section_header_top_padding(&self, padding: f64) {
        self.setSectionHeaderTopPadding(padding);
    }

    /// Enters or leaves editing mode.
    pub fn set_editing(&self, editing: bool, animated: bool) {
        self.setEditing_animated(editing, animated);
    }

    /// Whether rows accept taps as selection.
    pub fn set_allows_selection(&self, allows: bool) {
        self.setAllowsSelection(allows);
    }

    /// Whether several rows may stay selected at once.
    pub fn set_allows_multiple_selection(&self, allows: bool) {
        self.setAllowsMultipleSelection(allows);
    }

    /// Re-reads every cell, header, footer, and height.
    pub fn reload_data(&self) {
        self.reloadData();
    }

    /// Applies `deletes` and `inserts` as one animated batch — `.automatic`
    /// when `animated`, `.none` otherwise.
    pub fn apply_row_updates(&self, deletes: &[IndexPath], inserts: &[IndexPath], animated: bool) {
        let animation = if animated {
            UITableViewRowAnimation::Automatic
        } else {
            UITableViewRowAnimation::None
        };
        let delete_array = NSArray::from_retained_slice(
            &deletes.iter().map(|i| index_path(*i)).collect::<Vec<_>>(),
        );
        let insert_array = NSArray::from_retained_slice(
            &inserts.iter().map(|i| index_path(*i)).collect::<Vec<_>>(),
        );
        let table = self.retain();
        let updates = RcBlock::new(move || {
            table.deleteRowsAtIndexPaths_withRowAnimation(&delete_array, animation);
            table.insertRowsAtIndexPaths_withRowAnimation(&insert_array, animation);
        });
        // `performBatchUpdates` invokes `updates` synchronously, so the
        // captured `NSArray`s stay alive for the whole batch.
        self.performBatchUpdates_completion(Some(&updates), None);
    }

    /// The rows `UIKit` reports selected.
    pub fn selected_index_paths(&self) -> Vec<IndexPath> {
        self.indexPathsForSelectedRows()
            .map_or_else(Vec::new, |paths| {
                paths.iter().map(|p| table_index(&p)).collect()
            })
    }

    /// Reloads `indexes` — `UIKit` re-asks each row's data in place.
    pub fn reload_rows(&self, indexes: &[IndexPath], animated: bool) {
        let animation = if animated {
            UITableViewRowAnimation::Automatic
        } else {
            UITableViewRowAnimation::None
        };
        let paths = NSArray::from_retained_slice(
            &indexes.iter().map(|i| index_path(*i)).collect::<Vec<_>>(),
        );
        self.reloadRowsAtIndexPaths_withRowAnimation(&paths, animation);
    }

    /// The index path `cell` is placed at, if it is on screen.
    #[must_use]
    pub fn index_path_for_cell(&self, cell: &TableCell) -> Option<IndexPath> {
        self.indexPathForCell(cell).map(|p| table_index(&p))
    }

    /// Selects `index` without scrolling to it.
    pub fn select_row(&self, index: IndexPath, animated: bool) {
        self.selectRowAtIndexPath_animated_scrollPosition(
            Some(&index_path(index)),
            animated,
            UITableViewScrollPosition::None,
        );
    }

    /// Deselects `index`.
    pub fn deselect_row(&self, index: IndexPath, animated: bool) {
        self.deselectRowAtIndexPath_animated(&index_path(index), animated);
    }

    /// Scrolls `index`'s row to the top of the visible rect, unanimated.
    pub fn scroll_to_row(&self, index: IndexPath) {
        self.scrollToRowAtIndexPath_atScrollPosition_animated(
            &index_path(index),
            UITableViewScrollPosition::Top,
            false,
        );
    }

    /// Lays out subviews immediately.
    pub fn layout_if_needed(&self) {
        self.layoutIfNeeded();
    }

    fn dequeue_cell(&self, index_path: &NSIndexPath) -> Retained<TableCell> {
        let cell = self.dequeueReusableCellWithIdentifier_forIndexPath(
            &NSString::from_str(CELL_REUSE_IDENTIFIER),
            index_path,
        );
        cell.downcast::<TableCell>()
            .expect("registered cell class is TableCell")
    }

    fn header_footer(
        &self,
        kind: SectionKind,
        section: NSInteger,
    ) -> Option<Retained<TableHeaderFooterView>> {
        let identifier = match kind {
            SectionKind::Header => HEADER_REUSE_IDENTIFIER,
            SectionKind::Footer => FOOTER_REUSE_IDENTIFIER,
        };
        let view = self
            .dequeueReusableHeaderFooterViewWithIdentifier(&NSString::from_str(identifier))?
            .downcast::<TableHeaderFooterView>()
            .expect("registered header-footer class is TableHeaderFooterView");
        let shown = guarded("uikit::table::source.configure_header_footer", || {
            self.ivars().source().is_some_and(|source| {
                source.configure_header_footer(self, &view, kind, section.cast_unsigned())
            })
        });
        shown.then_some(view)
    }

    fn swipe_configuration(&self, index: IndexPath) -> Retained<UISwipeActionsConfiguration> {
        let mtm = self.mtm();
        let this = self.retain();
        let block = RcBlock::new(
            move |_action: NonNull<UIContextualAction>,
                  _view: NonNull<UIView>,
                  completion: NonNull<block2::DynBlock<dyn Fn(Bool)>>| {
                guarded("uikit::table::source.delete_row", || {
                    if let Some(source) = this.ivars().source() {
                        source.delete_row(&this, index);
                    }
                });
                // SAFETY: `completion` is a live block UIKit handed the
                // action; it stays valid for this call.
                unsafe { completion.as_ref() }.call((Bool::YES,));
            },
        );
        // SAFETY: `handler` takes a block the call copies, so the stack
        // block may be dropped once the call returns.
        let action = unsafe {
            UIContextualAction::contextualActionWithStyle_title_handler(
                UIContextualActionStyle::Destructive,
                Some(&NSString::from_str("Delete")),
                RcBlock::as_ptr(&block).cast(),
                mtm,
            )
        };
        let actions = NSArray::from_retained_slice(&[action]);
        UISwipeActionsConfiguration::configurationWithActions(&actions, mtm)
    }
}

/// A hook a [`TableCell`] fires back into its consumer.
type TableCellHandler = Rc<dyn Fn(&TableCell)>;

/// The per-instance state [`TableCell`] stores.
#[derive(Default)]
pub struct TableCellIvars {
    content: RefCell<Option<Retained<UIView>>>,
    constraints: RefCell<Vec<Retained<NSLayoutConstraint>>>,
    on_layout: RefCell<Option<TableCellHandler>>,
    on_activate: RefCell<Option<TableCellHandler>>,
    payload: RefCell<Option<Box<dyn Any>>>,
}

impl fmt::Debug for TableCellIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableCellIvars")
            .field("content", &self.content.borrow().is_some())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UITableViewCell`'s designated initializer for reuse pools is
    // `initWithStyle:reuseIdentifier:`, which `TableCell::new` calls, and the
    // class does not implement `Drop`.
    #[unsafe(super(UITableViewCell))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUiTableCell"]
    #[ivars = TableCellIvars]
    /// A cell mounting one content view inside per-row insets.
    pub struct TableCell;

    unsafe impl NSObjectProtocol for TableCell {}

    impl TableCell {
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews(&self) {
            // SAFETY: the super implementation keeps the cell's chrome laid
            // out before the content hook runs.
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            let handler = self.cell_ivars().on_layout.borrow().clone();
            if let Some(handler) = handler {
                guarded("uikit::table::cell.on_layout", || handler(self));
            }
        }

        #[unsafe(method(prepareForReuse))]
        fn prepare_for_reuse(&self) {
            // SAFETY: the super implementation performs the system's reuse
            // bookkeeping before the payload drops.
            let _: () = unsafe { msg_send![super(self), prepareForReuse] };
            self.cell_ivars().payload.take();
        }

        #[unsafe(method(setSelected:animated:))]
        fn set_selected(&self, selected: bool, animated: bool) {
            // SAFETY: the super implementation applies the system's own
            // selected chrome (none, since selection style is `.none`).
            let _: () = unsafe { msg_send![super(self), setSelected: selected, animated: animated] };
            self.apply_selected(selected);
        }

        #[unsafe(method(accessibilityActivate))]
        fn accessibility_activate(&self) -> Bool {
            let handler = self.cell_ivars().on_activate.borrow().clone();
            Bool::new(guarded("uikit::table::cell.on_activate", || {
                handler.is_some_and(|handler| {
                    handler(self);
                    true
                })
            }))
        }
    }
);

impl TableCell {
    /// The cell's ivars, lazily marking `objc2`'s drop flag for reuse-pool
    /// cells: `dequeueReusableCellWithIdentifier:` creates them through
    /// `+alloc` and `initWithStyle:reuseIdentifier:` without `set_ivars`, so
    /// the flag reads `Allocated` — a state `ivars()` panics on under debug
    /// assertions. The zero-filled storage is already a valid
    /// `TableCellIvars` (every field is `None`/empty), so the flag only
    /// needs marking.
    fn cell_ivars(&self) -> &TableCellIvars {
        lazy_ivars(self)
    }

    /// Creates a reuse-pool cell.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableCellIvars::default());
        // SAFETY: `initWithStyle:reuseIdentifier:` is the reuse-pool
        // designated initializer.
        let this: Retained<Self> = unsafe {
            msg_send![super(this), initWithStyle: UITableViewCellStyle::Default, reuseIdentifier: Option::<&NSString>::None]
        };
        this.setSelectionStyle(UITableViewCellSelectionStyle::None);
        this.setBackgroundColor(Some(&UIColor::secondarySystemGroupedBackgroundColor()));
        this
    }

    /// Mounts `content` inside the cell's `contentView`, pinned `insets`
    /// inward. `shows_disclosure` toggles the disclosure-indicator accessory.
    pub fn configure(&self, content: &UIView, insets: EdgeInsets, shows_disclosure: bool) {
        let ivars = self.cell_ivars();
        if let Some(old) = ivars.content.replace(Some(content.retain())) {
            old.removeFromSuperview();
        }
        NSLayoutConstraint::deactivateConstraints(
            &NSArray::from_retained_slice(&ivars.constraints.take()),
            self.mtm(),
        );

        let content_view = self.contentView();
        content_view.addSubview(content);
        content.setTranslatesAutoresizingMaskIntoConstraints(false);
        let constraints = vec![
            content
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&content_view.leadingAnchor(), insets.left),
            content
                .topAnchor()
                .constraintEqualToAnchor_constant(&content_view.topAnchor(), insets.top),
            content
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&content_view.trailingAnchor(), -insets.right),
            content
                .bottomAnchor()
                .constraintEqualToAnchor_constant(&content_view.bottomAnchor(), -insets.bottom),
        ];
        NSLayoutConstraint::activateConstraints(
            &NSArray::from_retained_slice(&constraints),
            self.mtm(),
        );
        ivars.constraints.replace(constraints);
        self.setAccessoryType(if shows_disclosure {
            UITableViewCellAccessoryType::DisclosureIndicator
        } else {
            UITableViewCellAccessoryType::None
        });
        self.setNeedsLayout();
    }

    /// The mounted content view.
    pub fn content(&self) -> Option<Retained<UIView>> {
        self.cell_ivars().content.borrow().clone()
    }

    /// Stores `value` in the cell; the previous payload is dropped — and
    /// `prepareForReuse` drops it — so watchers the consumer stores here
    /// release with the reuse.
    pub fn set_payload(&self, value: Box<dyn Any>) {
        *self.cell_ivars().payload.borrow_mut() = Some(value);
    }

    /// Installs the hook `layoutSubviews` fires after the system layout —
    /// where placement proposals and separator insets get resolved.
    pub fn set_layout_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.cell_ivars().on_layout.replace(Some(Rc::new(handler)));
    }

    /// Installs the hook `accessibilityActivate` fires; a cell with a hook
    /// reports it handled the activation.
    pub fn set_activate_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.cell_ivars()
            .on_activate
            .replace(Some(Rc::new(handler)));
    }

    /// The separator's inset.
    pub fn set_separator_inset(&self, inset: EdgeInsets) {
        self.setSeparatorInset(UIEdgeInsets {
            top: inset.top,
            left: inset.left,
            bottom: inset.bottom,
            right: inset.right,
        });
    }

    /// Flags that layout changed mid-pass (a placement proposal arrived).
    pub fn layout_subtree_if_needed(&self) {
        self.layoutIfNeeded();
    }

    fn apply_selected(&self, selected: bool) {
        let background = if selected {
            UIColor::systemGray4Color()
        } else {
            UIColor::secondarySystemGroupedBackgroundColor()
        };
        self.setBackgroundColor(Some(&background));
        // SAFETY: the extern statics hold Apple's fixed trait bitmasks.
        let selected_trait = unsafe { UIAccessibilityTraitSelected };
        let mut traits = self.accessibilityTraits(self.mtm());
        if selected {
            traits |= selected_trait;
        } else {
            traits &= !selected_trait;
        }
        self.setAccessibilityTraits(traits, self.mtm());
    }
}

/// The per-instance state [`TableHeaderFooterView`] stores.
#[derive(Default)]
pub struct TableHeaderFooterViewIvars {
    payload: RefCell<Option<Box<dyn Any>>>,
}

impl fmt::Debug for TableHeaderFooterViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableHeaderFooterViewIvars")
            .field("payload", &self.payload.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `UITableViewHeaderFooterView`'s reuse initializer is
    // `initWithReuseIdentifier:`, which `TableHeaderFooterView::new` calls,
    // and the class does not implement `Drop`.
    #[unsafe(super(UITableViewHeaderFooterView))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUiTableHeaderFooterView"]
    #[ivars = TableHeaderFooterViewIvars]
    /// A header/footer whose text is applied through a
    /// `UIListContentConfiguration`.
    pub struct TableHeaderFooterView;

    unsafe impl NSObjectProtocol for TableHeaderFooterView {}

    impl TableHeaderFooterView {
        #[unsafe(method(prepareForReuse))]
        fn prepare_for_reuse(&self) {
            // SAFETY: the super implementation restores default state.
            let _: () = unsafe { msg_send![super(self), prepareForReuse] };
            self.view_ivars().payload.take();
        }
    }
);

impl TableHeaderFooterView {
    /// The view's ivars, lazily marking `objc2`'s drop flag for reuse-pool
    /// views: `dequeueReusableHeaderFooterViewWithIdentifier:` creates them
    /// through `+alloc` and `initWithReuseIdentifier:` without `set_ivars`.
    /// The zero-filled storage is already a valid
    /// `TableHeaderFooterViewIvars`, so the flag only needs marking.
    fn view_ivars(&self) -> &TableHeaderFooterViewIvars {
        lazy_ivars(self)
    }

    /// Creates a reuse-pool view.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableHeaderFooterViewIvars::default());
        // SAFETY: `initWithReuseIdentifier:` is the reuse-pool designated
        // initializer.
        unsafe { msg_send![super(this), initWithReuseIdentifier: Option::<&NSString>::None] }
    }

    /// Applies `text` as the header- or footer-styled content
    /// configuration.
    pub fn set_text(&self, text: &NSString, kind: SectionKind) {
        let mtm = self.mtm();
        let configuration = match kind {
            SectionKind::Header => UIListContentConfiguration::headerConfiguration(mtm),
            SectionKind::Footer => UIListContentConfiguration::footerConfiguration(mtm),
        };
        configuration.setText(Some(text));
        self.setContentConfiguration(Some(ProtocolObject::from_ref(&*configuration)));
    }

    /// Stores `value` in the view; the previous payload is dropped — and
    /// `prepareForReuse` drops the current one, so watchers the consumer
    /// stores here are released when the cell is recycled.
    pub fn set_payload(&self, value: Box<dyn Any>) {
        *self.view_ivars().payload.borrow_mut() = Some(value);
    }
}

/// The ivars of a `define_class` type the `UIKit` reuse pool can create:
/// `+alloc` zero-fills the storage — already a valid `Default`-shaped ivar
/// state for `Option`/`Vec`/`Cell` fields — while `objc2`'s `drop_flag`
/// still reads `Allocated`, which `ivars()` panics on under debug
/// assertions. This marks it `InitializedIvars` once, then reads the
/// storage at the `ivars` ivar's offset.
fn lazy_ivars<T>(this: &T) -> &T::Ivars
where
    T: ClassType + DefinedClass,
{
    let cls = T::class();
    let ivars_offset = cls
        .instance_variable(c"ivars")
        .expect("objc2 stores DefinedClass ivars under the `ivars` ivar")
        .offset();
    if let Some(flag) = cls.instance_variable(c"drop_flag") {
        // SAFETY: `drop_flag` is objc2's one-byte ivar-state marker at a
        // valid in-object offset; `0x00` is `Allocated` and `0x0f` is
        // `InitializedIvars`. The zero-filled storage is already a valid
        // `T::Ivars`, so marking it records what is already true.
        unsafe {
            let slot = std::ptr::from_ref::<T>(this)
                .cast::<u8>()
                .offset(flag.offset())
                .cast_mut();
            if *slot == 0x00 {
                *slot = 0x0f;
            }
        }
    }
    // SAFETY: the `ivars` ivar is the storage `#[ivars = T::Ivars]`
    // registers, so the offset addresses a correctly aligned `T::Ivars`;
    // freshly allocated objects hold it zero-filled — a valid ivar state
    // for `Option`/`Vec`/`Cell` fields — and `set_ivars` objects hold it
    // initialized.
    unsafe {
        &*std::ptr::from_ref::<T>(this)
            .cast::<u8>()
            .offset(ivars_offset)
            .cast::<T::Ivars>()
    }
}
