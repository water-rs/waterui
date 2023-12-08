use waterui::{
    self,
    component::{button, stack::vstack, text},
    ffi::utils::{EventObject, ViewObject},
    view,
    widget::Text,
    View, ViewExt,
};

#[no_mangle]
pub extern "C" fn waterui_main() -> ViewObject {
    let view = Home::default();
    ViewObject::from(Home::default().boxed())
}

#[derive(Debug, Default)]
#[view]
struct Home {
    #[state]
    num: u64,
}

#[view]
impl View for Home {
    fn view(&self) -> impl View {
        let num = self.num.clone();
        vstack((
            text("Counter"),
            Text::display(self.num.get()),
            button("Increase").action(move || {
                *num.get_mut() += 1;
            }),
        ))
    }
}
