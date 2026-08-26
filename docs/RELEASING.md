# Release procedure

WaterUI releases are published by Cargo and release-plz from versions already
recorded in the repository. Cargo manifests are the source of truth: every
package that Cargo considers publishable is managed automatically, and
packages with `publish = false` are excluded.

## Before merging the release pull request

1. Verify `dev` and `main` share history and GitHub can compare them normally.
2. Fetch every submodule gitlink by exact SHA from its configured remote.
3. Run the workspace CI matrix and the declared Rust 1.95 MSRV check.
4. Package every publishable crate in Nami, WaterKit, and WaterUI.
5. Rehearse registry publication in dependency order, then install the CLI
   from that registry and run `water create` followed by `water build`.
6. Build and smoke-test both WPE runtime architectures from their packaged ZIP
   files.
7. Review [the 0.3 migration guide](MIGRATION_0.3.md) and every changelog entry.

## Release

WaterUI 0.3 crosses three repositories and one support layer. Publish in this
order; do not start a later stage until every crate from the preceding stage is
visible in the crates.io index:

1. Nami 0.11.0, Nami Core 0.3.3, and Nami Derive 0.2.4.
2. The WaterUI support packages required by WaterKit: Shaderloom 0.1.0,
   `waterui-build-support` 0.1.0, Filtrate Derive 0.1.0, and Filtrate 0.2.0.
3. The complete WaterKit 0.1.1 workspace.
4. The complete WaterUI workspace, including the WaterUI 0.3 cohort and
   `waterui-cli` 0.1.4.

Cargo publishes every selected workspace in dependency order. The split
release workflow runs `release-plz release` independently from
`release-plz release-pr`, so publishing and CLI/WPE assets are not discarded if
preparing the following release PR finds a changelog or history problem.

After each release commit reaches its repository's `main` branch, monitor the
release workflow until crates.io publication, tags, GitHub releases, CLI
archives, WPE runtime manifests, checksums, and the Homebrew formula have all
completed. If publication stops after some crate versions are immutable on
crates.io, fix forward with new versions; do not delete tags or attempt to
overwrite published versions.

## Published-channel verification

Install the released CLI from an empty Cargo cache, create a project whose
manifest resolves WaterUI 0.3, and build it without a workspace path or git
patch. Repeat the applicable run path on macOS, iOS Simulator, Android,
Linux/WPE, and Windows. An issue is released only after this public-artifact
path succeeds; a passing build from `dev` is not sufficient.
