struct Sample;

#[waterui::view_builder]
impl waterui::View for Sample {
    fn body(self, _env: &waterui::Environment) -> impl waterui::View {
        ()
    }
}

fn main() {}
