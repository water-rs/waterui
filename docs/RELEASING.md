# Release procedure

WaterUI releases are calculated and published by release-plz. Cargo manifests
are the source of truth: every package that Cargo considers publishable is
managed automatically, and packages with `publish = false` are excluded.

## Before merging the release pull request

1. Verify `dev` and `main` share history and GitHub can compare them normally.
2. Fetch every submodule gitlink by exact SHA from its configured remote.
3. Run the workspace CI matrix and the declared Rust 1.95 MSRV check.
4. Package every publishable crate in Nami, WaterKit, and WaterUI.
5. Rehearse registry publication in dependency order, then install the CLI
   from that registry and run `water create` followed by `water build`.
6. Build and smoke-test both WPE runtime architectures from their packaged ZIP
   files.
7. Review [the 0.3 migration guide](MIGRATION_0.3.md) and every generated
   changelog entry.

## Release

The release-plz pull request owns version changes, workspace dependency
propagation, changelogs, tags, GitHub releases, and crates.io publication. Do
not maintain a second package allowlist or publish crates manually ahead of the
release pull request.

After the release-plz pull request is merged, monitor the release workflow
until crates.io publication, CLI archives, WPE runtime manifests, checksums,
and the Homebrew formula have all completed. If publication stops after some
crate versions are immutable on crates.io, fix forward with new versions; do
not delete tags or attempt to overwrite published versions.

## Published-channel verification

Install the released CLI from an empty Cargo cache, create a project whose
manifest resolves WaterUI 0.3, and build it without a workspace path or git
patch. Repeat the applicable run path on macOS, iOS Simulator, Android,
Linux/WPE, and Windows. An issue is released only after this public-artifact
path succeeds; a passing build from `dev` is not sufficient.
