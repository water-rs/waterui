use crate::array::waterui_str;
use ::waterui_str::Str;

impl_binding!(
    waterui_binding_str,
    Str,
    waterui_str,
    waterui_read_binding_str,
    waterui_set_binding_str,
    waterui_watch_binding_str,
    waterui_drop_binding_str
);

impl_binding!(
    waterui_binding_double,
    f64,
    f64,
    waterui_read_binding_double,
    waterui_set_binding_double,
    waterui_watch_binding_double,
    waterui_drop_binding_double
);

impl_binding!(
    waterui_binding_int,
    i32,
    i32,
    waterui_read_binding_int,
    waterui_set_binding_int,
    waterui_watch_binding_int,
    waterui_drop_binding_int
);

impl_binding!(
    waterui_binding_bool,
    bool,
    bool,
    waterui_read_binding_bool,
    waterui_set_binding_bool,
    waterui_watch_binding_bool,
    waterui_drop_binding_bool
);
