//! The `gradle-wrapper.jar` every Android scaffold needs but the
//! `android_shared` template does not embed.
//!
//! The repository carries no binary files, so the jar is materialized the
//! way Hydrolysis's wrappers are — lazily, the first time a Gradle task can
//! run (scaffolding itself must stay offline-capable): the Gradle version and
//! distribution flavor are read out of the scaffolded
//! `gradle-wrapper.properties`, the pinned `-bin.zip`/`-all.zip` is fetched
//! from services.gradle.org and verified against its published `.sha256`,
//! the `gradle-wrapper.jar` template is extracted from the
//! `gradle-wrapper-main-<version>.jar` inside it and verified against the
//! published `-wrapper.jar.sha256` (<https://gradle.org/release-checksums/>).
//! Both layers are hash-verified: the distribution and the jar. The jar
//! caches under the shared `~/.water/build_cache`, so one machine pays each
//! Gradle version once — a second `water create` and a scaffold whose jar
//! was cleaned away both resolve without the network.

use std::{
    io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use smol::fs;

const PROPERTIES: &str = "gradle/wrapper/gradle-wrapper.properties";
const JAR: &str = "gradle/wrapper/gradle-wrapper.jar";

/// Ensure `<project>/gradle/wrapper/gradle-wrapper.jar` exists beside the
/// scaffolded properties, fetching and hash-verifying it when missing.
///
/// Idempotent and callable anywhere a Gradle project is expected:
/// [`crate::android::platform::run_gradle_tasks`] invokes it before resolving
/// `gradlew`, so a scaffold made by an older CLI or one whose jar was deleted
/// is healed rather than broken.
pub async fn ensure(host: &crate::toolchain::Host, project: &Path) -> io::Result<()> {
    let jar = project.join(JAR);
    if fs::metadata(&jar).await.is_ok() {
        return Ok(());
    }
    let properties_path = project.join(PROPERTIES);
    let properties = fs::read_to_string(&properties_path).await?;
    let distribution = distribution(&properties).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} declares no gradle-*-(bin|all).zip distributionUrl — cannot pick \
                 the wrapper jar to fetch",
                properties_path.display()
            ),
        )
    })?;

    let cached = fetch_wrapper_jar(host, &distribution).await?;
    if let Some(parent) = jar.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::copy(&cached, &jar).await?;
    Ok(())
}

/// The Gradle distribution a `gradle-wrapper.properties` pins, read out of
/// its `distributionUrl` as `(version, flavor)` — `gradle-<version>-bin.zip`
/// or `-all.zip`, release or `-rc-N` tag alike.
fn distribution(properties: &str) -> Option<(String, String)> {
    let url = properties
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("distributionUrl="))?;
    let name = url.rsplit('/').next()?;
    let stem = name.strip_suffix(".zip")?.strip_prefix("gradle-")?;
    let (version, flavor) = stem.rsplit_once('-')?;
    (flavor == "bin" || flavor == "all").then(|| (version.to_string(), flavor.to_string()))
}

/// The published `.sha256` sidecar for `url`, as the bare digest (the files
/// may carry `digest  name`).
async fn fetch_sha256(url: &str) -> io::Result<String> {
    let body = waterui_assets_core::download_remote_bytes(url)
        .await
        .map_err(io::Error::other)?;
    String::from_utf8(body)
        .map_err(io::Error::other)?
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| io::Error::other(format!("{url} carried no digest")))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The verified jar under `~/.water/build_cache/gradle-wrapper/<version>/`,
/// fetching the pinned distribution and extracting the jar when absent.
/// Gradle does not serve the jar bare — it ships as a resource inside the
/// distribution's `gradle-wrapper-main-<version>.jar`.
async fn fetch_wrapper_jar(
    host: &crate::toolchain::Host,
    distribution: &(String, String),
) -> io::Result<PathBuf> {
    let (version, flavor) = distribution;
    let directory = crate::water_dir::build_cache_root(host)
        .await
        .map_err(io::Error::other)?
        .join("gradle-wrapper")
        .join(version);
    let jar = directory.join("gradle-wrapper.jar");
    if fs::metadata(&jar).await.is_ok() {
        return Ok(jar);
    }

    let base = format!("https://services.gradle.org/distributions/gradle-{version}");
    let expected_jar = fetch_sha256(&format!("{base}-wrapper.jar.sha256")).await?;
    let archive_url = format!("{base}-{flavor}.zip");
    let expected_archive = fetch_sha256(&format!("{archive_url}.sha256")).await?;
    let archive = waterui_assets_core::download_remote_bytes(&archive_url)
        .await
        .map_err(io::Error::other)?;
    let digest = sha256_hex(&archive);
    if digest != expected_archive {
        return Err(io::Error::other(format!(
            "{archive_url} sha256 mismatch: got {digest}, expected {expected_archive}"
        )));
    }

    // The template the `wrapper` task writes is a resource of the
    // distribution's `gradle-wrapper-main-<version>.jar`.
    let cursor = io::Cursor::new(&archive);
    let mut outer = zip::ZipArchive::new(cursor).map_err(io::Error::other)?;
    let inner_name = format!("gradle-{version}/lib/plugins/gradle-wrapper-main-{version}.jar");
    let mut inner_bytes = Vec::new();
    io::Read::read_to_end(&mut outer.by_name(&inner_name)?, &mut inner_bytes)?;
    drop(outer);
    let cursor = io::Cursor::new(&inner_bytes);
    let mut inner = zip::ZipArchive::new(cursor).map_err(io::Error::other)?;
    let mut jar_bytes = Vec::new();
    io::Read::read_to_end(&mut inner.by_name("gradle-wrapper.jar")?, &mut jar_bytes)?;

    let digest = sha256_hex(&jar_bytes);
    if digest != expected_jar {
        return Err(io::Error::other(format!(
            "gradle-wrapper.jar sha256 mismatch: got {digest}, expected {expected_jar}"
        )));
    }

    fs::create_dir_all(&directory).await?;
    fs::write(&jar, &jar_bytes).await?;
    Ok(jar)
}

#[cfg(test)]
mod tests {
    use super::distribution;

    #[test]
    fn parses_the_scaffolded_distribution_url() {
        let properties =
            "distributionUrl=https\\://services.gradle.org/distributions/gradle-9.6.1-bin.zip\n";
        assert_eq!(
            distribution(properties),
            Some(("9.6.1".to_string(), "bin".to_string()))
        );
    }

    #[test]
    fn parses_all_and_rc_distributions() {
        assert_eq!(
            distribution(
                "distributionUrl=https\\://services.gradle.org/distributions/gradle-8.10.2-all.zip"
            ),
            Some(("8.10.2".to_string(), "all".to_string()))
        );
        assert_eq!(
            distribution(
                "distributionUrl=https\\://services.gradle.org/distributions/gradle-9.0.0-rc-1-bin.zip"
            ),
            Some(("9.0.0-rc-1".to_string(), "bin".to_string()))
        );
    }
}
