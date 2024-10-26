use std::{rc::Rc, vec::Vec};

use waterui_core::{components::Text, raw_view};
use waterui_lazy::{AnyLazyList, LazyList, LazyListExt};
use waterui_reactive::{compute::ToComputed, Computed};

#[derive(Debug, PartialEq)]
pub struct Table {
    pub columns: Computed<Vec<TableColumn>>,
}

impl Table {
    pub fn new(columns: impl ToComputed<Vec<TableColumn>>) -> Self {
        Self {
            columns: columns.to_computed(),
        }
    }
}

raw_view!(Table);

#[derive(Debug, PartialEq, Clone)]
pub struct TableColumn {
    pub rows: Rc<AnyLazyList<Text>>,
}

impl TableColumn {
    pub fn new<T: Into<Text>>(data: impl LazyList<Item = T> + 'static) -> Self {
        Self {
            rows: Rc::new(AnyLazyList::new(data.map(|data| data.into()))),
        }
    }
}
