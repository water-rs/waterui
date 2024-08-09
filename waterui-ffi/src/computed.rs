use crate::{array::waterui_str, IntoFFI};
use ::waterui_str::Str;
use waterui_reactive::Computed;

ffi_type!(
    waterui_computed_str,
    Computed<Str>,
    waterui_drop_computed_str
);
ffi_type!(
    waterui_computed_int,
    Computed<i32>,
    waterui_drop_computed_int
);

ffi_type!(
    waterui_computed_double,
    Computed<f64>,
    waterui_drop_computed_double
);

ffi_type!(
    waterui_computed_bool,
    Computed<bool>,
    waterui_drop_computed_bool
);

impl_computed!(
    waterui_computed_str,
    waterui_str,
    waterui_read_computed_str,
    waterui_watch_computed_str
);

impl_computed!(
    waterui_computed_int,
    i32,
    waterui_read_computed_int,
    waterui_watch_computed_int
);

impl_computed!(
    waterui_computed_bool,
    bool,
    waterui_read_computed_bool,
    waterui_watch_computed_bool
);

impl_computed!(
    waterui_computed_double,
    f64,
    waterui_read_computed_double,
    waterui_watch_computed_double
);
