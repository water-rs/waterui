fn main() {
    let name = "world";
    let _ = waterui::text!("Hello, {name}");
    let _ = waterui::text!("{count} items", count = 3);
    let _ = waterui::text!("I have {#count} apple", count = 3);
    let _ = waterui::text!("{{literal}} braces");
}
