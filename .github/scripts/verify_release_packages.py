#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ReleasePackage:
    name: str
    version: str
    has_dependency_target: bool
    manifest_dir: Path


def run(command: list[str], cwd: Path) -> None:
    print("+", " ".join(command))
    subprocess.run(command, cwd=cwd, check=True)


def capture(command: list[str], cwd: Path) -> str:
    return subprocess.check_output(command, cwd=cwd, text=True)


def load_release_package_names(repo_root: Path) -> list[str]:
    release_config = tomllib.loads((repo_root / "release-plz.toml").read_text())
    return [
        package["name"]
        for package in release_config.get("package", [])
        if package.get("release", True)
    ]


def load_workspace_packages(repo_root: Path) -> dict[str, ReleasePackage]:
    metadata = json.loads(
        capture(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=repo_root)
    )
    packages: dict[str, ReleasePackage] = {}
    for package in metadata["packages"]:
        target_kinds = {kind for target in package["targets"] for kind in target["kind"]}
        has_dependency_target = any(
            kind in {"lib", "proc-macro"} for kind in target_kinds
        )
        packages[package["name"]] = ReleasePackage(
            name=package["name"],
            version=package["version"],
            has_dependency_target=has_dependency_target,
            manifest_dir=Path(package["manifest_path"]).parent,
        )
    return packages


def unpack_crate(crate_path: Path, destination: Path) -> Path:
    with tarfile.open(crate_path) as archive:
        archive.extractall(destination)
    package_root = destination / crate_path.stem
    if not package_root.is_dir():
        raise RuntimeError(f"packaged crate root missing after unpack: {package_root}")
    return package_root


def create_consumer(
    crate_name: str,
    dependency_path: Path,
    packaged_roots: dict[str, Path],
    root: Path,
) -> Path:
    consumer_dir = root / f"consumer-{crate_name}"
    run(
        [
            "cargo",
            "init",
            "--lib",
            "--name",
            f"consumer_{crate_name.replace('-', '_')}",
            str(consumer_dir),
        ],
        cwd=root,
    )
    cargo_toml = consumer_dir / "Cargo.toml"
    cargo_toml_lines = [
        "[package]",
        f'name = "consumer_{crate_name.replace("-", "_")}"',
        'version = "0.1.0"',
        'edition = "2024"',
        "",
        "[dependencies]",
        f'{crate_name} = {{ path = "{dependency_path.as_posix()}" }}',
        "",
        "[patch.crates-io]",
    ]
    cargo_toml_lines.extend(
        f'{package_name} = {{ path = "{package_path.as_posix()}" }}'
        for package_name, package_path in sorted(packaged_roots.items())
    )
    cargo_toml_lines.append("")
    cargo_toml.write_text("\n".join(cargo_toml_lines))
    return consumer_dir


def release_package_patch_config(
    repo_root: Path, package: ReleasePackage, packages: list[ReleasePackage]
) -> list[str]:
    config: list[str] = []
    for dependency in packages:
        if dependency.name == package.name:
            continue
        dependency_path = dependency.manifest_dir.relative_to(repo_root).as_posix()
        config.extend(
            [
                "--config",
                f'patch.crates-io."{dependency.name}".path="{dependency_path}"',
            ]
        )
    return config


def package_crate(
    repo_root: Path, package: ReleasePackage, packages: list[ReleasePackage]
) -> None:
    run(
        [
            "cargo",
            "package",
            "--no-verify",
            "-p",
            package.name,
            *release_package_patch_config(repo_root, package, packages),
        ],
        cwd=repo_root,
    )


def unpack_selected_packages(
    repo_root: Path, packages: list[ReleasePackage], destination: Path
) -> dict[str, Path]:
    unpacked_packages: dict[str, Path] = {}
    for package in packages:
        crate_file = repo_root / "target" / "package" / f"{package.name}-{package.version}.crate"
        if not crate_file.is_file():
            raise RuntimeError(f"expected packaged crate archive is missing: {crate_file}")
        unpacked_packages[package.name] = unpack_crate(crate_file, destination)
    return unpacked_packages


def verify_dependency_package(
    package: ReleasePackage, packaged_roots: dict[str, Path]
) -> None:
    consumer_root = Path(tempfile.mkdtemp(prefix=f"{package.name}-consumer-"))
    try:
        consumer_dir = create_consumer(
            package.name,
            packaged_roots[package.name],
            packaged_roots,
            consumer_root,
        )
        run(["cargo", "check"], cwd=consumer_dir)
    finally:
        shutil.rmtree(consumer_root)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Verify release packages build as standalone published dependencies."
    )
    parser.add_argument(
        "--package",
        dest="packages",
        action="append",
        help="Restrict verification to one or more release package names.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    release_package_names = load_release_package_names(repo_root)
    workspace_packages = load_workspace_packages(repo_root)

    selected_packages = (
        args.packages if args.packages is not None else release_package_names
    )

    unknown_packages = sorted(set(selected_packages) - set(release_package_names))
    if unknown_packages:
        unknown = ", ".join(unknown_packages)
        raise RuntimeError(f"unknown non-release packages requested: {unknown}")

    release_packages: list[ReleasePackage] = []
    for package_name in selected_packages:
        package = workspace_packages.get(package_name)
        if package is None:
            raise RuntimeError(f"release package missing from cargo metadata: {package_name}")
        release_packages.append(package)

    for package in release_packages:
        package_crate(repo_root, package, release_packages)

    unpack_root = Path(tempfile.mkdtemp(prefix="release-packages-"))
    try:
        packaged_roots = unpack_selected_packages(repo_root, release_packages, unpack_root)
        for package in release_packages:
            if package.has_dependency_target:
                verify_dependency_package(package, packaged_roots)
            else:
                print(
                    f"Skipping dependency-consumer verification for binary-only package {package.name}"
                )
    finally:
        shutil.rmtree(unpack_root)


if __name__ == "__main__":
    main()
