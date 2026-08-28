#!/usr/bin/env python3

import argparse
import datetime
import filecmp
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import zipfile


SYSTEM_LIBRARIES = {
    "ld-linux-aarch64.so.1",
    "ld-linux-x86-64.so.2",
    "libc.so.6",
    "libdl.so.2",
    "libm.so.6",
    "libpthread.so.0",
    "libresolv.so.2",
    "librt.so.1",
}

SYSTEM_GRAPHICS_LIBRARIES = {
    "libEGL.so.1",
    "libGL.so.1",
    "libGLX.so.0",
    "libGLdispatch.so.0",
    "libOpenGL.so.0",
    "libvulkan.so.1",
}

SANDBOX_TOOLS = {
    pathlib.Path("/usr/bin/bwrap"): "bubblewrap >= 0.3.1",
    pathlib.Path("/usr/bin/xdg-dbus-proxy"): "xdg-dbus-proxy",
}


def run(*arguments: str) -> str:
    return subprocess.run(
        arguments,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout


def is_elf(path: pathlib.Path) -> bool:
    if not path.is_file() or path.is_symlink():
        return False
    with path.open("rb") as source:
        return source.read(4) == b"\x7fELF"


def copy_file(source: pathlib.Path, destination: pathlib.Path) -> pathlib.Path:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        if not filecmp.cmp(source, destination, shallow=False):
            raise RuntimeError(
                f"runtime dependency collision at {destination}: {source}"
            )
        return destination
    shutil.copy2(source, destination)
    return destination


def package_owner(path: pathlib.Path) -> str:
    result = subprocess.run(
        ["dpkg-query", "-S", str(path)],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    if result.returncode != 0:
        raise RuntimeError(f"runtime dependency has no Debian package owner: {path}")
    return result.stdout.split(":", maxsplit=1)[0].split(",", maxsplit=1)[0]


def copy_plugin_packages(
    root: pathlib.Path,
) -> tuple[list[pathlib.Path], set[pathlib.Path]]:
    copied = []
    sources = set()
    for package in ("gstreamer1.0-plugins-base", "gstreamer1.0-plugins-good"):
        for name in run("dpkg-query", "-L", package).splitlines():
            source = pathlib.Path(name)
            if (
                source.is_file()
                and source.suffix == ".so"
                and "gstreamer-1.0" in source.parts
            ):
                sources.add(source.resolve())
                copied.append(
                    copy_file(source, root / "lib/gstreamer-1.0" / source.name)
                )

    multiarch = run("dpkg-architecture", "-qDEB_HOST_MULTIARCH").strip()
    scanner = pathlib.Path(
        f"/usr/lib/{multiarch}/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner"
    )
    sources.add(scanner.resolve())
    copied.append(
        copy_file(scanner, root / "libexec/gstreamer-1.0/gst-plugin-scanner")
    )
    modules = pathlib.Path(f"/usr/lib/{multiarch}/gio/modules")
    for source in sorted(modules.glob("*.so")):
        sources.add(source.resolve())
        copied.append(copy_file(source, root / "lib/gio/modules" / source.name))
    return copied, sources


def dependency_paths(path: pathlib.Path) -> list[pathlib.Path]:
    dependencies = []
    for name in run("lddtree", "-l", str(path)).splitlines():
        candidate = pathlib.Path(name)
        if candidate.is_absolute() and candidate.exists():
            dependencies.append(candidate.resolve())
    return dependencies


def soname(path: pathlib.Path) -> str | None:
    dynamic = run("readelf", "--dynamic", str(path))
    match = re.search(r"\(SONAME\).*\[(.+)]", dynamic)
    return match.group(1) if match else None


def copy_dependency_closure(
    root: pathlib.Path, seeds: list[pathlib.Path]
) -> tuple[set[pathlib.Path], set[str]]:
    copied_sources = set()
    system_requirements = set()
    queue = list(seeds)
    queued = {path.resolve() for path in queue}
    while queue:
        binary = queue.pop()
        for dependency in dependency_paths(binary):
            if root in dependency.parents:
                continue
            name = dependency.name
            if name in SYSTEM_LIBRARIES or name in SYSTEM_GRAPHICS_LIBRARIES:
                system_requirements.add(name)
                continue
            destination = copy_file(dependency, root / "lib" / name)
            copied_sources.add(dependency)
            library_soname = soname(dependency)
            if library_soname and library_soname != name:
                link = root / "lib" / library_soname
                if link.exists() and link.resolve() != destination.resolve():
                    raise RuntimeError(f"conflicting runtime SONAME {library_soname}")
                if not link.exists():
                    link.symlink_to(name)
            resolved_destination = destination.resolve()
            if resolved_destination not in queued:
                queue.append(destination)
                queued.add(resolved_destination)
    return copied_sources, system_requirements


def set_relative_rpaths(root: pathlib.Path) -> None:
    library_root = root / "lib"
    for path in sorted(root.rglob("*")):
        if not is_elf(path):
            continue
        relative_library_root = os.path.relpath(library_root, path.parent)
        rpath = "$ORIGIN"
        if relative_library_root != ".":
            rpath = f"$ORIGIN/{relative_library_root}"
        run("patchelf", "--set-rpath", rpath, str(path))


def glibc_versions(path: pathlib.Path) -> list[tuple[int, ...]]:
    versions = []
    output = run("readelf", "--version-info", str(path))
    for match in re.findall(r"GLIBC_(\d+(?:\.\d+)*)", output):
        versions.append(tuple(int(part) for part in match.split(".")))
    return versions


def validate_glibc(root: pathlib.Path, maximum: str) -> None:
    maximum_version = tuple(int(part) for part in maximum.split("."))
    violations = []
    for path in sorted(root.rglob("*")):
        if not is_elf(path):
            continue
        required = glibc_versions(path)
        if required and max(required) > maximum_version:
            violations.append(f"{path.relative_to(root)} requires GLIBC_{'.'.join(map(str, max(required)))}")
    if violations:
        raise RuntimeError("\n".join(violations))


def copy_licenses(
    root: pathlib.Path,
    source: pathlib.Path,
    repository: pathlib.Path,
    packaged_sources: set[pathlib.Path],
) -> list[dict[str, object]]:
    license_root = root / "licenses"
    waterui_root = license_root / "waterui"
    waterui_root.mkdir(parents=True)
    shutil.copy2(repository / "LICENSE-APACHE", waterui_root / "LICENSE-APACHE")
    shutil.copy2(repository / "LICENSE-MIT", waterui_root / "LICENSE-MIT")

    webkit_root = license_root / "wpe-webkit"
    for license_path in sorted(source.rglob("*")):
        if not license_path.is_file():
            continue
        upper_name = license_path.name.upper()
        if not (upper_name.startswith("LICENSE") or upper_name.startswith("COPYING")):
            continue
        relative = license_path.relative_to(source)
        destination = webkit_root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(license_path, destination)

    packages = []
    for package in sorted({package_owner(path) for path in packaged_sources}):
        version = run("dpkg-query", "-W", "-f=${Version}", package).strip()
        copyright_file = pathlib.Path(f"/usr/share/doc/{package}/copyright")
        if not copyright_file.is_file():
            raise RuntimeError(f"runtime package {package} has no copyright file")
        destination = license_root / "system" / f"{package}.txt"
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(copyright_file, destination)
        packages.append(
            {
                "name": package,
                "SPDXID": f"SPDXRef-Package-{re.sub(r'[^A-Za-z0-9.-]', '-', package)}",
                "versionInfo": version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": False,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": "NOASSERTION",
                "copyrightText": "NOASSERTION",
            }
        )
    return packages


def write_metadata(
    root: pathlib.Path,
    version: str,
    architecture: str,
    released: str,
    system_requirements: set[str],
    system_packages: list[dict[str, object]],
) -> None:
    identity = {
        "schema_version": 1,
        "engine": "wpe",
        "version": version,
        "platform": "linux",
        "architecture": architecture,
        "system_requirements": sorted(
            system_requirements
            | set(SANDBOX_TOOLS.values())
            | {"glibc", "Linux GPU driver"}
        ),
    }
    (root / "runtime.json").write_text(
        json.dumps(identity, indent=2, sort_keys=True) + "\n"
    )

    packages = [
        {
            "name": "WPE WebKit",
            "SPDXID": "SPDXRef-Package-WPEWebKit",
            "versionInfo": version,
            "downloadLocation": f"https://wpewebkit.org/releases/wpewebkit-{version}.tar.xz",
            "filesAnalyzed": False,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "LGPL-2.1-or-later AND BSD-2-Clause",
            "copyrightText": "NOASSERTION",
        },
        *system_packages,
    ]
    sbom = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"WaterUI WPE runtime {version} linux {architecture}",
        "documentNamespace": f"https://waterui.dev/sbom/wpe/{version}/linux/{architecture}",
        "creationInfo": {
            "created": released,
            "creators": ["Tool: WaterUI WPE runtime packager"],
        },
        "packages": packages,
    }
    (root / "sbom.spdx.json").write_text(
        json.dumps(sbom, indent=2, sort_keys=True) + "\n"
    )


def write_zip(root: pathlib.Path, archive: pathlib.Path, released: str) -> None:
    timestamp = datetime.datetime.fromisoformat(released.replace("Z", "+00:00"))
    zip_timestamp = (
        timestamp.year,
        timestamp.month,
        timestamp.day,
        timestamp.hour,
        timestamp.minute,
        timestamp.second,
    )
    with zipfile.ZipFile(
        archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as output:
        for path in sorted(root.rglob("*")):
            relative = path.relative_to(root).as_posix()
            if path.is_dir():
                continue
            info = zipfile.ZipInfo(relative, zip_timestamp)
            mode = path.lstat().st_mode
            info.external_attr = mode << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            if path.is_symlink():
                output.writestr(info, os.readlink(path).encode())
            else:
                with path.open("rb") as source, output.open(info, "w") as destination:
                    shutil.copyfileobj(source, destination, length=1024 * 1024)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--architecture", required=True)
    parser.add_argument("--maximum-glibc", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--prefix", required=True, type=pathlib.Path)
    parser.add_argument("--released", required=True)
    parser.add_argument("--repository", required=True, type=pathlib.Path)
    parser.add_argument("--source", required=True, type=pathlib.Path)
    parser.add_argument("--version", required=True)
    arguments = parser.parse_args()

    root = arguments.prefix.resolve()
    bridge = root / "lib/libwaterui_wpe.so"
    if not bridge.is_file():
        raise RuntimeError(f"WPE bridge was not installed at {bridge}")
    for tool, requirement in SANDBOX_TOOLS.items():
        if not tool.is_file():
            raise RuntimeError(f"WPE sandbox requires {requirement} at {tool}")

    _, plugin_sources = copy_plugin_packages(root)
    seeds = [path for path in root.rglob("*") if is_elf(path)]
    packaged_sources, system_requirements = copy_dependency_closure(
        root, seeds
    )
    packaged_sources.update(plugin_sources)
    set_relative_rpaths(root)
    validate_glibc(root, arguments.maximum_glibc)
    system_packages = copy_licenses(
        root, arguments.source, arguments.repository, packaged_sources
    )
    write_metadata(
        root,
        arguments.version,
        arguments.architecture,
        arguments.released,
        system_requirements,
        system_packages,
    )

    arguments.output.mkdir(parents=True, exist_ok=True)
    filename = (
        f"waterui-wpe-{arguments.version}-linux-{arguments.architecture}.zip"
    )
    archive = arguments.output / filename
    write_zip(root, archive, arguments.released)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    artifact = {
        "engine": "wpe",
        "version": arguments.version,
        "platform": "linux",
        "architecture": arguments.architecture,
        "file": filename,
        "sha256": checksum,
        "size": archive.stat().st_size,
    }
    metadata = arguments.output / f"{filename}.artifact.json"
    metadata.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
