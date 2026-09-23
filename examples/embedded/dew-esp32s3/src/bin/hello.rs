//! Minimal boot-verification binary: proves std/ESP-IDF/QEMU plumbing
//! before involving the dew stack.

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("HELLO_QEMU_OK");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
