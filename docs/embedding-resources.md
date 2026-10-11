# Embedding resource ownership

Each native application environment owns a `waterui_core::ResourceContext`.
It contains the asset root and font directory supplied by the host. Cloning
an environment preserves that context; installing a different context in
the clone does not change the original instance.

Native initialization installs the context before constructing the app.
Standalone initialization captures the existing `WATERUI_ASSETS_ROOT`
override or executable-relative package layout once. An embedded host
supplies its own directories; asset resolution never mutates process
environment variables or consults a global resource context.

`ImageAsset` and `VideoAsset` remain views. Their bodies resolve their URLs
through the context in the rendering environment. Their default stretch
axes remain identical to `Photo` and fit-mode `Video`, respectively.

For operations outside view rendering, pass the context explicitly:

```rust,ignore
let resources = waterui::ResourceContext::from_environment(&env);
let data = waterui::asset!("settings.json").load(resources)?;
let image_url = waterui::asset!("logo.png").url(resources);
let video = waterui::asset!("movie.mp4").player(resources);
let site = waterui::include_web!(resources, "web");
```

Local `asset!` paths are logical paths inside the packaged asset root.
Remote URL and compile-time `embed = true` forms keep their existing
behavior. Bundle path/URL methods and file-handle load methods now require
a `&ResourceContext`; `bundle_root` borrows the root from that context.

Apple packages provide their `Bundle.module` asset and font URLs through
`WaterUIResourceContext`. The backend copies these into the Rust context
and registers package fonts before creating the app. The ordinary native
main bundle supplies the standalone Apple context.
