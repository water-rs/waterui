fn takes_view(_: impl waterui::View) {}

fn main() {
    let builder = |flag: bool| {
        waterui::view! {
            match flag {
                true => "visible",
                false => (),
            }
        }
    };

    takes_view(builder(true));
}
