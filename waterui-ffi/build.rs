extern crate cbindgen;

use std::env;

use cbindgen::generate;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    generate(crate_dir)
        .expect("Unable to generate bindings")
        .write_to_file("waterui.h");
}
