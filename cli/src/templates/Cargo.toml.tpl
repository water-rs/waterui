[package]
name = "__CRATE_NAME__"
version = "0.1.0"
edition = "2024"
authors = ["WaterUI CLI"]
license = "MIT"

[lib]
crate-type = ["lib", "staticlib", "cdylib"]

[dependencies]
waterui = "0.1"
waterui-ffi = "0.1"
