use alloc::{vec, vec::Vec};
use core::fmt;

use nami::impl_constant;
use waterui_color::Srgb;

use super::{HighlightChunk, Highlighter};

super::languages! {
    Plaintext => ["plaintext", "none", "nolang"],
    Ada => ["ada"],
    Asm => ["asm", "assembly", "assembler"],
    Awk => ["awk"],
    Bash => ["bash", "sh", "shell"],
    Bibtex => ["bibtex", "bib"],
    Bicep => ["bicep"],
    Blueprint => ["blueprint", "blp"],
    C => ["c", "h"],
    Capnp => ["capnp"],
    Clojure => ["clojure", "clj", "cljc"],
    CSharp => ["c_sharp", "c#", "csharp", "cs"],
    Cpp => ["cpp", "c++", "hpp", "h++", "cc", "hh"],
    Css => ["css"],
    Cue => ["cue"],
    D => ["d", "dlang"],
    Dart => ["dart"],
    Diff => ["diff"],
    Dockerfile => ["dockerfile", "docker"],
    Eex => ["eex"],
    Elisp => ["elisp", "el", "emacs-lisp"],
    Elixir => ["elixir", "ex", "exs", "leex"],
    Elm => ["elm"],
    Erlang => ["erlang", "erl", "hrl", "es", "escript"],
    Forth => ["forth", "fth"],
    Fortran => ["fortran", "for"],
    Fish => ["fish"],
    Gdscript => ["gdscript", "gd"],
    Gleam => ["gleam"],
    Glsl => ["glsl"],
    Go => ["go", "golang"],
    Haskell => ["haskell", "hs"],
    Hcl => ["hcl", "terraform"],
    Heex => ["heex"],
    Html => ["html", "htm"],
    Ini => ["ini"],
    Java => ["java"],
    Javascript => ["javascript", "js"],
    Json => ["json"],
    Jsx => ["jsx"],
    Julia => ["julia", "jl"],
    Kotlin => ["kotlin", "kt", "kts"],
    Latex => ["latex", "tex"],
    Llvm => ["llvm"],
    Lua => ["lua"],
    Make => ["make", "mk", "makefile"],
    Matlab => ["matlab", "m"],
    Meson => ["meson"],
    Nix => ["nix"],
    ObjectiveC => ["objc", "objective_c"],
    Ocaml => ["ocaml", "ml"],
    OcamlInterface => ["ocaml_interface", "mli"],
    OpenScad => ["openscad", "scad"],
    Pascal => ["pascal"],
    Php => ["php"],
    ProtoBuf => ["proto", "protobuf"],
    Python => ["python", "py"],
    R => ["r"],
    Racket => ["racket", "rkt"],
    Regex => ["regex"],
    Ruby => ["ruby", "rb"],
    Rust => ["rust", "rs"],
    Scala => ["scala"],
    Scheme => ["scheme", "scm", "ss"],
    Scss => ["scss"],
    Sql => ["sql"],
    Svelte => ["svelte"],
    Swift => ["swift"],
    Toml => ["toml"],
    Typescript => ["typescript", "ts"],
    Tsx => ["tsx"],
    Vimscript => ["vim", "vimscript"],
    Wast => ["wast"],
    Wat => ["wat", "wasm"],
    X86asm => ["x86asm", "x86"],
    Wgsl => ["wgsl"],
    Yaml => ["yaml"],
    Zig => ["zig"],
}

impl_constant!(Language);

/// Lightweight highlighter used when compiling for WebAssembly.
pub struct DefaultHighlighter;

impl fmt::Debug for DefaultHighlighter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefaultHighlighter").finish()
    }
}

impl Default for DefaultHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultHighlighter {
    /// Creates a new no-op highlighter used in wasm builds.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Highlighter for DefaultHighlighter {
    fn highlight<'a>(&mut self, _language: Language, text: &'a str) -> Vec<HighlightChunk<'a>> {
        vec![HighlightChunk {
            text,
            color: Srgb::WHITE,
        }]
    }
}
