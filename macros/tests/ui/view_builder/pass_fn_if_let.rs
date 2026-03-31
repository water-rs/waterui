#[waterui::view_builder]
fn choose(flag: bool) -> impl waterui::View {
    let content = if flag { Some("ready") } else { None };

    if let Some(content) = content {
        content
    } else {
        ()
    }
}

fn main() {
    let _ = choose(true);
}
