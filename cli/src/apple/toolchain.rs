//! Apple toolchain module

use eyre::Context as _;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;

use crate::toolchain::{Host, Toolchain, ToolchainError};

/// Represents the complete Apple toolchain consisting of Xcode and an Apple SDK
pub type AppleToolchain = (Xcode, AppleSdk);

/// Represents the Xcode toolchain
#[derive(Debug, Clone, Default)]
pub struct Xcode;

impl Toolchain for Xcode {
    type Installation = Infallible;
    async fn check(
        &self,
        host: &Host,
    ) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if Xcode is installed and available
        if host.which("xcodebuild").await.is_ok() && host.which("xcode-select").await.is_ok() {
            Ok(())
        } else {
            Err(ToolchainError::unfixable(
                "Xcode is not installed or not found in PATH",
                "Please install Xcode from the App Store or the Apple Developer website and ensure it's available in your PATH.",
            ))
        }
    }
}

/// Represents an Apple SDK (e.g., iOS, macOS)
#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub enum AppleSdk {
    /// iOS SDK
    #[serde(rename = "iOS")]
    Ios,
    /// iOS Simulator SDK
    #[serde(rename = "iOS Simulator")]
    IosSimulator,
    /// macOS SDK
    #[serde(rename = "macOS")]
    Macos,
    /// tvOS SDK
    #[serde(rename = "tvOS")]
    TvOs,
    /// watchOS SDK
    #[serde(rename = "watchOS")]
    WatchOs,
    /// visionOS SDK
    #[serde(rename = "visionOS")]
    VisionOs,
}

impl AppleSdk {
    /// Get the SDK name as used by `xcrun`
    #[must_use]
    pub const fn sdk_name(&self) -> &str {
        match self {
            Self::Ios => "iphoneos",
            Self::IosSimulator => "iphonesimulator",
            Self::Macos => "macosx",
            Self::TvOs => "appletvos",
            Self::WatchOs => "watchos",
            Self::VisionOs => "xros",
        }
    }
}

/// The development team used to sign device builds.
///
/// Physical-device builds must be signed in every profile — iOS refuses
/// unsigned code outright — so `DEVELOPMENT_TEAM` cannot come from the Xcode
/// project (which does not know the developer's team) and is resolved here
/// instead. Preference order:
///
/// 1. The team Xcode last provisioned with
///    (`IDEProvisioningTeamManagerLastSelectedTeamID`), then any other team
///    Xcode knows an account for (`IDEProvisioningTeamByIdentifier`). A
///    signed-in account can mint both the provisioning profile and the
///    "Apple Development" certificate it needs, so these work even when the
///    keychain holds no matching identity yet.
/// 2. The team embedded in a keychain development certificate — Apple
///    records the team as the certificate subject's organizational unit
///    (`OU`); the `(...)` suffix in the common name is the certificate's
///    own identifier, not the team. Such a team is only usable when a
///    matching profile is already installed locally; without an Xcode
///    account the portal cannot mint one, which is why it ranks last.
///
/// # Errors
/// Fails when neither an Xcode account team nor a development certificate
/// exists — the error tells the user where to add an account.
pub async fn development_team_id(host: &Host) -> eyre::Result<String> {
    if let Some(team) = xcode_account_team(host).await? {
        return Ok(team);
    }
    #[cfg(target_os = "macos")]
    for certificate in keychain_certificates(host).await? {
        if let Some(team) = team_id_in_certificate(&certificate) {
            return Ok(team);
        }
    }
    Err(eyre::eyre!(
        "No signing team found. Physical iOS builds must be signed: open \
         Xcode → Settings → Accounts and sign in an Apple ID (a free \
         account is enough), then re-run `water run`."
    ))
}

/// Every certificate the keychain lists, as DER bytes.
///
/// `security find-certificate -a -Z -p` prints every keychain certificate
/// as a PEM block preceded by its `SHA-256 hash:` line; the PEM reader
/// keeps only the certificate blocks.
#[cfg(target_os = "macos")]
pub(crate) async fn keychain_certificates(host: &Host) -> eyre::Result<Vec<Vec<u8>>> {
    let output = host
        .output("security", ["find-certificate", "-a", "-Z", "-p"])
        .await
        .wrap_err("failed to run `security find-certificate`")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(pem_certificate_der(&stdout))
}

/// Every PEM certificate block in `text` decoded to DER.
#[cfg(target_os = "macos")]
pub(crate) fn pem_certificate_der(text: &str) -> Vec<Vec<u8>> {
    x509_parser::pem::Pem::iter_from_buffer(text.as_bytes())
        .filter_map(std::result::Result::ok)
        .filter(|pem| pem.label == "CERTIFICATE")
        .map(|pem| pem.contents)
        .collect()
}

/// The team ID a development certificate carries — the subject's first
/// organizational unit. Returns `None` for non-development certificates.
#[cfg(target_os = "macos")]
fn team_id_in_certificate(der: &[u8]) -> Option<String> {
    use x509_parser::prelude::FromDer as _;
    let (_, certificate) = x509_parser::certificate::X509Certificate::from_der(der).ok()?;
    let subject = certificate.subject();
    let development = subject
        .iter_common_name()
        .filter_map(|name| name.as_str().ok())
        .any(is_development_certificate_name);
    if !development {
        return None;
    }
    subject
        .iter_organizational_unit()
        .next()?
        .as_str()
        .ok()
        .map(str::to_owned)
}

/// The common-name prefixes Apple's development certificates carry.
#[cfg(target_os = "macos")]
pub(crate) fn is_development_certificate_name(common_name: &str) -> bool {
    common_name.contains("Apple Development:")
        || common_name.contains("iPhone Developer:")
        || common_name.contains("iOS Development:")
}

/// SHA-1 of a DER certificate as uppercase hex — the hash `find-identity`
/// prints for each identity.
#[cfg(target_os = "macos")]
pub(crate) fn certificate_sha1_hex(der: &[u8]) -> String {
    use sha1::Digest as _;
    hex::encode_upper(sha1::Sha1::digest(der))
}

/// SHA-1 hashes of every certificate in a decoded profile's
/// `DeveloperCertificates` array.
#[cfg(target_os = "macos")]
pub(crate) fn profile_certificate_hashes(
    data: &plist::Dictionary,
) -> std::collections::HashSet<String> {
    data.get("DeveloperCertificates")
        .and_then(plist::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_data().map(certificate_sha1_hex))
        .collect()
}

/// `(sha-1, display name)` pairs `security find-identity -v -p
/// codesigning` lists, for development identities only — each entry looks
/// like `  1) 40HEXDIGITS "Apple Development: name (id)"`.
#[cfg(target_os = "macos")]
pub(crate) fn development_identities(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let entry = line.split_once(')')?.1.trim();
            let (hash, name) = entry.split_once(' ')?;
            let name = name.trim().trim_matches('"');
            is_development_certificate_name(name).then(|| (hash.to_string(), name.to_string()))
        })
        .collect()
}

/// The identity hash the profile's `DeveloperCertificates` names, if the
/// keychain holds it — pairing a profile with the `codesign --sign`
/// identity that can actually use it.
#[cfg(target_os = "macos")]
pub(crate) fn identity_for_profile(
    identities: &[(String, String)],
    data: &plist::Dictionary,
) -> Option<String> {
    let accepted = profile_certificate_hashes(data);
    identities
        .iter()
        .map(|(hash, _)| hash)
        .find(|hash| accepted.contains(*hash))
        .cloned()
}

/// A team record under `IDEProvisioningTeamByIdentifier` in Xcode's
/// preferences — the map key is the *account* UUID, so the usable team ID
/// only ever comes from the record.
#[derive(Deserialize)]
struct XcodeTeamRecord {
    /// The Apple Developer team this account may provision under.
    #[serde(rename = "teamID")]
    team_id: String,
}

/// Xcode's account registry — only the keys the signing path reads.
#[derive(Deserialize)]
struct XcodePreferences {
    /// `IDEProvisioningTeamManagerLastSelectedTeamID` — the team Xcode last
    /// provisioned with, when it recorded one.
    #[serde(rename = "IDEProvisioningTeamManagerLastSelectedTeamID", default)]
    last_selected_team_id: Option<String>,
    /// `IDEProvisioningTeamByIdentifier` — account UUID → team records.
    #[serde(rename = "IDEProvisioningTeamByIdentifier", default)]
    teams_by_identifier: std::collections::BTreeMap<String, Vec<XcodeTeamRecord>>,
}

/// Pick the development team out of the parsed registry.
///
/// Selection order: `last_selected` when a signed-in account still
/// advertises that team — a stale key from a removed account is never
/// used — then the first team the accounts advertise (accounts sort by
/// UUID), then `None` so the certificate path runs.
fn select_xcode_account_team(preferences: &XcodePreferences) -> Option<String> {
    let mut teams: Vec<&str> = Vec::new();
    for records in preferences.teams_by_identifier.values() {
        for record in records {
            if !teams.contains(&record.team_id.as_str()) {
                teams.push(&record.team_id);
            }
        }
    }
    if let Some(selected) = preferences
        .last_selected_team_id
        .as_deref()
        .filter(|team| !team.is_empty())
        && teams.contains(&selected)
    {
        return Some(selected.to_string());
    }
    teams.first().map(|team| (*team).to_string())
}

/// A team Xcode can provision for: the account's last-selected team, or any
/// team its accounts advertise. Reads Xcode's account registry once from
/// `~/Library/Preferences/com.apple.dt.Xcode.plist`; a missing Xcode install
/// or unsigned-in state yields `None` while a corrupt or unreadable file is
/// an error, never a silent fall-through.
///
/// `IDEProvisioningTeamByIdentifier` maps *account* UUIDs to lists of team
/// records — the usable team lives in each record's `teamID`, never in the
/// key itself.
async fn xcode_account_team(host: &Host) -> eyre::Result<Option<String>> {
    let Some(home) = host.home_dir() else {
        return Ok(None);
    };
    let plist = home.join("Library/Preferences/com.apple.dt.Xcode.plist");
    let bytes = match smol::fs::read(&plist).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to read {}", plist.display()));
        }
    };
    let preferences: XcodePreferences = plist::from_bytes(&bytes)
        .wrap_err_with(|| format!("failed to parse {}", plist.display()))?;
    Ok(select_xcode_account_team(&preferences))
}

impl std::fmt::Display for AppleSdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid panics in Display; serde is for config IO, not formatting.
        match self {
            Self::Ios => "iOS",
            Self::IosSimulator => "iOS Simulator",
            Self::Macos => "macOS",
            Self::TvOs => "tvOS",
            Self::WatchOs => "watchOS",
            Self::VisionOs => "visionOS",
        }
        .fmt(f)
    }
}

impl Toolchain for AppleSdk {
    type Installation = Infallible;
    async fn check(
        &self,
        host: &Host,
    ) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if the required Apple SDK is available
        let result = host
            .run("xcrun", ["--sdk", self.sdk_name(), "--show-sdk-path"])
            .await;

        if result.is_err() {
            return Err(ToolchainError::unfixable(
                format!("{self} SDK is not installed or not available"),
                format!(
                    "Please install {self} SDK through Xcode or use xcode-select to configure the active developer directory."
                ),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppleSdk, Xcode, XcodePreferences, XcodeTeamRecord, select_xcode_account_team,
        xcode_account_team,
    };
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[test]
    fn xcode_ok_when_tools_on_path() {
        let machine = TestMachine::new();
        machine.install("xcodebuild");
        machine.install("xcode-select");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Xcode.check(&host)).expect("xcodebuild + xcode-select on PATH must be ok");
    }

    #[test]
    fn xcode_missing_is_unfixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Xcode.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "Xcode requires a manual App Store install: {result:?}"
        );
    }

    #[test]
    fn apple_sdk_ok_when_xcrun_reports_path() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        machine.respond("XCRUN_SDK_PATH", "/fake/SDKs/iPhoneOS.sdk\n");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(AppleSdk::Ios.check(&host))
            .expect("an SDK path from xcrun must satisfy the check");
    }

    #[test]
    fn apple_sdk_missing_is_unfixable() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AppleSdk::Ios.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "xcrun without an SDK path must be unfixable: {result:?}"
        );
    }

    /// A registry the way the parser sees it: account UUIDs → team records,
    /// plus an optional last-selected team.
    fn preferences(last_selected: Option<&str>, accounts: &[(&str, &[&str])]) -> XcodePreferences {
        XcodePreferences {
            last_selected_team_id: last_selected.map(str::to_string),
            teams_by_identifier: accounts
                .iter()
                .map(|(account, teams)| {
                    (
                        (*account).to_string(),
                        teams
                            .iter()
                            .map(|team| XcodeTeamRecord {
                                team_id: (*team).to_string(),
                            })
                            .collect(),
                    )
                })
                .collect(),
        }
    }

    /// Serialize a preferences fixture as a real XML property list at
    /// `~/Library/Preferences/com.apple.dt.Xcode.plist` in the fake home.
    fn write_xcode_plist(machine: &TestMachine, value: &serde_json::Value) {
        let preferences_dir = machine.home().join("Library/Preferences");
        std::fs::create_dir_all(&preferences_dir).expect("create preferences directory");
        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, value).expect("fixture serializes as a plist");
        std::fs::write(preferences_dir.join("com.apple.dt.Xcode.plist"), bytes)
            .expect("write preferences fixture");
    }

    /// The shape a real Xcode writes: the account UUID keys the map, the
    /// usable team ID lives in each record's `teamID`.
    #[test]
    fn account_team_comes_from_the_record_not_the_key() {
        let prefs = preferences(
            None,
            &[("DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C", &["4AZ53N9R83"])],
        );
        assert_eq!(
            select_xcode_account_team(&prefs),
            Some("4AZ53N9R83".to_string()),
            "the account UUID must never be returned as the team"
        );
    }

    #[test]
    fn account_team_prefers_an_eligible_last_selected_team() {
        let prefs = preferences(
            Some("4AZ53N9R83"),
            &[(
                "DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C",
                &["4AZ53N9R83", "PAIDTEAM99"],
            )],
        );
        assert_eq!(
            select_xcode_account_team(&prefs),
            Some("4AZ53N9R83".to_string())
        );
    }

    #[test]
    fn a_stale_last_selected_team_is_never_returned() {
        let prefs = preferences(
            Some("REMOVED42"),
            &[("DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C", &["4AZ53N9R83"])],
        );
        assert_eq!(
            select_xcode_account_team(&prefs),
            Some("4AZ53N9R83".to_string()),
            "a last-selected team no account advertises falls back to records"
        );
        assert_eq!(
            select_xcode_account_team(&preferences(Some("REMOVED42"), &[])),
            None,
            "without team records the stale key cannot be verified"
        );
    }

    #[test]
    fn account_team_absent_without_registry_data() {
        assert_eq!(select_xcode_account_team(&preferences(None, &[])), None);
        assert_eq!(
            select_xcode_account_team(&preferences(
                None,
                &[("DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C", &[])],
            )),
            None
        );
    }

    #[test]
    fn without_a_last_selected_team_the_first_account_team_wins() {
        let prefs = preferences(
            None,
            &[
                ("DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C", &["4AZ53N9R83"]),
                ("A1B2C3D4-E5F6-7890-ABCD-EF1234567890", &["OTHERTEAM1"]),
            ],
        );
        // Account UUIDs sort deterministically: A1B2… precedes DE0D….
        assert_eq!(
            select_xcode_account_team(&prefs),
            Some("OTHERTEAM1".to_string())
        );
    }

    #[test]
    fn the_same_team_across_accounts_counts_once() {
        let prefs = preferences(
            None,
            &[
                ("DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C", &["4AZ53N9R83"]),
                ("A1B2C3D4-E5F6-7890-ABCD-EF1234567890", &["4AZ53N9R83"]),
            ],
        );
        assert_eq!(
            select_xcode_account_team(&prefs),
            Some("4AZ53N9R83".to_string())
        );
    }

    #[test]
    fn account_team_reads_a_real_preferences_file() {
        let machine = TestMachine::new();
        write_xcode_plist(
            &machine,
            &serde_json::json!({
                "IDEProvisioningTeamByIdentifier": {
                    "DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C": [{
                        "teamID": "4AZ53N9R83",
                        "teamName": "Personal Team",
                        "isFreeProvisioningTeam": true,
                        "teamType": "Personal Team"
                    }]
                }
            }),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(
            smol::block_on(xcode_account_team(&host)).unwrap(),
            Some("4AZ53N9R83".to_string())
        );
    }

    #[test]
    fn a_real_stale_last_selected_falls_back_to_records() {
        let machine = TestMachine::new();
        write_xcode_plist(
            &machine,
            &serde_json::json!({
                "IDEProvisioningTeamManagerLastSelectedTeamID": "REMOVED42",
                "IDEProvisioningTeamByIdentifier": {
                    "DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C": [{"teamID": "4AZ53N9R83"}]
                }
            }),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(
            smol::block_on(xcode_account_team(&host)).unwrap(),
            Some("4AZ53N9R83".to_string())
        );
    }

    #[test]
    fn a_missing_preferences_file_is_not_a_team() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(smol::block_on(xcode_account_team(&host)).unwrap(), None);
    }

    #[test]
    fn a_corrupt_preferences_file_is_an_error() {
        let machine = TestMachine::new();
        let dir = machine.home().join("Library/Preferences");
        std::fs::create_dir_all(&dir).expect("create preferences directory");
        std::fs::write(dir.join("com.apple.dt.Xcode.plist"), b"not a plist")
            .expect("write corrupt fixture");
        let host = machine.host(Vec::<(String, String)>::new());
        assert!(smol::block_on(xcode_account_team(&host)).is_err());
    }

    #[test]
    fn an_unreadable_preferences_path_is_an_error() {
        let machine = TestMachine::new();
        std::fs::create_dir_all(
            machine
                .home()
                .join("Library/Preferences/com.apple.dt.Xcode.plist"),
        )
        .expect("create directory in place of the plist");
        let host = machine.host(Vec::<(String, String)>::new());
        assert!(smol::block_on(xcode_account_team(&host)).is_err());
    }

    #[test]
    fn a_team_record_without_team_id_is_an_error() {
        let machine = TestMachine::new();
        write_xcode_plist(
            &machine,
            &serde_json::json!({
                "IDEProvisioningTeamByIdentifier": {
                    "DE0DE8B1-3DBB-4F70-B15D-B16BF1872F1C": [{"teamName": "Personal Team"}]
                }
            }),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        assert!(smol::block_on(xcode_account_team(&host)).is_err());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod signing_tests {
    use super::{
        certificate_sha1_hex, development_identities, identity_for_profile,
        is_development_certificate_name, pem_certificate_der, profile_certificate_hashes,
        team_id_in_certificate,
    };

    /// A self-signed certificate shaped like an `Apple Development` one:
    /// `CN=Apple Development: <email> (<certificate-id>)`, `OU=<team>`.
    /// Generated for these tests; identifies nothing real.
    const DEVELOPMENT_CERT: &str = include_str!("../toolchain/testdata/apple_development.pem");
    /// A self-signed certificate whose common name is not an Apple
    /// development name.
    const OTHER_CERT: &str = include_str!("../toolchain/testdata/other_signing.pem");
    /// `sha1sum` of the development fixture's DER.
    const DEV_CERT_SHA1: &str = "5D03DD01B4F95D47874C9BFD9367A978D838A228";
    const OTHER_CERT_SHA1: &str = "AAAAAAAABBBBBBBBCCCCCCCCDDDDDDDDEEEEEEEE";

    fn dev_cert_der() -> Vec<u8> {
        x509_parser::pem::Pem::iter_from_buffer(DEVELOPMENT_CERT.as_bytes())
            .next()
            .expect("the fixture holds one PEM block")
            .expect("the fixture PEM decodes")
            .contents
    }

    fn find_identity_output() -> String {
        format!(
            "     1) {DEV_CERT_SHA1} \"Apple Development: devin.test@example.com (TESTCERT42)\"\n     2) {OTHER_CERT_SHA1} \"Apple Development: devin.other@example.com (OTHERID9X)\"\n     2 valid identities found\n"
        )
    }

    #[test]
    fn certificate_sha1_matches_openssl() {
        assert_eq!(certificate_sha1_hex(&dev_cert_der()), DEV_CERT_SHA1);
    }

    #[test]
    fn identity_pairs_with_a_profile_certificate() {
        let profile = plist::Dictionary::from_iter([(
            "DeveloperCertificates".to_string(),
            plist::Value::Array(vec![plist::Value::Data(dev_cert_der())]),
        )]);
        let identities = development_identities(&find_identity_output());
        assert_eq!(
            identity_for_profile(&identities, &profile).as_deref(),
            Some(DEV_CERT_SHA1)
        );
    }

    #[test]
    fn identity_absent_when_profile_lists_another_certificate() {
        let profile = plist::Dictionary::from_iter([(
            "DeveloperCertificates".to_string(),
            plist::Value::Array(vec![plist::Value::Data(vec![0xDE, 0xAD])]),
        )]);
        let identities = development_identities(&find_identity_output());
        assert_eq!(identity_for_profile(&identities, &profile), None);
    }

    #[test]
    fn non_development_identities_are_skipped() {
        let identities = development_identities(&format!(
            "     1) {DEV_CERT_SHA1} \"Devin Signing Test\"\n     1 valid identities found\n"
        ));
        assert_eq!(identities, []);
    }

    #[test]
    fn find_identity_parsing_keeps_development_names_only() {
        let output = find_identity_output()
            + "     3) DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF \"Apple Distribution: x\"\n";
        let identities = development_identities(&output);
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].0, DEV_CERT_SHA1);
    }

    #[test]
    fn profile_hash_set_covers_every_certificate() {
        let profile = plist::Dictionary::from_iter([(
            "DeveloperCertificates".to_string(),
            plist::Value::Array(vec![
                plist::Value::Data(dev_cert_der()),
                plist::Value::Data(vec![0x01, 0x02]),
            ]),
        )]);
        let hashes = profile_certificate_hashes(&profile);
        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains(DEV_CERT_SHA1));
    }

    #[test]
    fn team_id_comes_from_subject_organizational_unit() {
        let [certificate] = pem_certificate_der(DEVELOPMENT_CERT)
            .try_into()
            .expect("the fixture holds exactly one certificate");
        assert_eq!(
            team_id_in_certificate(&certificate).as_deref(),
            Some("TESTTEAM42"),
            "the team is the subject OU, not the common name's (…) suffix"
        );
    }

    #[test]
    fn non_development_certificate_yields_no_team() {
        let [certificate] = pem_certificate_der(OTHER_CERT)
            .try_into()
            .expect("the fixture holds exactly one certificate");
        assert_eq!(team_id_in_certificate(&certificate), None);
    }

    #[test]
    fn pem_reader_skips_find_certificate_hash_lines() {
        // `security find-certificate -a -Z -p` interleaves `SHA-256 hash:` lines.
        let output = format!("SHA-256 hash: 00FF\n{DEVELOPMENT_CERT}SHA-256 hash: ABCD\n");
        let certificates = pem_certificate_der(&output);
        assert_eq!(certificates.len(), 1);
    }

    #[test]
    fn development_common_names() {
        assert!(is_development_certificate_name(
            "Apple Development: a@b.c (XYZ)"
        ));
        assert!(is_development_certificate_name("iPhone Developer: a@b.c"));
        assert!(!is_development_certificate_name("Devin Signing Test"));
    }
}
