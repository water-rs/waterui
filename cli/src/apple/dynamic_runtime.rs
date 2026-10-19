//! Shared `WaterUI` runtime linkage for dynamically loaded Apple modules.

use std::ffi::OsStr;
use std::path::Path;

use eyre::{Context as _, Result, bail};

pub const INSTALL_NAME: &str = "@rpath/libwaterui_dylib.dylib";

/// The `-l` link name `staged_name` implies — the recorded runtime file
/// name `libwaterui_dylib-<metadata>.dylib` maps to `waterui_dylib-<metadata>`.
///
/// Linking the canonical `libwaterui_dylib.dylib` would record the
/// canonical install name next to rustc's hashed dependency edge whenever
/// the module resolves a runtime symbol through it, and `retarget_module`'s
/// rename would leave a duplicate `LC_LOAD_DYLIB` that aborts in dyld
/// (water-rs/waterui#1949). Binding the hashed file records the same
/// `@rpath/libwaterui_dylib-<metadata>.dylib` entry the edge carries — one
/// reference the retarget step redirects to [`INSTALL_NAME`].
///
/// `staged_name` comes from [`crate::workflows`]' `RustDynamicLibraries` —
/// the name the artifact's own dynamic section recorded this build — so
/// stale artifacts left beside it cannot influence the flag. A name that
/// is not a hashed runtime file keeps the canonical `waterui_dylib`.
pub fn runtime_link_name(staged_name: &str) -> String {
    let stem = staged_name.strip_prefix("lib").unwrap_or(staged_name);
    let stem = stem.strip_suffix(".dylib").unwrap_or(stem);
    if stem.starts_with("waterui_dylib-") {
        stem.to_string()
    } else {
        "waterui_dylib".to_string()
    }
}

pub async fn prepare_host_runtime(host: &crate::toolchain::Host, path: &Path) -> Result<()> {
    require_runtime(path)?;
    if install_name(path).await? != INSTALL_NAME {
        host.run(
            "install_name_tool",
            [
                OsStr::new("-id"),
                OsStr::new(INSTALL_NAME),
                path.as_os_str(),
            ],
        )
        .await
        .wrap_err("Failed to assign the shared WaterUI runtime install name")?;
    }
    if install_name(path).await? != INSTALL_NAME {
        bail!(
            "Shared WaterUI runtime {} did not retain install name {}",
            path.display(),
            INSTALL_NAME
        );
    }
    Ok(())
}

/// Give a module the canonical `@rpath/<file>` install name a bundled
/// executable binds when it links the file by path. Idempotent.
pub async fn canonicalize_install_name(host: &crate::toolchain::Host, path: &Path) -> Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| eyre::eyre!("Module {} has no file name", path.display()))?;
    let wanted = format!("@rpath/{}", file_name.to_string_lossy());
    if install_name(path).await? != wanted {
        host.run(
            "install_name_tool",
            [
                OsStr::new("-id"),
                OsStr::new(wanted.as_str()),
                path.as_os_str(),
            ],
        )
        .await
        .wrap_err("Failed to assign the module install name")?;
    }
    if install_name(path).await? != wanted {
        bail!(
            "Module {} did not retain install name {}",
            path.display(),
            wanted
        );
    }
    Ok(())
}

pub async fn retarget_module(
    host: &crate::toolchain::Host,
    module_path: &Path,
    runtime_path: &Path,
) -> Result<()> {
    require_runtime(runtime_path)?;
    let current_install_name = install_name(runtime_path).await?;
    if current_install_name != INSTALL_NAME {
        host.run(
            "install_name_tool",
            [
                OsStr::new("-change"),
                current_install_name.as_ref(),
                OsStr::new(INSTALL_NAME),
                module_path.as_os_str(),
            ],
        )
        .await
        .wrap_err("Failed to redirect preview module to the host WaterUI runtime")?;
    }

    let output = host
        .run("otool", [OsStr::new("-L"), module_path.as_os_str()])
        .await
        .wrap_err("Failed to inspect preview module linkage")?;
    let references = linked_libraries(&output)
        .filter(|library| *library == INSTALL_NAME)
        .count();
    if references != 1 {
        bail!(
            "Preview module {} records {} references to the shared host runtime {} — expected exactly one",
            module_path.display(),
            references,
            INSTALL_NAME
        );
    }
    Ok(())
}

fn require_runtime(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("Shared WaterUI runtime was not built at {}", path.display());
    }
    Ok(())
}

/// The `LC_ID_DYLIB` install name the 64-bit Mach-O dylib at `path`
/// records for itself, read from its load commands alone — the image
/// itself (tens of megabytes for the shared runtime) is never read.
async fn install_name(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    smol::unblock(move || {
        recorded_install_name(&path)
            .wrap_err_with(|| format!("Failed to inspect dynamic library {}", path.display()))?
            .ok_or_else(|| eyre::eyre!("Dynamic library {} has no install name", path.display()))
    })
    .await
}

fn recorded_install_name(path: &Path) -> Result<Option<String>> {
    use object::read::macho::{LoadCommandVariant, MachHeader as _};

    let cache = object::ReadCache::new(std::fs::File::open(path)?);
    let header = object::macho::MachHeader64::<object::Endianness>::parse(&cache, 0)
        .map_err(|error| eyre::eyre!("not a 64-bit Mach-O image: {error}"))?;
    let endian = header
        .endian()
        .map_err(|error| eyre::eyre!("unreadable Mach-O header: {error}"))?;
    let mut commands = header
        .load_commands(endian, &cache, 0)
        .map_err(|error| eyre::eyre!("unreadable load commands: {error}"))?;
    while let Some(command) = commands
        .next()
        .map_err(|error| eyre::eyre!("unreadable load command: {error}"))?
    {
        if let LoadCommandVariant::IdDylib(dylib) = command
            .variant()
            .map_err(|error| eyre::eyre!("unreadable load command: {error}"))?
        {
            let name = command
                .string(endian, dylib.dylib.name)
                .map_err(|error| eyre::eyre!("unreadable install name: {error}"))?;
            return Ok(Some(String::from_utf8_lossy(name).into_owned()));
        }
    }
    Ok(None)
}

fn linked_libraries(output: &str) -> impl Iterator<Item = &str> {
    output.lines().skip(1).filter_map(|line| {
        let library = line.split_whitespace().next()?;
        (!library.is_empty()).then_some(library)
    })
}

#[cfg(test)]
mod tests {
    use super::{INSTALL_NAME, linked_libraries, runtime_link_name};

    #[test]
    fn parses_macho_linked_libraries() {
        let output = concat!(
            "/tmp/libpreview.dylib:\n",
            "\t@rpath/libwaterui_dylib.dylib (compatibility version 0.0.0, current version 0.0.0)\n",
            "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)\n",
        );

        assert_eq!(linked_libraries(output).next(), Some(INSTALL_NAME));
    }

    #[test]
    fn link_name_follows_the_hashed_artifact_name() {
        assert_eq!(
            runtime_link_name("libwaterui_dylib-74e47922123ae009.dylib"),
            "waterui_dylib-74e47922123ae009"
        );
    }

    #[test]
    fn link_name_stays_canonical_for_the_unhashed_artifact() {
        assert_eq!(runtime_link_name("libwaterui_dylib.dylib"), "waterui_dylib");
    }

    #[test]
    fn link_name_stays_canonical_for_unrelated_artifacts() {
        assert_eq!(
            runtime_link_name("libstd-3f21d70fba8f3088.dylib"),
            "waterui_dylib"
        );
        assert_eq!(runtime_link_name("libc++.1.dylib"), "waterui_dylib");
    }
}
