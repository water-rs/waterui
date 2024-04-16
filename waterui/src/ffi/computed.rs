use core::mem::ManuallyDrop;

use super::{Data, Int, Subscriber, Utf8Data};
use alloc::{string::String, vec::Vec};
use waterui_reactive::{Compute, Computed};

impl_computed!(
    waterui_read_computed_data,
    waterui_subscribe_computed_data,
    waterui_unsubscribe_computed_data,
    waterui_drop_computed_data,
    ComputedData,
    Vec<u8>,
    Data
);

impl_computed!(
    waterui_read_computed_str,
    waterui_subscribe_computed_str,
    waterui_unsubscribe_computed_str,
    waterui_drop_computed_str,
    ComputedStr,
    String,
    Utf8Data
);

impl_computed!(
    waterui_read_computed_int,
    waterui_subscribe_computed_int,
    waterui_unsubscribe_computed_int,
    waterui_drop_computed_int,
    ComputedInt,
    Int,
    Int
);
