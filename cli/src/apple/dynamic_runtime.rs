//! Shared `WaterUI` runtime linkage for dynamically loaded Apple modules.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context as _, Result, bail};

use crate::utils::run_command_os;

pub const FILE_NAME: &str = "libwaterui_dylib.dylib";
pub const INSTALL_NAME: &str = "@rpath/libwaterui_dylib.dylib";

pub fn build_path(lib_dir: &Path) -> PathBuf {
    lib_dir.join("deps").join(FILE_NAME)
}

pub async fn prepare_host_runtime(path: &Path) -> Result<()> {
    require_runtime(path)?;
    if install_name(path).await? != INSTALL_NAME {
        run_command_os(
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

pub async fn retarget_module(module_path: &Path, build_lib_dir: &Path) -> Result<()> {
    let runtime_path = build_path(build_lib_dir);
    require_runtime(&runtime_path)?;
    let current_install_name = install_name(&runtime_path).await?;
    if current_install_name != INSTALL_NAME {
        run_command_os(
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

    let output = run_command_os("otool", [OsStr::new("-L"), module_path.as_os_str()])
        .await
        .wrap_err("Failed to inspect preview module linkage")?;
    if !linked_libraries(&output).any(|library| library == INSTALL_NAME) {
        bail!(
            "Preview module {} is not linked to the shared host runtime {}",
            module_path.display(),
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

async fn install_name(path: &Path) -> Result<String> {
    let output = run_command_os("otool", [OsStr::new("-D"), path.as_os_str()])
        .await
        .wrap_err_with(|| format!("Failed to inspect dynamic library {}", path.display()))?;
    output
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            color_eyre::eyre::eyre!("Dynamic library {} has no install name", path.display())
        })
}

fn linked_libraries(output: &str) -> impl Iterator<Item = &str> {
    output.lines().skip(1).filter_map(|line| {
        let library = line.split_whitespace().next()?;
        (!library.is_empty()).then_some(library)
    })
}

#[cfg(test)]
mod tests {
    use super::{INSTALL_NAME, linked_libraries};

    #[test]
    fn parses_macho_linked_libraries() {
        let output = concat!(
            "/tmp/libpreview.dylib:\n",
            "\t@rpath/libwaterui_dylib.dylib (compatibility version 0.0.0, current version 0.0.0)\n",
            "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)\n",
        );

        assert_eq!(linked_libraries(output).next(), Some(INSTALL_NAME));
    }
}
