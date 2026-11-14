#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Materialize the Gradle wrapper assets this tree carries.

The wrapper JARs are binary assets and stay out of the repository; this
script installs them where `./gradlew` expects them:

    uv run backends/hydrolysis/scripts/fetch-gradle-wrapper.py

The in-tree host's `distributionUrl` derives from a declaration, never a
literal kept in step by hand: it follows `[package.metadata.waterui]`'s
`android-gradle-version` in the repository's root manifest — the release the
scaffolded `gradle-wrapper.properties` renders — and this script rewrites it
from that declaration first. The bench reference is a frozen import whose
committed wrapper is left as imported. For each wrapper directory the script
then reads the pinned Gradle version, downloads the pinned
`gradle-<version>-bin.zip` from services.gradle.org verified against the
published `-bin.zip.sha256`, extracts the `gradle-wrapper.jar` template
the `wrapper` task writes (a resource inside
`gradle-wrapper-main-<version>.jar`), and verifies it byte-for-byte against
the published `-wrapper.jar.sha256` (<https://gradle.org/release-checksums/>)
before installing it. Both layers are hash-verified: the distribution and
the jar.
"""

from __future__ import annotations

import hashlib
import io
import re
import sys
import tomllib
import urllib.request
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Every Gradle wrapper this tree carries. Each has a
# `gradle/wrapper/gradle-wrapper.properties` beside it; the jar lands in the
# same directory.
WRAPPERS = [
    REPO / "android",
    REPO / "bench" / "android" / "reference",
]


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str) -> bytes:
    # The two wrappers pin the same Gradle version; fetch each URL once per run.
    cached = _FETCH_CACHE.get(url)
    if cached is not None:
        return cached
    print(f"fetch {url}")
    data = None
    for attempt in range(4):
        try:
            data = urllib.request.urlopen(url, timeout=120).read()
            break
        except Exception as error:  # noqa: BLE001 — report and retry, pin wins
            if attempt == 3:
                raise SystemExit(f"fetch {url} failed after 4 attempts: {error}")
            import time

            time.sleep(2**attempt * 2)
    _FETCH_CACHE[url] = data
    return data


_FETCH_CACHE: dict[str, bytes] = {}


def fetch_sha256(url: str) -> str:
    text = fetch(url).decode().strip()
    # The checksum files may carry just the digest or `digest  name`.
    return text.split()[0]


def declared_gradle_version() -> str:
    """`[package.metadata.waterui].android-gradle-version` in the root
    manifest — the one declaration the in-tree host follows and the
    scaffolded `gradle-wrapper.properties` renders."""
    with (REPO.parent.parent / "Cargo.toml").open("rb") as manifest:
        metadata = tomllib.load(manifest)["package"]["metadata"]["waterui"]
    version = metadata.get("android-gradle-version")
    if not isinstance(version, str) or not version.strip():
        raise SystemExit(
            "[package.metadata.waterui] declares no android-gradle-version string"
        )
    return version


def pin_wrapper(properties_path: Path, version: str) -> None:
    """Rewrite `distributionUrl` from the wrapper's declared release,
    leaving the file untouched when it already pins it — a no-diff run is a
    clean tree."""
    properties = properties_path.read_text()
    url = (
        "distributionUrl=https\\\\://services.gradle.org/distributions/"
        f"gradle-{version}-bin.zip"
    )
    pinned = re.sub(r"(?m)^distributionUrl=.*$", url, properties)
    if "distributionUrl=" not in pinned:
        raise SystemExit(f"no distributionUrl in {properties_path}")
    if pinned != properties:
        properties_path.write_text(pinned)
        print(
            f"  pinned {properties_path.relative_to(REPO.parent.parent)} "
            f"at gradle-{version}"
        )


def main() -> None:
    pin_wrapper(
        REPO / "android" / "gradle" / "wrapper" / "gradle-wrapper.properties",
        declared_gradle_version(),
    )

    for project in WRAPPERS:
        wrapper_dir = project / "gradle" / "wrapper"
        properties = (wrapper_dir / "gradle-wrapper.properties").read_text()
        match = re.search(r"distributionUrl=.*gradle-([\d.]+)-bin\.zip", properties)
        if match is None:
            raise SystemExit(f"no Gradle version in {wrapper_dir}")
        version = match.group(1)

        published_jar_sha = fetch_sha256(
            f"https://services.gradle.org/distributions/gradle-{version}-wrapper.jar.sha256"
        )
        published_zip_sha = fetch_sha256(
            f"https://services.gradle.org/distributions/gradle-{version}-bin.zip.sha256"
        )
        archive = fetch(
            f"https://services.gradle.org/distributions/gradle-{version}-bin.zip"
        )
        digest = sha256(archive)
        if digest != published_zip_sha:
            raise SystemExit(
                f"gradle-{version}-bin.zip sha256 mismatch:\n"
                f"  got      {digest}\n  expected {published_zip_sha}"
            )

        with zipfile.ZipFile(io.BytesIO(archive)) as zf:
            inner = zf.read(
                f"gradle-{version}/lib/plugins/gradle-wrapper-main-{version}.jar"
            )
        with zipfile.ZipFile(io.BytesIO(inner)) as zf:
            jar = zf.read("gradle-wrapper.jar")
        digest = sha256(jar)
        if digest != published_jar_sha:
            raise SystemExit(
                f"gradle-wrapper.jar sha256 mismatch:\n"
                f"  got      {digest}\n  expected {published_jar_sha}"
            )

        out = wrapper_dir / "gradle-wrapper.jar"
        out.write_bytes(jar)
        print(f"  wrote {out.relative_to(REPO)} ({len(jar)} bytes, {digest[:12]}…)")


if __name__ == "__main__":
    sys.exit(main())
