#[waterui::view_builder]
fn bad(flag: bool) -> waterui::AnyView {
    if flag {
        ()
    } else {
        "bad"
    }
}

fn main() {}
