//! Select macOS from the SDK's zippered macOS/Mac Catalyst runtime objects.
//!
//! Apple's ld(1) documents `-r`, `-platform_version`, and `-keep_private_externs`.
//! Relinking each member selects its destination without editing load commands or
//! combining independently selectable archive members.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use eyre::{Context, Result};
use futures_util::{StreamExt, TryStreamExt};
use object::read::archive::ArchiveFile;
use smol::fs;

use crate::toolchain::Host;

pub(super) async fn macos_objects(
    host: &Host,
    runtimes: &[PathBuf],
    directory: &Path,
    deployment: &str,
) -> Result<Vec<PathBuf>> {
    let sdk = host
        .run("xcrun", ["--sdk", "macosx", "--show-sdk-version"])
        .await?;
    let mut objects = Vec::new();
    for (index, runtime) in runtimes.iter().enumerate() {
        let directory = directory.join(index.to_string());
        fs::create_dir_all(&directory).await?;
        let archive = directory.join("input.a");
        let mut args = [
            "--sdk",
            "macosx",
            "libtool",
            "-static",
            "-arch_only",
            "arm64",
            "-o",
        ]
        .map(OsString::from)
        .to_vec();
        args.extend([
            archive.as_os_str().to_owned(),
            runtime.as_os_str().to_owned(),
        ]);
        host.run("xcrun", args).await?;
        let data = fs::read(&archive).await?;
        let parsed = ArchiveFile::parse(data.as_slice())?;
        let members = parsed
            .members()
            .map(|member| member?.data(data.as_slice()))
            .collect::<object::read::Result<Vec<_>>>()?;
        // Keep each archive member separate and in input order. Eight independent
        // partial links may run concurrently; no whole-archive link or dead strip
        // happens here. The consumer still selects individual runtime objects.
        let selected: Vec<_> = futures_util::stream::iter(members.into_iter().enumerate())
            .map(|(index, data)| {
                let input = directory.join(format!("{index}-input.o"));
                let output = directory.join(format!("{index}.o"));
                let sdk = sdk.trim();
                async move {
                    fs::write(&input, data).await?;
                    let mut args = [
                        "--sdk",
                        "macosx",
                        "ld",
                        "-r",
                        "-arch",
                        "arm64",
                        "-platform_version",
                        "macos",
                        deployment,
                        sdk,
                        "-keep_private_externs",
                        "-o",
                    ]
                    .map(OsString::from)
                    .to_vec();
                    args.extend([output.as_os_str().to_owned(), input.as_os_str().to_owned()]);
                    host.run("xcrun", args).await.wrap_err_with(|| {
                        format!(
                            "Cannot select macOS for runtime member {} in {}",
                            index,
                            runtime.display()
                        )
                    })?;
                    Ok::<_, eyre::Report>(output)
                }
            })
            .buffered(8)
            .try_collect()
            .await?;
        objects.extend(selected);
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use object::{Object, ObjectSymbol};

    // This consumes the selected SDK's real runtime: a synthetic object would
    // miss the zippered platform metadata that causes XCFramework rejection.
    #[test]
    #[ignore = "requires Xcode and runs Apple partial links; run on the macOS cloud gate"]
    fn macos_runtime_keeps_member_symbols_and_selects_one_platform() {
        smol::block_on(async {
            let host = Host::current();
            let resource = host
                .run("xcrun", ["--sdk", "macosx", "clang", "-print-resource-dir"])
                .await
                .unwrap();
            let runtime = Path::new(resource.trim()).join("lib/darwin/libclang_rt.osx.a");
            let temporary = tempfile::tempdir().unwrap();
            let outputs = macos_objects(&host, &[runtime], temporary.path(), "13.0")
                .await
                .unwrap();
            let input = fs::read(temporary.path().join("0/input.a")).await.unwrap();
            let archive = ArchiveFile::parse(input.as_slice()).unwrap();
            let members = archive
                .members()
                .collect::<object::read::Result<Vec<_>>>()
                .unwrap();
            assert_ne!(outputs, [] as [std::path::PathBuf; 0]);
            assert_eq!(outputs.len(), members.len());
            for (member, output) in members.into_iter().zip(outputs) {
                let original = object::File::parse(member.data(input.as_slice()).unwrap()).unwrap();
                let data = fs::read(&output).await.unwrap();
                let selected = object::File::parse(data.as_slice()).unwrap();
                let subsections = |file: &object::File<'_>| match file.flags() {
                    object::FileFlags::MachO { flags } => {
                        flags & object::macho::MH_SUBSECTIONS_VIA_SYMBOLS
                    }
                    other => panic!("Expected Mach-O runtime object, got {other:?}"),
                };
                assert_eq!(subsections(&original), subsections(&selected));
                let symbols = |file: &object::File<'_>| {
                    file.symbols()
                        .filter(ObjectSymbol::is_global)
                        .map(|symbol| {
                            (
                                symbol.name().unwrap().to_owned(),
                                symbol.is_undefined(),
                                symbol.is_weak(),
                                format!("{:?}", symbol.scope()),
                            )
                        })
                        .collect::<std::collections::BTreeSet<_>>()
                };
                assert_eq!(symbols(&original), symbols(&selected));
                let tags = host
                    .run(
                        "xcrun",
                        [
                            OsString::from("vtool"),
                            OsString::from("-show-build"),
                            output.into_os_string(),
                        ],
                    )
                    .await
                    .unwrap();
                assert!(tags.contains("platform MACOS"), "{tags}");
                assert_eq!(tags.matches("platform ").count(), 1, "{tags}");
            }
        });
    }
}
