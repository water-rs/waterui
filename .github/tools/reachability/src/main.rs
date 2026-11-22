//! `reachability`: inspects `.wreach` (`__DATA,__wreach` on Apple) markers in a
//! linked release artifact and gates against unexpected realizations or the
//! erased dispatch entry (`#4694`).
//!
//! Subcommands:
//! - `reachability list <artifact>`
//! - `reachability check <artifact> [--require <name>...] [--allow <name>...]`

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use object::read::archive::ArchiveFile;
use object::read::macho::{FatArch, MachOFatFile32, MachOFatFile64};
use object::{File, FileKind, Object, ObjectSection};
use waterui_meta::{is_reach_section, reach_records};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(output) => {
            if !output.is_empty() {
                print!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("reachability error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    let Some((subcommand, rest)) = args.split_first() else {
        return Err(usage());
    };
    match subcommand.as_str() {
        "list" => {
            if rest.len() != 1 {
                return Err(usage());
            }
            let binary = resolve_artifact_binary(Path::new(&rest[0]))?;
            let reached = read_artifact_markers(&binary)?;
            let mut out = String::new();
            for marker in reached {
                out.push_str(&marker);
                out.push('\n');
            }
            Ok(out)
        }
        "check" => {
            let Some((artifact_arg, flags)) = rest.split_first() else {
                return Err(usage());
            };
            let mut required = BTreeSet::new();
            let mut allowed = BTreeSet::new();
            let mut iter = flags.iter();
            while let Some(flag) = iter.next() {
                match flag.as_str() {
                    "--require" => {
                        let value = iter
                            .next()
                            .ok_or_else(|| "--require expects a realization name".to_string())?;
                        required.insert(value.clone());
                    }
                    "--allow" => {
                        let value = iter
                            .next()
                            .ok_or_else(|| "--allow expects a realization name".to_string())?;
                        allowed.insert(value.clone());
                    }
                    other => return Err(format!("unknown flag `{other}`\n{}", usage())),
                }
            }
            let binary = resolve_artifact_binary(Path::new(artifact_arg))?;
            let reached = read_artifact_markers(&binary)?;
            evaluate_gate(&binary, &reached, &required, &allowed)
        }
        other => Err(format!("unknown subcommand `{other}`\n{}", usage())),
    }
}

fn usage() -> String {
    "usage:\n  reachability list <artifact>\n  reachability check <artifact> [--require <name>...] [--allow <name>...]".to_string()
}

/// Resolves `path` to its executable file, descending into `<App>.app/Contents/MacOS/`
/// when given a macOS application bundle directory.
fn resolve_artifact_binary(path: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    let macos_dir = path.join("Contents/MacOS");
    if macos_dir.is_dir() {
        let mut executables = Vec::new();
        let entries = std::fs::read_dir(&macos_dir)
            .map_err(|err| format!("failed to read {}: {err}", macos_dir.display()))?;
        for entry in entries {
            let entry = entry
                .map_err(|err| format!("failed to read entry in {}: {err}", macos_dir.display()))?;
            let candidate = entry.path();
            if candidate.is_file() {
                executables.push(candidate);
            }
        }
        executables.sort();
        return match executables.len() {
            1 => Ok(executables.remove(0)),
            0 => Err(format!("no executable found in {}", macos_dir.display())),
            _ => Err(format!(
                "multiple executables in {}; pass the binary path directly: {:?}",
                macos_dir.display(),
                executables
            )),
        };
    }
    Err(format!(
        "artifact {} does not exist or is not a binary/.app bundle",
        path.display()
    ))
}

/// Reads and deduplicates all `.wreach` / `__wreach` marker records from `path`.
fn read_artifact_markers(path: &Path) -> Result<BTreeSet<String>, String> {
    let data = std::fs::read(path)
        .map_err(|err| format!("failed to read artifact {}: {err}", path.display()))?;
    parse_markers_from_bytes(&data, path)
}

fn parse_markers_from_bytes(data: &[u8], path: &Path) -> Result<BTreeSet<String>, String> {
    let mut markers = BTreeSet::new();
    let mut parsed_objects = 0usize;
    visit_objects(data, path, &mut |file| {
        parsed_objects += 1;
        for section in file.sections() {
            let Ok(name) = section.name() else { continue };
            if !is_reach_section(name) {
                continue;
            }
            let data = section.data().map_err(|err| {
                format!(
                    "failed to read section `{name}` in {}: {err}",
                    path.display()
                )
            })?;
            for raw in reach_records(data) {
                let text = std::str::from_utf8(raw).map_err(|err| {
                    format!(
                        "non-UTF-8 `.wreach` record {raw:?} in {}: {err}",
                        path.display()
                    )
                })?;
                markers.insert(text.to_owned());
            }
        }
        Ok(())
    })?;
    if parsed_objects == 0 {
        return Err(format!(
            "{} is not a recognized object, archive, or universal binary",
            path.display()
        ));
    }
    Ok(markers)
}

fn visit_objects<'a>(
    data: &'a [u8],
    path: &Path,
    visit: &mut impl FnMut(File<'a>) -> Result<(), String>,
) -> Result<(), String> {
    match FileKind::parse(data) {
        Ok(FileKind::Archive) => {
            let archive = ArchiveFile::parse(data)
                .map_err(|err| format!("failed to parse archive {}: {err}", path.display()))?;
            for member in archive.members() {
                let member = member.map_err(|err| {
                    format!("failed to read archive member in {}: {err}", path.display())
                })?;
                if member.name() == b"lib.rmeta" {
                    continue;
                }
                let member_data = member.data(data).map_err(|err| {
                    format!(
                        "failed to extract archive member in {}: {err}",
                        path.display()
                    )
                })?;
                if let Ok(file) = File::parse(member_data) {
                    visit(file)?;
                }
            }
            Ok(())
        }
        Ok(FileKind::MachOFat32) => {
            let fat = MachOFatFile32::parse(data)
                .map_err(|err| format!("failed to parse fat Mach-O {}: {err}", path.display()))?;
            for arch in fat.arches() {
                let slice = arch.data(data).map_err(|err| {
                    format!("failed to read fat slice in {}: {err}", path.display())
                })?;
                visit_objects(slice, path, visit)?;
            }
            Ok(())
        }
        Ok(FileKind::MachOFat64) => {
            let fat = MachOFatFile64::parse(data)
                .map_err(|err| format!("failed to parse fat64 Mach-O {}: {err}", path.display()))?;
            for arch in fat.arches() {
                let slice = arch.data(data).map_err(|err| {
                    format!("failed to read fat64 slice in {}: {err}", path.display())
                })?;
                visit_objects(slice, path, visit)?;
            }
            Ok(())
        }
        _ => {
            if let Ok(file) = File::parse(data) {
                visit(file)?;
            }
            Ok(())
        }
    }
}

fn evaluate_gate(
    artifact: &Path,
    reached: &BTreeSet<String>,
    required: &BTreeSet<String>,
    allowed: &BTreeSet<String>,
) -> Result<String, String> {
    let missing: Vec<&String> = required
        .iter()
        .filter(|name| !reached.contains(*name))
        .collect();
    let unexpected: Vec<&String> = reached
        .iter()
        .filter(|name| !required.contains(*name) && !allowed.contains(*name))
        .collect();

    let reached_list: Vec<&str> = reached.iter().map(String::as_str).collect();
    if !missing.is_empty() || !unexpected.is_empty() {
        let mut msg = format!("reachability check failed for {}:\n", artifact.display());
        if !missing.is_empty() {
            msg.push_str(&format!(
                "  missing required realizations (did `.wreach` get stripped?): {missing:?}\n"
            ));
        }
        if !unexpected.is_empty() {
            if unexpected.iter().any(|name| name.as_str() == "erased") {
                msg.push_str(
                    "  erased dispatch entry (`erased`) survived into the release artifact!\n",
                );
            }
            msg.push_str(&format!(
                "  unexpected realizations survived dead-code elimination: {unexpected:?}\n"
            ));
        }
        msg.push_str(&format!("  all reached markers: {reached_list:?}"));
        return Err(msg);
    }

    Ok(format!(
        "reachability ok ({}): {:?}\n",
        artifact.display(),
        reached_list
    ))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn gate_accepts_required_and_allowed_and_rejects_erased_or_unused() {
        let artifact = Path::new("app");
        let required = BTreeSet::from(["text".to_string(), "button".to_string()]);
        let allowed = BTreeSet::from(["fixed_container".to_string(), "label".to_string()]);

        let clean = BTreeSet::from([
            "button".to_string(),
            "fixed_container".to_string(),
            "text".to_string(),
        ]);
        assert!(evaluate_gate(artifact, &clean, &required, &allowed).is_ok());

        let with_erased = BTreeSet::from([
            "button".to_string(),
            "erased".to_string(),
            "text".to_string(),
        ]);
        let err = evaluate_gate(artifact, &with_erased, &required, &allowed)
            .expect_err("erased dispatch entry must fail the gate");
        assert!(
            err.contains("erased dispatch entry (`erased`) survived"),
            "{err}"
        );

        let with_unused = BTreeSet::from([
            "button".to_string(),
            "slider".to_string(),
            "text".to_string(),
        ]);
        let err = evaluate_gate(artifact, &with_unused, &required, &allowed)
            .expect_err("unused realization must fail the gate");
        assert!(err.contains("slider"), "{err}");

        let missing_button = BTreeSet::from(["text".to_string()]);
        let err = evaluate_gate(artifact, &missing_button, &required, &allowed)
            .expect_err("missing required marker must fail the gate");
        assert!(err.contains("missing required realizations"), "{err}");
    }

    #[test]
    fn linked_stripped_lto_binary_keeps_reached_markers_and_drops_dead_ones() {
        let meta_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../utils/meta");
        let tmp = std::env::temp_dir().join(format!("waterui-reach-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let meta_rlib = tmp.join("libwaterui_meta.rlib");
        let status = Command::new("rustc")
            .args([
                "--edition=2024",
                "--crate-type=rlib",
                "--crate-name=waterui_meta",
                "-O",
                "-Cembed-bitcode=yes",
                "-Ccodegen-units=1",
            ])
            .arg(meta_dir.join("src/lib.rs"))
            .arg("-o")
            .arg(&meta_rlib)
            .status()
            .expect("invoke rustc for waterui_meta");
        assert!(status.success(), "compiling waterui_meta failed");

        let clean_src = tmp.join("clean.rs");
        std::fs::write(
            &clean_src,
            r#"
#[inline(always)]
fn text() {
    waterui_meta::mark_reached!("text");
}

#[inline(always)]
fn button() {
    waterui_meta::mark_reached!("button");
}

#[inline(never)]
pub fn unused_slider() {
    waterui_meta::mark_reached!("slider");
}

#[inline(never)]
pub fn erased_entry() {
    waterui_meta::mark_reached!("erased");
}

fn main() {
    text();
    button();
}
"#,
        )
        .expect("write clean.rs");

        let clean_bin = tmp.join(if cfg!(windows) { "clean.exe" } else { "clean" });
        let mut build_clean = Command::new("rustc");
        build_clean
            .args([
                "--edition=2024",
                "-O",
                "-Clto=fat",
                "-Ccodegen-units=1",
                "-Cstrip=symbols",
            ])
            .arg("--extern")
            .arg(format!("waterui_meta={}", meta_rlib.display()))
            .arg(&clean_src)
            .arg("-o")
            .arg(&clean_bin);
        let status = build_clean.status().expect("compile clean binary");
        assert!(status.success(), "linking clean binary failed");

        let reached = read_artifact_markers(&clean_bin).expect("read markers from clean binary");
        let required = BTreeSet::from(["text".to_string(), "button".to_string()]);
        evaluate_gate(&clean_bin, &reached, &required, &BTreeSet::new())
            .expect("clean binary should keep text+button even after LTO inlining + symbol stripping, and drop slider+erased");

        // Now verify a binary that reaches `erased` and `slider` fails the gate.
        let leaky_src = tmp.join("leaky.rs");
        std::fs::write(
            &leaky_src,
            r#"
fn main() {
    waterui_meta::mark_reached!("text");
    waterui_meta::mark_reached!("button");
    waterui_meta::mark_reached!("erased");
    waterui_meta::mark_reached!("slider");
}
"#,
        )
        .expect("write leaky.rs");
        let leaky_bin = tmp.join(if cfg!(windows) { "leaky.exe" } else { "leaky" });
        let status = Command::new("rustc")
            .args([
                "--edition=2024",
                "-O",
                "-Clto=fat",
                "-Ccodegen-units=1",
                "-Cstrip=symbols",
            ])
            .arg("--extern")
            .arg(format!("waterui_meta={}", meta_rlib.display()))
            .arg(&leaky_src)
            .arg("-o")
            .arg(&leaky_bin)
            .status()
            .expect("compile leaky binary");
        assert!(status.success(), "linking leaky binary failed");

        let leaky_reached =
            read_artifact_markers(&leaky_bin).expect("read markers from leaky binary");
        let err = evaluate_gate(&leaky_bin, &leaky_reached, &required, &BTreeSet::new())
            .expect_err("leaky binary must fail the gate");
        assert!(err.contains("erased"), "{err}");
        assert!(err.contains("slider"), "{err}");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
