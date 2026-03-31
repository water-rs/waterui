fn takes_view(_: impl waterui::View) {}

fn main() {
    takes_view(waterui::view! {
        "discarded";
        if true {
            "shown"
        } else {
            ()
        }
    });

    takes_view(waterui::view! {
        if true {
            "ignored";
        }
    });
}
