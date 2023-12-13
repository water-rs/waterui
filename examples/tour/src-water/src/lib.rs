use std::ops::Deref;

use chrono::Local;
use waterui::{
    ffi::utils::{EventObject, ViewObject},
    view,
    widget::{button, text, vstack, when, Action, Menu, TextField},
    widget::{DatePicker, Text},
    Binding, View, ViewExt,
};

#[no_mangle]
pub extern "C" fn waterui_main() -> ViewObject {
    let view = Home::default();
    ViewObject::from(view.boxed())
}

#[derive(Debug, Default)]
#[view]
struct Home {
    #[state]
    input: String,
}

struct State<T> {
    inner: Option<T>,
}

#[view]
impl View for Home {
    fn view(&self) -> impl View {
        vstack((
            TextField::new("field", self.input.clone()), // id:0
            text("reactive:"),                           // id:1
            text(self.input.to_string()),                // id:2
        ))
    }
}

/*
let a=1;
(
    View1(a), // id:UUID
    Text()    // id:UUID
)


*/
