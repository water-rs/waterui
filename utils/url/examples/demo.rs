//! Demo of `WaterUI` URL functionality

use waterui_url::Url;

fn main() {
    println!("🌊 WaterUI URL Demo\n");

    // Web URLs
    println!("📡 Web URLs:");
    let web_url = Url::parse("https://example.com/api/v1/users?id=123").unwrap();
    println!("  URL: {}", web_url);
    println!("  Is web: {}", web_url.is_web());
    println!("  Scheme: {:?}", web_url.scheme());
    println!("  Host: {:?}", web_url.host());
    println!("  Path: {}", web_url.path());
    println!();

    // Local paths
    println!("📁 Local Paths:");
    let local_path = Url::new("/home/user/documents/file.pdf");
    println!("  URL: {}", local_path);
    println!("  Is local: {}", local_path.is_local());
    println!("  Is absolute: {}", local_path.is_absolute());
    println!("  Extension: {:?}", local_path.extension());
    println!("  Filename: {:?}", local_path.filename());
    println!();

    // Relative paths
    println!("🔗 Relative Paths:");
    let relative = Url::new("./images/photo.jpg");
    println!("  URL: {}", relative);
    println!("  Is relative: {}", relative.is_relative());
    println!("  Extension: {:?}", relative.extension());
    println!();

    // Data URLs
    println!("💾 Data URLs:");
    let data_url = Url::from_data("text/plain", b"Hello, WaterUI!");
    println!("  URL: {}", data_url);
    println!("  Is data: {}", data_url.is_data());
    println!("  Scheme: {:?}", data_url.scheme());
    println!();

    // URL joining
    println!("🔗 URL Joining:");
    let base = Url::new("https://cdn.example.com/assets/");
    let joined = base.join("images/logo.png");
    println!("  Base: {}", base);
    println!("  Joined: {}", joined);
    println!("  Final URL: {}", joined.as_str());
    println!();

    // Windows paths
    println!("🪟 Windows Paths:");
    let windows = Url::new("C:\\Users\\John\\Documents\\file.docx");
    println!("  URL: {}", windows);
    println!("  Is absolute: {}", windows.is_absolute());
    println!("  Extension: {:?}", windows.extension());
    println!();

    println!("✅ All URL operations working correctly!");
}
