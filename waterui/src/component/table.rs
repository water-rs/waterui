use alloc::vec::Vec;

use crate::component::{
    views::{AnyViews, Views},
    Text,
};
use waterui_reactive::{compute::IntoComputed, Computed};
#[derive(Debug)]
pub struct TableConfig {
    pub columns: Computed<Vec<TableColumn>>,
}

configurable!(Table, TableConfig);

impl Table {
    pub fn new(columns: impl IntoComputed<Vec<TableColumn>>) -> Self {
        Self(TableConfig {
            columns: columns.into_computed(),
        })
    }
}

impl_compute_result!(TableColumn);

#[derive(Clone)]
pub struct TableColumn {
    pub rows: AnyViews<Text>,
}

impl_compute_result!(AnyViews<Text>);
impl_debug!(TableColumn);

impl TableColumn {
    pub fn new(contents: impl Views<Item = Text> + 'static) -> Self {
        Self {
            rows: AnyViews::new(contents),
        }
    }
}
