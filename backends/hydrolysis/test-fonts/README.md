# Test fonts

Hydrolysis's rendering tests and the styled `waterui-testing` suites shape
against a fixed set of faces, so a layout assertion tuned on one platform's
system fonts holds on every other, and snapshot goldens are identical across
macOS, Linux, and Windows runners. No font bytes are compiled into any crate
and none are committed: `install.py` builds them from pinned, hash-verified
sources and installs them into the current user's font location, where the
test host's system font discovery resolves them by family name like any other
installed font.

Install once per machine; re-running is idempotent:

    uv run backends/hydrolysis/test-fonts/install.py

Install locations:

- macOS: `~/Library/Fonts`
- Linux: `~/.local/share/fonts`, followed by `fc-cache -f`
- Windows: `%LOCALAPPDATA%\Microsoft\Windows\Fonts`, registered per-user
  under `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Fonts`

The built files also stay in this directory (gitignored — the repository
carries no binary assets). A test that names a family the machine does not
carry fails with an error naming the family and this script — run it, then
re-run the test.

## The faces

Roboto, from the release the `water` CLI's font registry ships to applications
(<https://github.com/googlefonts/roboto/releases/download/v2.138/roboto-android.zip>),
licensed under the Apache License 2.0 (see `LICENSE` beside the files).

`NotoSans*-Regular` faces, `pyftsubset` subsets of the Noto Sans families the
CLI ships to applications (`fonts-noto-cjk` and `fonts-noto-core` upstreams,
<https://github.com/googlefonts/noto-cjk> and
<https://github.com/googlefonts/noto-fonts>), kept to the sample strings the
suite shapes and licensed under the SIL Open Font License 1.1 (see
`LICENSE-OFL.txt` beside the files). The Noto subsets are the fallback half of
the same determinism: a script Roboto cannot cover resolves to a known face on
every host — a clean CI image carries no CJK, Hangul, Thai, or Devanagari face
at all.

Production rendering is untouched: applications keep the resource fonts the
CLI stages next to the executable, and the system fallback chain behind them.

`TestVariable-ABC.ttf` (renamed from harfbuzz's `Roboto-Variable.ABC.ttf`,
Apache License 2.0) is a variable face — `wght` 100–900 and `wdth` 75–100 —
reachable in tests by family name `Test Variable ABC`; the rename keeps its
family out of the `roboto` bucket so generic-family classification never
claims it. `BungeeColor-Regular.ttf` (family `Bungee Color`, SIL Open
Font License 1.1) is a COLRv0 colour face covering the layered-colour-glyph
case. `PacificoSubset.ttf`, `NotoColorEmojiSubset.ttf` and
`DejaVuSansEmojiCoverage.ttf` cover the overhanging-ink, colour-emoji and
monochrome-emoji cases the font-fixture tests in
`src/renderer/tests/frame_work.rs` shape by family name.
