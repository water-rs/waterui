// TODO: Open and render panic reports in a user-friendly way

pub struct PanicReport {
    file: String, // The file where the panic occurred, like "src/main.rs"
    line: u32,    // The line number in the file where the panic occurred
    message: String,
}

impl PanicReport {
    pub fn new(file: impl Into<String>, line: u32, message: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            line,
            message: message.into(),
        }
    }
}
