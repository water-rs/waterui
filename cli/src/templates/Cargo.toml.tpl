[package]
name = "__CRATE_NAME__"
version = "0.1.0"
edition = "2024"
authors = ["__AUTHOR__"]

[lib]
crate-type = ["lib", "staticlib", "cdylib"]

[dependencies]
__WATERUI_DEPS__

[workspace]