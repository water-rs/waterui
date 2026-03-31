struct Sample(bool);

impl waterui::View for Sample {
    #[waterui::view_builder]
    fn body(self, _env: &waterui::Environment) -> impl waterui::View {
        match self.0 {
            true => "enabled",
            false => (),
        }
    }
}

fn main() {
    let _ = Sample(true);
}
