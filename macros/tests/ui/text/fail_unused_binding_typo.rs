fn main() {
    let files = [1, 2, 3];
    let _ = waterui::text!("{count} items", cnt = files.len());
}
