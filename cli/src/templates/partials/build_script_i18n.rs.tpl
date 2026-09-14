    // This crate is generated outside the application crate root, so its own
    // `CARGO_MANIFEST_DIR` has no `i18n/`. Hand the application's translation
    // directory to `catalog!`/`text!` through `WATERUI_I18N_DIR` so runtime
    // `text("...")` lookups resolve against the app's catalog here too. The
    // directory is watched so adding, editing or removing a locale file
    // rebuilds this crate with the new catalog.
    let i18n_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("{{ ctx.project_root_relative_path() }}")
        .join("i18n");
    println!("cargo:rustc-env=WATERUI_I18N_DIR={}", i18n_dir.display());
    if i18n_dir.is_dir() {
        println!("cargo:rerun-if-changed={}", i18n_dir.display());
    }
