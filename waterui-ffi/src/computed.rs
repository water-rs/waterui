use crate::array::waterui_str;
use ::waterui_str::Str;

impl_computed!(
    waterui_computed_str,
    Str,
    waterui_str,
    waterui_read_computed_str,
    waterui_watch_computed_str,
    waterui_drop_computed_str
);

impl_computed!(
    waterui_computed_int,
    i32,
    i32,
    waterui_read_computed_int,
    waterui_watch_computed_int,
    waterui_drop_computed_int
);

impl_computed!(
    waterui_computed_bool,
    bool,
    bool,
    waterui_read_computed_bool,
    waterui_watch_computed_bool,
    waterui_drop_computed_bool
);

impl_computed!(
    waterui_computed_double,
    f64,
    f64,
    waterui_read_computed_double,
    waterui_watch_computed_double,
    waterui_drop_computed_double
);
