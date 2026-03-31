#[waterui::view_builder]
async fn bad(flag: bool) -> impl waterui::View {
    if flag {
        ()
    } else {
        "bad"
    }
}

fn main() {}
