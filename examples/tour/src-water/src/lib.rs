use waterui::{
    self,
    component::{stack::vstack, text, Stack, Text},
    ffi::WaterUIWidget,
    view,
    widget::Widget,
    View,
};

#[no_mangle]
pub extern "C" fn waterui_main() -> WaterUIWidget {
    let view = Home;

    let widget = Widget::from_view(view);
    widget.into()
}

#[view]
struct Home;

#[view]
impl View for Home {
    fn view(&self) -> impl View {
        vstack((text("Lexo:"), text("WaterUI")))
    }
}

#[test]
fn test() {
    let widget = Widget::from_view(Home);
    println!("{:?}", widget);
}
