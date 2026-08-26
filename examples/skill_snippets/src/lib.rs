//! Compile gate for the `waterui` user skill.
//!
//! Every fenced ```rust block in `.claude/skills/waterui/SKILL.md` and
//! `.claude/skills/waterui/references/*.md` is transcribed here, one module per
//! skill file, in file order. See `README.md` next to this crate's manifest for
//! the transcription rules, the conventions the `§` comments follow, and how to
//! run the gate.
//!
//! ```bash
//! cargo check -p skill_snippets --all-targets
//! cargo check -p skill_snippets --all-targets --features compile-gate-tests
//! ```
//!
//! The `compile-gate-tests` feature exposes the `#[waterui::test]` and
//! `#[waterui::bench]` transcriptions to the compiler. They must never be
//! executed: they address elements that do not exist, by design.

pub mod ref_components;
pub mod ref_i18n;
pub mod ref_interaction;
pub mod ref_media;
pub mod ref_navigation;
pub mod ref_project;
pub mod ref_reactivity;
pub mod ref_styling;
pub mod ref_testing;
pub mod ref_troubleshooting;
pub mod skill_md;
