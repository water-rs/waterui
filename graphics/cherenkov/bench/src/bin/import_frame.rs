//! `cherenkov-import-frame` — turns a captured `cherenkov` display list
//! into a corpus scene (#211).
//!
//! The capture root holds:
//! - `manifest.json`: `{width, height, clear, list, fonts, images}` where
//!   `list` is the `DisplayList` file's path relative to the root,
//!   `fonts`/`images` entries carry the engine `FontId`/`ImageId` raw
//!   value the list names, the blob's path relative to the root, and —
//!   for fonts — the collection index. An image entry's `encoding` field
//!   is a `cherenkov_scene::ImageEncoding`.
//! - the blob files the entries name.
//!
//! The output is a `scene.json` plus `resources/` — the blobs under
//! their content hashes, exactly as the corpus generator writes them.
//! This is a local debugging tool: its output is never committed —
//! captured frames can be megabytes of scene data, and committed scenes
//! are generated (`generate-corpus`, #211/#215).

use std::path::PathBuf;
use std::sync::Arc;

use cherenkov_bench::capture::{Capture, CapturedFont, CapturedImage, convert};
use cherenkov_scene::{Color, ImageEncoding, Scene};
use clap::Parser;
use serde::Deserialize;

/// A manifest font entry: `file`'s bytes are the `id` font's data.
#[derive(Deserialize)]
struct FontEntry {
    id: u64,
    file: String,
    #[serde(default)]
    index: u32,
}

/// A manifest image entry: `file`'s bytes are the `id` image's encoding.
#[derive(Deserialize)]
struct ImageEntry {
    id: u64,
    file: String,
    #[serde(default)]
    encoding: ImageEncoding,
}

#[derive(Deserialize)]
struct Manifest {
    width: u32,
    height: u32,
    #[serde(default = "transparent")]
    clear: Color,
    list: String,
    #[serde(default)]
    fonts: Vec<FontEntry>,
    #[serde(default)]
    images: Vec<ImageEntry>,
}

const fn transparent() -> Color {
    Color::new(cherenkov_scene::ColorSpace::Srgb, [0.0, 0.0, 0.0, 0.0])
}

#[derive(Parser)]
#[command(
    name = "cherenkov-import-frame",
    about = "Import a captured display list as a corpus scene"
)]
struct Args {
    /// Directory holding `manifest.json`, the display list and the blobs.
    #[arg(long)]
    root: PathBuf,
    /// Scene directory to write `scene.json` + `resources/` into.
    #[arg(long)]
    out: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(args.root.join("manifest.json"))?)?;
    let list: cherenkov::DisplayList =
        serde_json::from_str(&std::fs::read_to_string(args.root.join(&manifest.list))?)?;
    let fonts = manifest
        .fonts
        .iter()
        .map(|f| {
            Ok(CapturedFont {
                id: f.id,
                data: Arc::<[u8]>::from(std::fs::read(args.root.join(&f.file))?),
                index: f.index,
            })
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    let images = manifest
        .images
        .iter()
        .map(|i| {
            Ok(CapturedImage {
                id: i.id,
                data: Arc::<[u8]>::from(std::fs::read(args.root.join(&i.file))?),
                encoding: i.encoding,
            })
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    let capture = Capture {
        width: manifest.width,
        height: manifest.height,
        clear: manifest.clear,
        list,
        fonts,
        images,
    };
    let (scene, resources) = convert(&capture)?;
    scene.save(&args.out)?;
    for resource in &resources {
        let hash = Scene::store_resource(&args.out, &resource.data)?;
        assert_eq!(hash, resource.hash, "scene names resources by content hash");
    }
    println!(
        "{} commands, {} fonts, {} images -> {}",
        capture.list.commands().len(),
        capture.fonts.len(),
        capture.images.len(),
        args.out.display()
    );
    Ok(())
}
