//! Native dependencies reported by rustc for a static archive's final host link.

use eyre::{Result, bail};

use super::{CargoTarget, RustBuild, combined_build_output};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLink {
    pub name: String,
    pub framework: bool,
}

impl RustBuild {
    /// Ask the same Cargo invocation for rustc's native link contract. Cargo
    /// replays the compiler diagnostic on a fresh unit as well as a rebuilt one.
    ///
    /// # Errors
    /// Returns an error if the build fails, Cargo's output cannot be read,
    /// or rustc never reports the `native-static-libs` flags.
    pub async fn native_static_libraries(&self, release: bool) -> Result<Vec<NativeLink>> {
        let _lease = self.shared_target_lease().await?;
        let profile_dir = self.lib_output_dir(release).await?;
        let output = self
            .cargo_build_output(release, CargoTarget::Lib, &profile_dir)
            .await?;
        if !output.status.success() {
            bail!(
                "Failed to obtain static library link requirements: {}",
                combined_build_output(&output)
            );
        }
        for message in cargo_metadata::Message::parse_stream(output.stdout.as_slice()) {
            if let cargo_metadata::Message::CompilerMessage(message) = message?
                && let Some(flags) = message.message.message.strip_prefix("native-static-libs: ")
            {
                return parse(flags);
            }
        }
        let stderr = String::from_utf8(output.stderr)?;
        for line in stderr.lines() {
            if let Some((_, flags)) = line.split_once("native-static-libs: ") {
                return parse(flags);
            }
        }
        bail!("rustc did not report native-static-libs for the embedded archive")
    }
}

fn parse(flags: &str) -> Result<Vec<NativeLink>> {
    let mut words = flags.split_whitespace();
    let mut links = Vec::new();
    while let Some(flag) = words.next() {
        let (name, framework) = if flag == "-framework" {
            (
                words
                    .next()
                    .ok_or_else(|| eyre::eyre!("missing framework name in {flags}"))?,
                true,
            )
        } else if let Some(name) = flag.strip_prefix("-l") {
            (name, false)
        } else {
            bail!("Unsupported native static link requirement {flag:?} in {flags}");
        };
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_+.-".contains(c))
        {
            bail!("Invalid native library name {name:?}");
        }
        links.push(NativeLink {
            name: name.to_owned(),
            framework,
        });
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_native_link_order_and_repeated_libraries() {
        let links = parse("-lSystem -framework CoreFoundation -lc++ -lSystem").unwrap();
        assert_eq!(
            links
                .iter()
                .map(|link| link.name.as_str())
                .collect::<Vec<_>>(),
            ["System", "CoreFoundation", "c++", "System"]
        );
        assert!(links[1].framework);
        assert!(!links[2].framework);
    }

    #[test]
    fn malformed_or_unrepresented_flags_fail() {
        for flags in ["-framework", "-l", "-Wl,unknown", "-l\"unsafe\""] {
            assert!(parse(flags).is_err(), "{flags}");
        }
    }
}
