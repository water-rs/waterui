//! Development provisioning profiles: decode, validate, select, obtain.
//!
//! A `.mobileprovision` file is a CMS `SignedData` envelope wrapping a
//! property list; it is decoded in-process with the `cms`/`der`/`plist`
//! crates rather than by parsing `security cms -D` output. When no installed
//! profile qualifies, one is obtained through `xcodebuild`.
//!
//! `xcodebuild` is used for exactly one thing: when no installed development
//! profile matches the team, bundle identifier, destination device and
//! entitlements, a throwaway project generated here is built with
//! `-allowProvisioningUpdates` so Xcode registers the device and installs a
//! profile. Building, packaging and signing stay on the Xcode-free path.

use std::fmt;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use askama::Template;
use cms::content_info::ContentInfo;
use cms::signed_data::SignedData;
use der::Decode;
use der::asn1::OctetStringRef;
use eyre::{Context, ContextCompat, bail};
use smol::fs;
use tracing::{debug, info};

use crate::platform::TargetPlatform;
use crate::toolchain::Host;

/// The directories Xcode installs provisioning profiles into.
fn profile_directories(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Library/MobileDevice/Provisioning Profiles"),
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
    ]
}

/// What the app being signed requires from a development provisioning
/// profile.
#[derive(Debug)]
pub struct SigningRequest<'a> {
    /// The development team the build signs under.
    pub team: &'a str,
    /// The bundle identifier of the app.
    pub bundle_id: &'a str,
    /// Hardware UDID of the destination device when the package is bound to
    /// one (`water run`); `None` for a device-agnostic package
    /// (`water package`), which accepts any profile covering at least one
    /// device.
    pub device_udid: Option<&'a str>,
    /// The entitlements the app declares — its `<scheme>.entitlements`
    /// dictionary. The selected profile must grant every entry.
    pub entitlements: &'a plist::Dictionary,
    /// The platform the app is signed for — drives the generated
    /// provisioning project's SDK and the xcodebuild destination.
    pub platform: TargetPlatform,
}

/// A `.mobileprovision` file that passed validation, together with its
/// decoded plist, the keychain identity it signs with, and the App ID
/// prefix the match was made under.
///
/// The identity and prefix are part of the selection so the signer can
/// never pair the profile with a different — or unusable — certificate.
#[derive(Debug)]
pub struct ProfileSelection {
    /// Location of the profile on disk.
    pub path: PathBuf,
    /// Its decoded plist dictionary.
    pub data: plist::Dictionary,
    /// SHA-1 of the usable development identity the profile's
    /// `DeveloperCertificates` names — the value `codesign --sign` takes.
    pub identity: String,
    /// The `ApplicationIdentifierPrefix` entry the App ID match used —
    /// often but not always the team ID (TN2415).
    pub app_id_prefix: String,
}

/// Why one installed `.mobileprovision` was rejected for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileRejection {
    /// `TeamIdentifier` does not contain the requested team.
    WrongTeam {
        /// Teams the profile names.
        profile_teams: Vec<String>,
    },
    /// `Entitlements.application-identifier` covers neither the bundle
    /// identifier nor a wildcard that contains it.
    AppIdMismatch {
        /// The profile's `application-identifier`, when it declares one.
        application_identifier: Option<String>,
    },
    /// `ExpirationDate` has passed.
    Expired,
    /// A key the signing path requires is absent or has the wrong type — a
    /// profile cannot be selected on metadata it does not carry.
    InvalidMetadata {
        /// The required plist key, e.g. `ExpirationDate`.
        key: &'static str,
    },
    /// The file is not a decodable `.mobileprovision`.
    Undecodable {
        /// The decoder's own description of what was wrong.
        reason: String,
    },
    /// A development profile must set `get-task-allow`; App Store / ad hoc /
    /// enterprise profiles do not.
    NotDevelopment,
    /// The destination device's UDID is absent from `ProvisionedDevices` and
    /// `ProvisionsAllDevices` is not set.
    DeviceNotProvisioned {
        /// The UDID the profile does not cover.
        udid: String,
    },
    /// The profile provisions no device at all — a `ProvisionedDevices`
    /// list that is absent or empty, with no `ProvisionsAllDevices`.
    MissingDeviceList,
    /// The profile does not grant every entitlement the app declares.
    MissingEntitlements {
        /// The entitlement keys the profile does not cover.
        keys: Vec<String>,
    },
    /// `DeveloperCertificates` names no certificate the keychain holds as
    /// an identity with a private key — the profile could never sign.
    NoUsableCertificate,
}

impl fmt::Display for ProfileRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongTeam { profile_teams } => {
                write!(f, "issued to team(s) {}, not the requested team", {
                    if profile_teams.is_empty() {
                        "<none>".to_string()
                    } else {
                        profile_teams.join(", ")
                    }
                })
            }
            Self::AppIdMismatch {
                application_identifier,
            } => write!(
                f,
                "application-identifier {} does not cover the bundle id",
                application_identifier.as_deref().unwrap_or("<absent>")
            ),
            Self::Expired => write!(f, "expired"),
            Self::InvalidMetadata { key } => {
                write!(
                    f,
                    "required metadata `{key}` is missing or has the wrong type"
                )
            }
            Self::Undecodable { reason } => {
                write!(f, "is not a decodable provisioning profile: {reason}")
            }
            Self::NotDevelopment => {
                write!(f, "not a development profile (no get-task-allow)")
            }
            Self::DeviceNotProvisioned { udid } => write!(
                f,
                "does not provision destination device {udid} and is not ProvisionsAllDevices"
            ),
            Self::MissingDeviceList => {
                write!(
                    f,
                    "provisions no devices (no ProvisionedDevices and no ProvisionsAllDevices)"
                )
            }
            Self::MissingEntitlements { keys } => write!(
                f,
                "does not grant the entitlements the app declares: {}",
                keys.join(", ")
            ),
            Self::NoUsableCertificate => write!(
                f,
                "is issued for a certificate the keychain does not hold with a private key"
            ),
        }
    }
}

/// No installed `.mobileprovision` satisfied the signing request; carries
/// every candidate's rejection so the failure names them all.
#[derive(Debug)]
pub struct NoMatchingProfile {
    /// `(path, rejection)` for each candidate that decoded; empty when no
    /// profiles are installed at all.
    rejections: Vec<(PathBuf, ProfileRejection)>,
}

impl fmt::Display for NoMatchingProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.rejections.is_empty() {
            return write!(
                f,
                "no provisioning profiles are installed under \
                 ~/Library/MobileDevice/Provisioning Profiles"
            );
        }
        write!(f, "no installed provisioning profile qualifies:")?;
        for (path, rejection) in &self.rejections {
            write!(
                f,
                "\n  {} — {rejection}",
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned()
                )
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for NoMatchingProfile {}

/// Decode a `.mobileprovision`'s CMS `SignedData` envelope into its plist
/// dictionary.
///
/// # Errors
/// Returns an error when the bytes are not a CMS `SignedData` carrying an
/// octet-string payload, or the payload is not a plist dictionary.
pub fn decode_mobileprovision(der_bytes: &[u8]) -> eyre::Result<plist::Dictionary> {
    let info = ContentInfo::from_der(der_bytes)
        .map_err(|error| eyre::eyre!("provisioning profile is not a CMS ContentInfo: {error}"))?;
    let signed: SignedData = info.content.decode_as().map_err(|error| {
        eyre::eyre!("provisioning profile's CMS content is not SignedData: {error}")
    })?;
    let econtent = signed
        .encap_content_info
        .econtent
        .wrap_err("provisioning profile carries no encapsulated content")?;
    let octets: OctetStringRef = econtent.decode_as().map_err(|error| {
        eyre::eyre!("provisioning profile's encapsulated content is not an octet string: {error}")
    })?;
    let value = plist::Value::from_reader(Cursor::new(octets.as_bytes()))
        .wrap_err("provisioning profile's payload is not a plist")?;
    let plist::Value::Dictionary(dictionary) = value else {
        bail!("provisioning profile's payload is not a plist dictionary");
    };
    Ok(dictionary)
}

/// The `TeamIdentifier` entries a decoded profile names.
fn profile_team_ids(data: &plist::Dictionary) -> Vec<String> {
    data.get("TeamIdentifier")
        .and_then(plist::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_string().map(str::to_string))
        .collect()
}

/// The profile's `Entitlements.application-identifier`.
fn profile_application_identifier(data: &plist::Dictionary) -> Option<&str> {
    data.get("Entitlements")
        .and_then(plist::Value::as_dictionary)
        .and_then(|entitlements| entitlements.get("application-identifier"))
        .and_then(plist::Value::as_string)
}

/// The `ApplicationIdentifierPrefix` entries a decoded profile declares.
///
/// `application-identifier` and the keychain-access-group grants are
/// prefixed with these — often the team ID, but not always (TN2415).
fn profile_app_id_prefixes(data: &plist::Dictionary) -> Vec<String> {
    data.get("ApplicationIdentifierPrefix")
        .and_then(plist::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_string().map(str::to_string))
        .collect()
}

/// How an `application-identifier` covers the request's bundle identifier:
/// `PREFIX.<bundle>` exactly or a `PREFIX.<prefix>.*` wildcard (which
/// includes the common `PREFIX.*`). `PREFIX` is one of the profile's
/// `ApplicationIdentifierPrefix` entries, not necessarily the team ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppIdMatch {
    /// `PREFIX.<bundle>` verbatim.
    Exact,
    /// `PREFIX.<prefix>.*` with `bundle` under `<prefix>` (including
    /// `PREFIX.*`).
    Wildcard,
}

fn app_id_match(application_identifier: &str, prefix: &str, bundle_id: &str) -> Option<AppIdMatch> {
    let expected = format!("{prefix}.{bundle_id}");
    if application_identifier == expected {
        return Some(AppIdMatch::Exact);
    }
    // A wildcard `PREFIX.<prefix>.*` covers bundle ids under `<prefix>`;
    // `PREFIX.*` covers every bundle id under the prefix.
    let wildcard = application_identifier.strip_prefix(&format!("{prefix}."))?;
    if wildcard == "*" {
        return Some(AppIdMatch::Wildcard);
    }
    let covered_prefix = wildcard.strip_suffix(".*")?;
    bundle_id
        .starts_with(&format!("{covered_prefix}."))
        .then_some(AppIdMatch::Wildcard)
}

/// Whether a profile entitlement value covers the value the app declares.
///
/// Strings match exactly or through a trailing `*` wildcard (`TEAM.*` covers
/// `TEAM.<anything>`); arrays and dictionaries cover element-wise.
fn entitlement_covers(granted: &plist::Value, requested: &plist::Value) -> bool {
    match (granted, requested) {
        (plist::Value::String(grant), plist::Value::String(ask)) => grant
            .strip_suffix('*')
            .map_or_else(|| grant == ask, |prefix| ask.starts_with(prefix)),
        (plist::Value::Array(grants), plist::Value::Array(asks)) => asks
            .iter()
            .all(|ask| grants.iter().any(|grant| entitlement_covers(grant, ask))),
        (plist::Value::Dictionary(grants), plist::Value::Dictionary(asks)) => {
            asks.iter().all(|(key, ask)| {
                grants
                    .get(key)
                    .is_some_and(|grant| entitlement_covers(grant, ask))
            })
        }
        _ => granted == requested,
    }
}

/// One candidate that satisfied every requirement: the App ID coverage
/// kind, the matched `ApplicationIdentifierPrefix`, and the SHA-1 of the
/// keychain development identity its `DeveloperCertificates` name.
#[derive(Debug, PartialEq)]
struct AcceptedCandidate {
    kind: AppIdMatch,
    app_id_prefix: String,
    identity: String,
}

/// Evaluate one decoded profile against the signing request, returning the
/// first reason it fails — or the accepted candidate. `identities` is the
/// usable development-identity inventory probed once per selection, so a
/// profile that matches the request but names a certificate the keychain
/// holds without its private key is rejected like any other mismatch.
fn evaluate_candidate(
    data: &plist::Dictionary,
    request: &SigningRequest<'_>,
    identities: &[(String, String)],
    now: SystemTime,
) -> Result<AcceptedCandidate, ProfileRejection> {
    let profile_teams = profile_team_ids(data);
    if !profile_teams.iter().any(|id| id == request.team) {
        return Err(ProfileRejection::WrongTeam { profile_teams });
    }

    // A selected profile must prove a valid future expiry: a missing or
    // mistyped `ExpirationDate` is rejected, never assumed valid.
    let expiration = match data.get("ExpirationDate").and_then(plist::Value::as_date) {
        Some(expires) => SystemTime::from(expires),
        None => {
            return Err(ProfileRejection::InvalidMetadata {
                key: "ExpirationDate",
            });
        }
    };
    if now >= expiration {
        return Err(ProfileRejection::Expired);
    }

    let entitlements = data
        .get("Entitlements")
        .and_then(plist::Value::as_dictionary);
    if entitlements
        .and_then(|dict| dict.get("get-task-allow"))
        .and_then(plist::Value::as_boolean)
        != Some(true)
    {
        return Err(ProfileRejection::NotDevelopment);
    }

    // TeamIdentifier is not always the App ID prefix (TN2415): the team
    // check above reads `TeamIdentifier`, while `application-identifier`
    // and the keychain grants are matched against `ApplicationIdentifierPrefix`.
    let prefixes = profile_app_id_prefixes(data);
    if prefixes.is_empty() {
        return Err(ProfileRejection::InvalidMetadata {
            key: "ApplicationIdentifierPrefix",
        });
    }
    let application_identifier = profile_application_identifier(data);
    let Some((kind, app_id_prefix)) = application_identifier.and_then(|id| {
        prefixes.iter().find_map(|prefix| {
            app_id_match(id, prefix, request.bundle_id).map(|kind| (kind, prefix.clone()))
        })
    }) else {
        return Err(ProfileRejection::AppIdMismatch {
            application_identifier: application_identifier.map(str::to_string),
        });
    };

    let all_devices = data
        .get("ProvisionsAllDevices")
        .and_then(plist::Value::as_boolean)
        .unwrap_or(false);
    if !all_devices {
        let devices = data
            .get("ProvisionedDevices")
            .and_then(plist::Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(plist::Value::as_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        match request.device_udid {
            // A device-bound run must provision the exact destination device.
            Some(udid) => {
                if !devices
                    .iter()
                    .any(|listed| listed.eq_ignore_ascii_case(udid))
                {
                    return Err(ProfileRejection::DeviceNotProvisioned {
                        udid: udid.to_string(),
                    });
                }
            }
            // Generic packaging accepts any profile bound to at least one
            // device — a development profile listing none is not one.
            None => {
                if devices.is_empty() {
                    return Err(ProfileRejection::MissingDeviceList);
                }
            }
        }
    }

    let missing: Vec<String> = request
        .entitlements
        .iter()
        .filter(|(key, value)| {
            entitlements
                .and_then(|dict| dict.get(key))
                .is_none_or(|granted| !entitlement_covers(granted, value))
        })
        .map(|(key, _)| key.clone())
        .collect();
    if !missing.is_empty() {
        return Err(ProfileRejection::MissingEntitlements { keys: missing });
    }

    let Some(identity) = crate::apple::toolchain::identity_for_profile(identities, data) else {
        return Err(ProfileRejection::NoUsableCertificate);
    };

    Ok(AcceptedCandidate {
        kind,
        app_id_prefix,
        identity,
    })
}

/// Why [`select_development_profile`] could not return a selection.
#[derive(Debug, thiserror::Error)]
pub enum SelectError {
    /// Every candidate was rejected — or none are installed. Provisioning
    /// may produce a valid one.
    #[error("{0}")]
    NoMatch(NoMatchingProfile),
    /// A profile directory, profile file, home-directory lookup or the
    /// `security` tool failed. Unlike [`Self::NoMatch`], retrying selection
    /// after provisioning cannot fix an I/O or tool error, so the caller
    /// must not provision on this variant.
    #[error("{0}")]
    Failed(eyre::Report),
}

/// Scan the Xcode profile directories and pick the installed development
/// profile satisfying `request`, paired with the keychain identity it
/// signs under.
///
/// Every rejected candidate is logged with its reason and carried in the
/// [`NoMatchingProfile`] — a malformed profile appears as a rejection,
/// never as "no profiles installed".
///
/// # Errors
/// - [`SelectError::NoMatch`]: no installed profile qualifies; every
///   candidate's rejection is listed.
/// - [`SelectError::Failed`]: a real I/O or tool failure — a profile
///   directory or file that exists but cannot be read, a home directory
///   that cannot be located, `security find-identity` failing.
pub async fn select_development_profile(
    host: &Host,
    request: &SigningRequest<'_>,
) -> Result<ProfileSelection, SelectError> {
    use smol::stream::StreamExt as _;

    let home = host.home_dir().ok_or_else(|| {
        SelectError::Failed(eyre::eyre!(
            "cannot locate the user's home directory to scan for provisioning profiles"
        ))
    })?;

    let mut candidates = Vec::new();
    for directory in profile_directories(home) {
        // A profile directory that does not exist is legitimate absence;
        // one that exists but cannot be read is a real I/O failure.
        match fs::metadata(&directory).await {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(SelectError::Failed(
                    eyre::Report::new(error)
                        .wrap_err(format!("failed to stat {}", directory.display())),
                ));
            }
        }
        let mut entries = fs::read_dir(&directory).await.map_err(|error| {
            SelectError::Failed(
                eyre::Report::new(error)
                    .wrap_err(format!("failed to read {}", directory.display())),
            )
        })?;
        while let Some(entry) = entries.next().await {
            let entry = entry.map_err(|error| {
                SelectError::Failed(eyre::Report::new(error).wrap_err(format!(
                    "failed to read an entry of {}",
                    directory.display()
                )))
            })?;
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) == Some("mobileprovision") {
                candidates.push(path);
            }
        }
    }
    candidates.sort();

    let now = SystemTime::now();
    let mut rejections = Vec::new();

    // The usable development-identity inventory is probed once per
    // selection and feeds candidate eligibility — nothing to check against
    // when no profiles exist at all.
    let identities = if candidates.is_empty() {
        Vec::new()
    } else {
        usable_development_identities(host).await?
    };

    let mut wildcard_match: Option<ProfileSelection> = None;
    for path in candidates {
        let bytes = fs::read(&path).await.map_err(|error| {
            SelectError::Failed(
                eyre::Report::new(error).wrap_err(format!("failed to read {}", path.display())),
            )
        })?;
        let data = match decode_mobileprovision(&bytes) {
            Ok(data) => data,
            Err(error) => {
                let rejection = ProfileRejection::Undecodable {
                    reason: format!("{error:#}"),
                };
                debug!("rejecting {}: {rejection}", path.display());
                rejections.push((path, rejection));
                continue;
            }
        };
        match evaluate_candidate(&data, request, &identities, now) {
            Ok(accepted) => {
                let selection = ProfileSelection {
                    path,
                    data,
                    app_id_prefix: accepted.app_id_prefix,
                    identity: accepted.identity,
                };
                // Exact-`application-identifier` matches win over wildcard
                // ones: a profile Xcode minted for this bundle id carries
                // the right `AppIDName`.
                if accepted.kind == AppIdMatch::Exact {
                    return Ok(selection);
                }
                wildcard_match.get_or_insert(selection);
            }
            Err(rejection) => {
                debug!("rejecting {}: {rejection}", path.display());
                rejections.push((path, rejection));
            }
        }
    }
    if let Some(selection) = wildcard_match {
        return Ok(selection);
    }
    Err(SelectError::NoMatch(NoMatchingProfile { rejections }))
}

/// The keychain's usable development identities via `security
/// find-identity` — a spawn or nonzero exit is a tool failure, never an
/// empty inventory, so a provisioning retry is not triggered by a broken
/// keychain probe.
async fn usable_development_identities(host: &Host) -> Result<Vec<(String, String)>, SelectError> {
    let output = host
        .output("security", ["find-identity", "-v", "-p", "codesigning"])
        .await
        .map_err(|error| {
            SelectError::Failed(eyre::eyre!(
                "failed to run `security find-identity`: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(SelectError::Failed(eyre::eyre!(
            "`security find-identity -v -p codesigning` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(crate::apple::toolchain::development_identities(
        &String::from_utf8_lossy(&output.stdout),
    ))
}

/// Apple's entitlement substitution (TN2415): an App-ID-shaped wildcard
/// grant `PREFIX.*` becomes `PREFIX.<bundle>` — the stem is the dotted
/// prefix (`TEAM.*`, `TEAM.shared.*`). Other wildcard-bearing grants that
/// are not App-ID-shaped, such as `applinks:*`, pass through untouched.
/// Applied recursively over arrays and dictionaries.
fn concretize_entitlement(value: &plist::Value, bundle_id: &str) -> plist::Value {
    match value {
        plist::Value::String(grant) => {
            let app_id_shaped = grant.strip_suffix(".*").is_some_and(|stem| {
                !stem.is_empty()
                    && stem
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            });
            if app_id_shaped {
                let stem = &grant[..grant.len() - 2];
                plist::Value::String(format!("{stem}.{bundle_id}"))
            } else {
                value.clone()
            }
        }
        plist::Value::Array(items) => plist::Value::Array(
            items
                .iter()
                .map(|item| concretize_entitlement(item, bundle_id))
                .collect(),
        ),
        plist::Value::Dictionary(dict) => plist::Value::Dictionary(
            dict.iter()
                .map(|(key, item)| (key.clone(), concretize_entitlement(item, bundle_id)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// The entitlement dictionary the `codesign` signature must claim for
/// `request` under the selected `profile`.
///
/// The profile's grant dictionary authorizes values; the signature itself
/// must carry concrete ones (TN2415): `application-identifier` is always
/// the fully qualified `{prefix}.{bundle_id}`, and a wildcard grant
/// (`PREFIX.*`) is rewritten to the value this app exercises
/// (`PREFIX.{bundle_id}`). Keys the app requested keep the requested value
/// — verified covered during selection — so no capability the app did not
/// declare is invented here.
///
/// # Errors
/// Returns an error when the profile carries no `Entitlements` dictionary.
pub fn signing_entitlements(
    profile: &plist::Dictionary,
    request: &SigningRequest<'_>,
    app_id_prefix: &str,
) -> eyre::Result<plist::Dictionary> {
    let Some(grants) = profile
        .get("Entitlements")
        .and_then(plist::Value::as_dictionary)
    else {
        bail!("the provisioning profile carries no Entitlements dictionary");
    };
    let mut signed = plist::Dictionary::new();
    for (key, grant) in grants {
        let value = if key == "application-identifier" {
            plist::Value::String(format!("{app_id_prefix}.{}", request.bundle_id))
        } else {
            concretize_entitlement(
                request.entitlements.get(key).unwrap_or(grant),
                request.bundle_id,
            )
        };
        signed.insert(key.clone(), value);
    }
    Ok(signed)
}

/// A provisioning failure reported by `xcodebuild`, mapped to the action the
/// user takes next. Each variant is one failure class Xcode reports.
#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    /// Xcode has no Apple ID signed in.
    #[error(
        "xcodebuild cannot provision a profile: no Apple ID is signed into Xcode. \
         Open Xcode > Settings > Accounts, sign in with your Apple ID, then retry."
    )]
    NoAppleIdSignedIn,
    /// The team cannot register another App ID — the free tier caps
    /// registered App IDs.
    #[error(
        "xcodebuild cannot provision a profile: the development team has reached the \
         maximum number of registered App IDs{list}. Remove an App ID under \
         developer.apple.com > Certificates, Identifiers & Profiles > Identifiers, then retry.",
        list = format_registered_app_ids(.app_ids)
    )]
    AppIdLimit {
        /// The App IDs Xcode reported as occupying the quota.
        app_ids: Vec<String>,
    },
    /// The destination device could not be registered to the team.
    #[error(
        "xcodebuild cannot provision a profile: device {udid} is not registered to the \
         development team. Paid teams register devices under developer.apple.com > \
         Certificates, Identifiers & Profiles > Devices; for a free team connect the \
         device to a Mac running Xcode once, then retry."
    )]
    DeviceNotRegistered {
        /// The device UDID Xcode could not register.
        udid: String,
    },
    /// The keychain holds no usable `Apple Development` signing certificate.
    #[error(
        "xcodebuild cannot provision a profile: no \"Apple Development\" signing \
         certificate with a private key exists in the keychain. In Xcode > Settings > \
         Accounts > Manage Certificates, create an Apple Development certificate, then retry."
    )]
    SigningCertificateMissing,
    /// xcodebuild could not resolve the destination specifier — the device is
    /// not visible to Xcode at all.
    #[error(
        "xcodebuild found no device matching '{destination}'.{detail} Check that \
         the device is connected, unlocked and trusted, then retry."
    )]
    DestinationUnavailable {
        /// The `-destination` specifier that matched nothing.
        destination: String,
        /// Extra detail from the output (e.g. a missing iOS platform), with a
        /// leading space when present.
        detail: String,
    },
    /// Any other failure; the message carries xcodebuild's error lines.
    #[error("xcodebuild failed to create a provisioning profile:{errors}")]
    Failed {
        /// The `error:` lines extracted from the output.
        errors: String,
    },
    /// `xcodebuild` could not be spawned at all.
    #[error("failed to run xcodebuild: {0}")]
    Spawn(String),
}

fn format_registered_app_ids(app_ids: &[String]) -> String {
    if app_ids.is_empty() {
        String::new()
    } else {
        format!(" — Xcode reports: {}", app_ids.join(", "))
    }
}

/// The throwaway Xcode project's `project.pbxproj`, rendered with the real
/// team's settings so `xcodebuild` provisions for the app that will sign.
#[derive(Template)]
#[template(path = "src/apple/provisioning/project.pbxproj.tpl", escape = "none")]
struct ProvisioningProjectTemplate<'a> {
    team: &'a str,
    bundle_id: &'a str,
    /// The `*_DEPLOYMENT_TARGET` build setting name, e.g.
    /// `IPHONEOS_DEPLOYMENT_TARGET`.
    deployment_setting: &'a str,
    deployment_target: &'a str,
    /// `SDKROOT`, e.g. `iphoneos`.
    sdkroot: &'a str,
    /// `TARGETED_DEVICE_FAMILY`, e.g. `1,2`.
    device_family: &'a str,
}

/// The platform-specific build settings the generated project needs — the
/// SDK, deployment-target setting name and targeted device family of the
/// request's `TargetPlatform`.
fn xcode_project_settings(
    platform: TargetPlatform,
) -> eyre::Result<(&'static str, &'static str, &'static str)> {
    let deployment_setting = platform
        .deployment_target_setting()
        .ok_or_else(|| eyre::eyre!("{platform:?} has no Apple deployment-target build setting"))?;
    let sdkroot = platform
        .sdk_name()
        .ok_or_else(|| eyre::eyre!("{platform:?} has no Apple SDK"))?;
    let device_family = platform
        .targeted_device_family()
        .ok_or_else(|| eyre::eyre!("{platform:?} has no targeted device family"))?;
    Ok((deployment_setting, sdkroot, device_family))
}

/// Write the minimal Xcode project used to drive `-allowProvisioningUpdates`:
/// one app target with `CODE_SIGN_STYLE = Automatic` bound to the app's
/// team, bundle identifier and entitlements.
async fn write_provisioning_project(
    staging_dir: &Path,
    request: &SigningRequest<'_>,
    deployment_target: &str,
) -> eyre::Result<PathBuf> {
    let (deployment_setting, sdkroot, device_family) = xcode_project_settings(request.platform)?;
    let project = ProvisioningProjectTemplate {
        team: request.team,
        bundle_id: request.bundle_id,
        deployment_setting,
        deployment_target,
        sdkroot,
        device_family,
    }
    .render()
    .wrap_err("failed to render the provisioning project template")?;

    let project_dir = staging_dir.join("Provision.xcodeproj");
    let sources_dir = staging_dir.join("Provision");
    fs::create_dir_all(&project_dir).await?;
    fs::create_dir_all(&sources_dir).await?;
    fs::write(project_dir.join("project.pbxproj"), project).await?;
    fs::write(sources_dir.join("main.swift"), "print(\"provisioning\")\n").await?;

    let mut entitlements = Vec::new();
    plist::Value::Dictionary(request.entitlements.clone())
        .to_writer_xml(&mut entitlements)
        .wrap_err("failed to serialize provisioning entitlements")?;
    fs::write(sources_dir.join("Provision.entitlements"), entitlements).await?;

    Ok(project_dir)
}

/// Extract the `error:` lines xcodebuild prints — the failure summary worth
/// showing the user.
fn xcodebuild_error_lines(output: &str) -> String {
    let errors: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("error:"))
        .collect();
    if errors.is_empty() {
        output
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        errors.join("\n")
    }
}

/// Bundle-identifier-shaped quoted tokens, used to report the App IDs
/// occupying a capped free team's quota.
fn quoted_identifiers(output: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    for line in output.lines() {
        let mut rest = line;
        while let Some(start) = rest.find('"') {
            let Some(end) = rest[start + 1..].find('"') else {
                break;
            };
            let token = &rest[start + 1..start + 1 + end];
            let looks_like_app_id = token.contains('.')
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            if looks_like_app_id {
                seen.insert(token.to_string());
            }
            rest = &rest[start + 1 + end + 1..];
        }
    }
    seen.into_iter().collect()
}

/// Map a failed `xcodebuild` run's combined output to the action it calls
/// for. Order matters: when several conditions hold (e.g. no Apple ID also
/// produces "No profiles ... were found"), the first is the actionable one.
fn map_provisioning_failure(
    output: &str,
    device_udid: Option<&str>,
    destination: &str,
) -> ProvisionError {
    if output.contains("No Accounts")
        || output.contains("no accounts registered with Xcode")
        || output.contains("There are no accounts registered")
    {
        return ProvisionError::NoAppleIdSignedIn;
    }
    if output.contains("maximum number of registered App IDs")
        || output.contains("maximum allowed number of App IDs")
    {
        return ProvisionError::AppIdLimit {
            app_ids: quoted_identifiers(output),
        };
    }
    if output.contains("No signing certificate")
        || output.contains("no \"Apple Development\" signing certificate")
    {
        return ProvisionError::SigningCertificateMissing;
    }
    if output.contains("doesn't include the currently selected device")
        || output.contains("does not include the currently selected device")
        || output.contains("not registered on the developer account")
        || output.contains("Failed to register the device")
        || output.contains("couldn't register the device")
    {
        return ProvisionError::DeviceNotRegistered {
            udid: device_udid.unwrap_or("<unknown>").to_string(),
        };
    }
    if output.contains("Unable to find a device matching") {
        let detail = output
            .lines()
            .find(|line| line.contains("is not installed"))
            .map_or_else(String::new, |line| format!(" {}", line.trim()));
        return ProvisionError::DestinationUnavailable {
            destination: destination.to_string(),
            detail,
        };
    }
    ProvisionError::Failed {
        errors: xcodebuild_error_lines(output),
    }
}

/// The `-destination` specifier the provisioning build targets: the request
/// platform's device family, bound to the device's UDID when the run is
/// device-bound and `generic` otherwise.
fn provisioning_destination(request: &SigningRequest<'_>) -> Result<String, ProvisionError> {
    let destination_name =
        request
            .platform
            .xcode_destination_name()
            .ok_or_else(|| ProvisionError::Failed {
                errors: format!("{:?} has no xcodebuild destination", request.platform),
            })?;
    Ok(request.device_udid.map_or_else(
        || format!("generic/platform={destination_name}"),
        |udid| format!("platform={destination_name},id={udid}"),
    ))
}

/// Drive `xcodebuild -allowProvisioningUpdates` against a generated throwaway project.
///
/// Xcode registers the destination device and installs a development
/// provisioning profile for `request`. The profile lands in
/// `~/Library/MobileDevice/Provisioning Profiles`, where
/// [`select_development_profile`] finds it.
///
/// # Errors
/// - [`ProvisionError`]: xcodebuild failed; the variant names the next action.
/// - `eyre`: the throwaway project could not be written.
pub async fn provision_via_xcodebuild(
    host: &Host,
    request: &SigningRequest<'_>,
    staging_dir: &Path,
    deployment_target: &str,
) -> Result<(), ProvisionError> {
    let project_dir = write_provisioning_project(staging_dir, request, deployment_target)
        .await
        .map_err(|error| ProvisionError::Failed {
            errors: format!("could not write the throwaway Xcode project: {error}"),
        })?;

    let destination = provisioning_destination(request)?;
    info!(
        "No installed development profile covers {team}.{bundle}; asking Xcode to \
         provision one ({destination})",
        team = request.team,
        bundle = request.bundle_id,
    );

    let output = host
        .output(
            "xcodebuild",
            [
                "-project".into(),
                project_dir.as_os_str().to_owned(),
                "-scheme".into(),
                "Provision".into(),
                "-destination".into(),
                destination.clone().into(),
                "-allowProvisioningUpdates".into(),
                "-allowProvisioningDeviceRegistration".into(),
                "-configuration".into(),
                "Debug".into(),
                "build".into(),
            ],
        )
        .await
        .map_err(|error| ProvisionError::Spawn(error.to_string()))?;

    if output.status.success() {
        info!("xcodebuild installed a development provisioning profile");
        return Ok(());
    }

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Err(map_provisioning_failure(
        &combined,
        request.device_udid,
        &destination,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAM: &str = "TESTTEAM42";
    const BUNDLE: &str = "dev.waterui.fixture";
    const UDID: &str = "00008140-0000ABCD00112233";
    /// DER bytes standing in for the certificate a profile's
    /// `DeveloperCertificates` names — the usable identity's SHA-1 is
    /// computed over them.
    const DEV_CERT: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];
    /// A certificate no keychain identity matches.
    const STALE_CERT: &[u8] = &[0x01, 0x02, 0x03, 0x04];

    /// The usable identity inventory `security find-identity` would report
    /// for the fixture certificate.
    fn usable_identities() -> Vec<(String, String)> {
        vec![(
            crate::apple::toolchain::certificate_sha1_hex(DEV_CERT),
            "Apple Development: test@example.invalid".to_string(),
        )]
    }

    fn evaluate(
        data: &plist::Dictionary,
        request: &SigningRequest<'_>,
    ) -> Result<AcceptedCandidate, ProfileRejection> {
        evaluate_candidate(data, request, &usable_identities(), SystemTime::now())
    }

    /// A decoded-profile-shaped dictionary: `ApplicationIdentifierPrefix`
    /// defaults to the team (the common case); the test covering a distinct
    /// prefix overrides it.
    fn profile_fixture(
        team: &str,
        app_id: &str,
        devices: Option<Vec<&str>>,
        expired: bool,
    ) -> plist::Dictionary {
        let mut entitlements = plist::Dictionary::new();
        entitlements.insert(
            "application-identifier".to_string(),
            plist::Value::String(app_id.to_string()),
        );
        entitlements.insert(
            "com.apple.developer.team-identifier".to_string(),
            plist::Value::String(team.to_string()),
        );
        entitlements.insert("get-task-allow".to_string(), plist::Value::Boolean(true));
        entitlements.insert(
            "keychain-access-groups".to_string(),
            plist::Value::Array(vec![plist::Value::String(format!("{team}.*"))]),
        );

        let mut dict = plist::Dictionary::new();
        dict.insert("Name".to_string(), plist::Value::String("dev".to_string()));
        dict.insert(
            "UUID".to_string(),
            plist::Value::String("00000000-0000-0000-0000-000000000001".to_string()),
        );
        dict.insert(
            "TeamIdentifier".to_string(),
            plist::Value::Array(vec![plist::Value::String(team.to_string())]),
        );
        dict.insert(
            "ApplicationIdentifierPrefix".to_string(),
            plist::Value::Array(vec![plist::Value::String(team.to_string())]),
        );
        dict.insert(
            "DeveloperCertificates".to_string(),
            plist::Value::Array(vec![plist::Value::Data(DEV_CERT.to_vec())]),
        );
        dict.insert(
            "Entitlements".to_string(),
            plist::Value::Dictionary(entitlements),
        );
        let expiration = if expired {
            SystemTime::now() - std::time::Duration::from_secs(3600)
        } else {
            SystemTime::now() + std::time::Duration::from_secs(3600)
        };
        dict.insert(
            "ExpirationDate".to_string(),
            plist::Value::Date(expiration.into()),
        );
        if let Some(devices) = devices {
            dict.insert(
                "ProvisionedDevices".to_string(),
                plist::Value::Array(
                    devices
                        .into_iter()
                        .map(|udid| plist::Value::String(udid.to_string()))
                        .collect(),
                ),
            );
        }
        dict
    }

    fn request(entitlements: &plist::Dictionary) -> SigningRequest<'_> {
        SigningRequest {
            team: TEAM,
            bundle_id: BUNDLE,
            device_udid: Some(UDID),
            entitlements,
            platform: TargetPlatform::IOS,
        }
    }

    #[test]
    fn accepts_an_exact_profile_covering_the_device() {
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        let accepted = evaluate(&profile, &request(&plist::Dictionary::new()))
            .expect("the fixture profile satisfies the request");
        assert_eq!(accepted.kind, AppIdMatch::Exact);
        assert_eq!(accepted.app_id_prefix, TEAM);
        assert_eq!(
            accepted.identity,
            crate::apple::toolchain::certificate_sha1_hex(DEV_CERT)
        );
    }

    #[test]
    fn rejects_an_expired_profile() {
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), true);
        assert_eq!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::Expired)
        );
    }

    #[test]
    fn rejects_a_profile_without_expiration() {
        let mut profile =
            profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        profile.remove("ExpirationDate");
        assert_eq!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::InvalidMetadata {
                key: "ExpirationDate"
            })
        );
    }

    #[test]
    fn rejects_a_mistyped_expiration() {
        let mut profile =
            profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        profile.insert(
            "ExpirationDate".to_string(),
            plist::Value::String("next year".to_string()),
        );
        assert_eq!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::InvalidMetadata {
                key: "ExpirationDate"
            })
        );
    }

    #[test]
    fn rejects_a_profile_without_app_id_prefixes() {
        let mut profile =
            profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        profile.remove("ApplicationIdentifierPrefix");
        assert_eq!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::InvalidMetadata {
                key: "ApplicationIdentifierPrefix"
            })
        );
    }

    #[test]
    fn accepts_a_profile_whose_app_id_prefix_differs_from_the_team() {
        // TN2415: the App ID prefix is often but not always the team ID.
        // The team check reads TeamIdentifier; the app id reads the prefix.
        let mut profile = profile_fixture(
            TEAM,
            &format!("OTHERPFX99.{BUNDLE}"),
            Some(vec![UDID]),
            false,
        );
        profile.insert(
            "ApplicationIdentifierPrefix".to_string(),
            plist::Value::Array(vec![plist::Value::String("OTHERPFX99".to_string())]),
        );
        let accepted = evaluate(&profile, &request(&plist::Dictionary::new()))
            .expect("a valid distinct-prefix profile satisfies the request");
        assert_eq!(accepted.app_id_prefix, "OTHERPFX99");
    }

    #[test]
    fn rejects_a_prefix_match_under_a_undeclared_prefix() {
        // The app-id string uses OTHERPFX99, but the profile does not
        // declare it in ApplicationIdentifierPrefix.
        let profile = profile_fixture(
            TEAM,
            &format!("OTHERPFX99.{BUNDLE}"),
            Some(vec![UDID]),
            false,
        );
        assert!(matches!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::AppIdMismatch { .. })
        ));
    }

    #[test]
    fn rejects_a_profile_for_another_team() {
        let profile = profile_fixture("OTHERTEAM9", "OTHERTEAM9.*", Some(vec![UDID]), false);
        assert_eq!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::WrongTeam {
                profile_teams: vec!["OTHERTEAM9".to_string()]
            })
        );
    }

    #[test]
    fn accepts_a_team_wildcard_app_id() {
        let profile = profile_fixture(TEAM, &format!("{TEAM}.*"), Some(vec![UDID]), false);
        let accepted = evaluate(&profile, &request(&plist::Dictionary::new()))
            .expect("a team wildcard covers the bundle id");
        assert_eq!(accepted.kind, AppIdMatch::Wildcard);
    }

    #[test]
    fn rejects_an_app_id_for_a_sibling_bundle() {
        let profile = profile_fixture(
            TEAM,
            &format!("{TEAM}.dev.waterui.other"),
            Some(vec![UDID]),
            false,
        );
        assert!(matches!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::AppIdMismatch { .. })
        ));
    }

    #[test]
    fn accepts_a_prefix_wildcard_app_id() {
        let profile = profile_fixture(
            TEAM,
            &format!("{TEAM}.dev.waterui.*"),
            Some(vec![UDID]),
            false,
        );
        assert!(evaluate(&profile, &request(&plist::Dictionary::new())).is_ok());
    }

    #[test]
    fn rejects_when_the_device_is_not_provisioned() {
        let profile = profile_fixture(
            TEAM,
            &format!("{TEAM}.{BUNDLE}"),
            Some(vec!["00008140-9999OTHERDEVICE"]),
            false,
        );
        assert_eq!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::DeviceNotProvisioned {
                udid: UDID.to_string()
            })
        );
    }

    #[test]
    fn accepts_provisions_all_devices() {
        let mut profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), None, false);
        profile.insert(
            "ProvisionsAllDevices".to_string(),
            plist::Value::Boolean(true),
        );
        assert!(evaluate(&profile, &request(&plist::Dictionary::new())).is_ok());
    }

    #[test]
    fn rejects_a_deviceless_profile_for_generic_packaging() {
        // No destination UDID: a development profile still must provision
        // some device — a profile with no device list at all is rejected.
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![]), false);
        let empty = plist::Dictionary::new();
        let mut request = request(&empty);
        request.device_udid = None;
        assert_eq!(
            evaluate(&profile, &request),
            Err(ProfileRejection::MissingDeviceList)
        );
    }

    #[test]
    fn accepts_a_device_bound_profile_for_generic_packaging() {
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        let empty = plist::Dictionary::new();
        let mut request = request(&empty);
        request.device_udid = None;
        assert!(evaluate(&profile, &request).is_ok());
    }

    #[test]
    fn rejects_a_missing_entitlement() {
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        let mut entitlements = plist::Dictionary::new();
        entitlements.insert(
            "com.apple.developer.networking.multipath".to_string(),
            plist::Value::Boolean(true),
        );
        assert_eq!(
            evaluate(&profile, &request(&entitlements)),
            Err(ProfileRejection::MissingEntitlements {
                keys: vec!["com.apple.developer.networking.multipath".to_string()]
            })
        );
    }

    /// A crate-declared environment entitlement reaches the profile check
    /// like any other: a development profile without `aps-environment` is
    /// rejected as missing it, one granting the development environment is
    /// accepted.
    #[test]
    fn a_declared_aps_environment_requires_a_profile_granting_it() {
        let mut profile =
            profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        let mut entitlements = plist::Dictionary::new();
        entitlements.insert(
            "aps-environment".to_string(),
            plist::Value::String("development".to_string()),
        );
        assert_eq!(
            evaluate(&profile, &request(&entitlements)),
            Err(ProfileRejection::MissingEntitlements {
                keys: vec!["aps-environment".to_string()]
            })
        );
        let Some(plist::Value::Dictionary(grants)) = profile.get_mut("Entitlements") else {
            panic!("the fixture profile carries Entitlements");
        };
        grants.insert(
            "aps-environment".to_string(),
            plist::Value::String("development".to_string()),
        );
        assert!(evaluate(&profile, &request(&entitlements)).is_ok());
    }

    #[test]
    fn grants_a_wildcarded_entitlement_value() {
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        let mut entitlements = plist::Dictionary::new();
        entitlements.insert(
            "keychain-access-groups".to_string(),
            plist::Value::Array(vec![plist::Value::String(format!("{TEAM}.{BUNDLE}"))]),
        );
        assert!(evaluate(&profile, &request(&entitlements)).is_ok());
    }

    #[test]
    fn rejects_a_profile_whose_certificate_is_unusable() {
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        // No usable development identities at all.
        assert_eq!(
            evaluate_candidate(
                &profile,
                &request(&plist::Dictionary::new()),
                &[],
                SystemTime::now()
            ),
            Err(ProfileRejection::NoUsableCertificate)
        );
    }

    #[test]
    fn rejects_a_distribution_profile() {
        let mut profile =
            profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        profile
            .get_mut("Entitlements")
            .and_then(plist::Value::as_dictionary_mut)
            .expect("entitlements")
            .insert("get-task-allow".to_string(), plist::Value::Boolean(false));
        assert_eq!(
            evaluate(&profile, &request(&plist::Dictionary::new())),
            Err(ProfileRejection::NotDevelopment)
        );
    }

    /// Serialize a fixture dictionary the way a `.mobileprovision` on disk
    /// is stored: plist XML inside a CMS `SignedData` envelope.
    fn profile_file_bytes(profile: &plist::Dictionary) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::Value::Dictionary(profile.clone())
            .to_writer_xml(&mut payload)
            .expect("fixture serializes");
        cms_envelope(&payload)
    }

    /// A `Host` over a scratch machine whose `security` reports the given
    /// find-identity output.
    fn selection_machine() -> crate::toolchain::testing::TestMachine {
        crate::toolchain::testing::TestMachine::new()
    }

    fn profiles_dir(machine: &crate::toolchain::testing::TestMachine) -> PathBuf {
        machine
            .home()
            .join("Library/MobileDevice/Provisioning Profiles")
    }

    fn install_security(machine: &crate::toolchain::testing::TestMachine, identities: &str) {
        machine.install("security");
        machine.respond("SECURITY_FIND_IDENTITY", identities);
    }

    fn write_profile(dir: &Path, name: &str, profile: &plist::Dictionary) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create profile dir");
        let path = dir.join(name);
        std::fs::write(&path, profile_file_bytes(profile)).expect("write profile");
        path
    }

    #[test]
    fn selection_pairs_profile_and_identity() {
        let machine = selection_machine();
        let hash = crate::apple::toolchain::certificate_sha1_hex(DEV_CERT);
        install_security(
            &machine,
            &format!(
                "     1) {hash} \"Apple Development: test@example.invalid\"\n     1 valid identities found\n"
            ),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        let profile = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        let profile_path = write_profile(&profiles_dir(&machine), "aaa.mobileprovision", &profile);

        let selection = smol::block_on(select_development_profile(
            &host,
            &request(&plist::Dictionary::new()),
        ))
        .expect("the installed profile is selected");
        assert_eq!(selection.path, profile_path);
        assert_eq!(selection.identity, hash);
        assert_eq!(selection.app_id_prefix, TEAM);
    }

    #[test]
    fn selection_prefers_the_later_valid_identity_over_a_stale_exact() {
        let machine = selection_machine();
        let hash = crate::apple::toolchain::certificate_sha1_hex(DEV_CERT);
        install_security(
            &machine,
            &format!(
                "     1) {hash} \"Apple Development: test@example.invalid\"\n     1 valid identities found\n"
            ),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        let dir = profiles_dir(&machine);

        // Sorts first: an exact App ID match issued for a certificate whose
        // private key the keychain lacks.
        let mut stale = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        stale.insert(
            "DeveloperCertificates".to_string(),
            plist::Value::Array(vec![plist::Value::Data(STALE_CERT.to_vec())]),
        );
        write_profile(&dir, "aaa.mobileprovision", &stale);

        // Sorts later: a wildcard match with a usable identity.
        let usable = profile_fixture(TEAM, &format!("{TEAM}.*"), Some(vec![UDID]), false);
        let usable_path = write_profile(&dir, "zzz.mobileprovision", &usable);

        let selection = smol::block_on(select_development_profile(
            &host,
            &request(&plist::Dictionary::new()),
        ))
        .expect("the wildcard candidate with a usable identity is selected");
        assert_eq!(selection.path, usable_path);
        assert_eq!(selection.identity, hash);
    }

    #[test]
    fn absent_profile_directories_are_a_no_match() {
        let machine = selection_machine();
        let host = machine.host(Vec::<(String, String)>::new());
        match smol::block_on(select_development_profile(
            &host,
            &request(&plist::Dictionary::new()),
        )) {
            Err(SelectError::NoMatch(no_match)) => assert_eq!(no_match.rejections, []),
            other => panic!("expected NoMatch for absent directories, got {other:?}"),
        }
    }

    #[test]
    fn malformed_profile_is_a_diagnosed_rejection() {
        let machine = selection_machine();
        install_security(&machine, "");
        let host = machine.host(Vec::<(String, String)>::new());
        let dir = profiles_dir(&machine);
        std::fs::create_dir_all(&dir).expect("create profile dir");
        std::fs::write(dir.join("junk.mobileprovision"), b"not a cms blob")
            .expect("write junk profile");

        match smol::block_on(select_development_profile(
            &host,
            &request(&plist::Dictionary::new()),
        )) {
            Err(SelectError::NoMatch(no_match)) => {
                assert_eq!(no_match.rejections.len(), 1);
                assert!(matches!(
                    no_match.rejections[0].1,
                    ProfileRejection::Undecodable { .. }
                ));
            }
            other => panic!("expected a diagnosed rejection, got {other:?}"),
        }
    }

    /// An unreadable profile directory is an I/O failure, not "no profiles
    /// installed" — the error must not invoke provisioning.
    #[cfg(unix)]
    #[test]
    fn unreadable_profile_directory_is_a_hard_error() {
        use std::os::unix::fs::PermissionsExt as _;

        let machine = selection_machine();
        let host = machine.host(Vec::<(String, String)>::new());
        let dir = profiles_dir(&machine);
        std::fs::create_dir_all(&dir).expect("create profile dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).expect("deny read");

        let result = smol::block_on(select_development_profile(
            &host,
            &request(&plist::Dictionary::new()),
        ));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("restore permissions for cleanup");
        assert!(
            matches!(result, Err(SelectError::Failed(_))),
            "an I/O failure must surface as Failed, never NoMatch: {result:?}"
        );
    }

    #[test]
    fn home_lookup_failure_is_a_hard_error() {
        // A Host with no HOME/USERPROFILE has no home directory to scan.
        let host = Host::new(Vec::<PathBuf>::new(), Vec::<(String, String)>::new());
        match smol::block_on(select_development_profile(
            &host,
            &request(&plist::Dictionary::new()),
        )) {
            Err(SelectError::Failed(_)) => {}
            other => panic!("expected Failed for a missing home, got {other:?}"),
        }
    }

    /// Wrap a plist dictionary in a CMS `SignedData` envelope the way a real
    /// `.mobileprovision` is wrapped, without any key material.
    fn cms_envelope(payload: &[u8]) -> Vec<u8> {
        use cms::content_info::CmsVersion;
        use cms::signed_data::{EncapsulatedContentInfo, SignerInfos};
        use der::Encode;
        use der::asn1::{ObjectIdentifier, SetOfVec};

        let signed = SignedData {
            version: CmsVersion::V1,
            digest_algorithms: SetOfVec::default(),
            encap_content_info: EncapsulatedContentInfo {
                econtent_type: ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"),
                econtent: Some(
                    der::Any::new(der::Tag::OctetString, payload).expect("octet string"),
                ),
            },
            certificates: None,
            crls: None,
            signer_infos: SignerInfos::from(SetOfVec::default()),
        };
        ContentInfo {
            content_type: ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2"),
            content: der::Any::encode_from(&signed).expect("signed data encodes"),
        }
        .to_der()
        .expect("content info encodes")
    }

    #[test]
    fn decodes_a_cms_wrapped_profile() {
        let fixture = profile_fixture(TEAM, &format!("{TEAM}.{BUNDLE}"), Some(vec![UDID]), false);
        let mut payload = Vec::new();
        plist::Value::Dictionary(fixture.clone())
            .to_writer_xml(&mut payload)
            .expect("fixture serializes");
        let der = cms_envelope(&payload);
        let decoded = decode_mobileprovision(&der).expect("decodes the envelope");
        assert_eq!(decoded, fixture);
    }

    #[test]
    fn rejects_a_non_cms_blob() {
        assert!(decode_mobileprovision(b"not der").is_err());
        assert!(decode_mobileprovision(&[0x30, 0x03, 0x02, 0x01, 0x05]).is_err());
    }

    // Fixture: real `xcodebuild` output captured on a clean VM (no Xcode
    // accounts) — `error: No Accounts` is the actionable line.
    const XCODEBUILD_NO_ACCOUNT: &str = include_str!("testdata/xcodebuild_no_account.txt");

    // Fixture: real `xcodebuild` output captured when the destination device
    // specifier matches nothing.
    const XCODEBUILD_NO_DEVICE: &str = include_str!("testdata/xcodebuild_no_device.txt");

    #[test]
    fn maps_no_apple_id_output() {
        assert!(matches!(
            map_provisioning_failure(XCODEBUILD_NO_ACCOUNT, Some(UDID), "platform=iOS,id=x"),
            ProvisionError::NoAppleIdSignedIn
        ));
    }

    #[test]
    fn maps_unknown_device_output() {
        assert!(matches!(
            map_provisioning_failure(XCODEBUILD_NO_DEVICE, Some(UDID), "platform=iOS,id=x"),
            ProvisionError::DestinationUnavailable { .. }
        ));
    }

    #[test]
    fn maps_app_id_limit_output() {
        let output = concat!(
            "error: Cannot create a iOS App Development provisioning profile for ",
            "\"dev.waterui.fixture\". Your development team, \"Fixture Team\", has reached the ",
            "maximum number of registered App IDs. (in target 'Provision' from project 'Provision')\n",
            "** BUILD FAILED **\n"
        );
        match map_provisioning_failure(output, Some(UDID), "generic/platform=iOS") {
            ProvisionError::AppIdLimit { app_ids } => {
                assert_eq!(app_ids, vec!["dev.waterui.fixture".to_string()]);
            }
            other => panic!("expected AppIdLimit, got {other:?}"),
        }
    }

    #[test]
    fn maps_missing_certificate_output() {
        let output = concat!(
            "error: No signing certificate \"iOS Development\" found: No \"iOS Development\" ",
            "signing certificate matching team ID \"TESTTEAM42\" with a private key was found. ",
            "(in target 'Provision' from project 'Provision')\n** BUILD FAILED **\n"
        );
        assert!(matches!(
            map_provisioning_failure(output, Some(UDID), "generic/platform=iOS"),
            ProvisionError::SigningCertificateMissing
        ));
    }

    #[test]
    fn maps_unregistered_device_output() {
        let output = concat!(
            "error: Provisioning profile \"iOS Team Provisioning Profile: dev.waterui.fixture\" ",
            "doesn't include the currently selected device \"Devin's iPhone\" ",
            "(identifier 00008140-0000ABCD00112233). (in target 'Provision' from project 'Provision')\n",
            "** BUILD FAILED **\n"
        );
        match map_provisioning_failure(output, Some(UDID), "platform=iOS,id=x") {
            ProvisionError::DeviceNotRegistered { udid } => assert_eq!(udid, UDID),
            other => panic!("expected DeviceNotRegistered, got {other:?}"),
        }
    }

    #[test]
    fn maps_unrecognized_output_to_failure_lines() {
        let output = "note: something\nerror: unprecedented failure\n** BUILD FAILED **\n";
        match map_provisioning_failure(output, Some(UDID), "generic/platform=iOS") {
            ProvisionError::Failed { errors } => {
                assert!(errors.contains("unprecedented failure"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn maps_the_concrete_device_registration_failure() {
        let output = concat!(
            "error: Failed to register the device 'Devin's iPhone' with the developer account.\n",
            "** BUILD FAILED **\n"
        );
        match map_provisioning_failure(output, Some(UDID), "platform=iOS,id=x") {
            ProvisionError::DeviceNotRegistered { udid } => assert_eq!(udid, UDID),
            other => panic!("expected DeviceNotRegistered, got {other:?}"),
        }
    }

    #[test]
    fn generic_registration_failure_is_not_a_device_error() {
        // Registering a bundle identifier is a different failure class —
        // the device matcher must not swallow a bare "Failed to register".
        let output = concat!(
            "error: Failed to register bundle identifier. The app identifier ",
            "\"dev.waterui.fixture\" cannot be registered to your development team. ",
            "(in target 'Provision' from project 'Provision')\n** BUILD FAILED **\n"
        );
        match map_provisioning_failure(output, Some(UDID), "generic/platform=iOS") {
            ProvisionError::Failed { errors } => {
                assert!(errors.contains("Failed to register bundle identifier"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Manual verification on a machine with real Xcode installed: generate
    /// the actual throwaway project and compile it with signing disabled —
    /// the project is well-formed regardless of provisioning accounts.
    #[test]
    #[ignore = "requires a real Xcode toolchain"]
    fn generated_project_compiles_with_signing_disabled() {
        let machine = selection_machine();
        let host = Host::current();
        let empty = plist::Dictionary::new();
        let project_dir = smol::block_on(write_provisioning_project(
            &machine.home(),
            &request(&empty),
            "17.0",
        ))
        .expect("the generated project is written");
        let output = smol::block_on(host.output(
            "xcodebuild",
            [
                "-project".into(),
                project_dir.as_os_str().to_owned(),
                "-scheme".into(),
                "Provision".into(),
                "-configuration".into(),
                "Debug".into(),
                "build".into(),
                "CODE_SIGNING_ALLOWED=NO".into(),
            ],
        ))
        .expect("xcodebuild runs");
        assert!(
            output.status.success(),
            "the generated project must compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn renders_the_throwaway_project() {
        let rendered = ProvisioningProjectTemplate {
            team: TEAM,
            bundle_id: BUNDLE,
            deployment_setting: "IPHONEOS_DEPLOYMENT_TARGET",
            deployment_target: "17.0",
            sdkroot: "iphoneos",
            device_family: "1,2",
        }
        .render()
        .expect("template renders");
        assert!(rendered.contains(&format!("DEVELOPMENT_TEAM = {TEAM};")));
        assert!(rendered.contains(&format!("PRODUCT_BUNDLE_IDENTIFIER = {BUNDLE};")));
        assert!(rendered.contains("CODE_SIGN_STYLE = Automatic;"));
        // The source group must carry `path = Provision` — the files are
        // written under Provision/, not at the project root.
        assert!(rendered.contains("path = Provision;"));
        assert!(rendered.contains("IPHONEOS_DEPLOYMENT_TARGET = 17.0;"));
        assert!(rendered.contains("SDKROOT = iphoneos;"));
        assert!(rendered.contains("TARGETED_DEVICE_FAMILY = \"1,2\";"));
    }

    #[test]
    fn project_settings_follow_the_target_platform() {
        assert_eq!(
            xcode_project_settings(TargetPlatform::IOS).expect("iOS settings"),
            ("IPHONEOS_DEPLOYMENT_TARGET", "iphoneos", "1,2")
        );
        assert_eq!(
            xcode_project_settings(TargetPlatform::TvOS).expect("tvOS settings"),
            ("TVOS_DEPLOYMENT_TARGET", "appletvos", "3")
        );
        assert_eq!(
            xcode_project_settings(TargetPlatform::WatchOS).expect("watchOS settings"),
            ("WATCHOS_DEPLOYMENT_TARGET", "watchos", "4")
        );
        assert_eq!(
            xcode_project_settings(TargetPlatform::VisionOS).expect("visionOS settings"),
            ("XROS_DEPLOYMENT_TARGET", "xros", "7")
        );
        assert!(xcode_project_settings(TargetPlatform::Linux).is_err());
    }

    #[test]
    fn provisioning_destination_follows_the_target_platform() {
        let empty = plist::Dictionary::new();
        let mut request = request(&empty);
        request.platform = TargetPlatform::TvOS;
        assert_eq!(
            provisioning_destination(&request).expect("tvOS destination"),
            format!("platform=tvOS,id={UDID}")
        );
        request.device_udid = None;
        assert_eq!(
            provisioning_destination(&request).expect("generic tvOS destination"),
            "generic/platform=tvOS"
        );
    }

    #[test]
    fn signing_entitlements_are_concrete_under_a_wildcard_profile() {
        // TN2415: the signed application-identifier is always fully
        // qualified; wildcard grants are rewritten to the app's value.
        let profile = profile_fixture(TEAM, &format!("{TEAM}.*"), Some(vec![UDID]), false);
        let signed = signing_entitlements(&profile, &request(&plist::Dictionary::new()), TEAM)
            .expect("entitlements concretize");
        assert_eq!(
            signed.get("application-identifier"),
            Some(&plist::Value::String(format!("{TEAM}.{BUNDLE}")))
        );
        assert_eq!(
            signed.get("keychain-access-groups"),
            Some(&plist::Value::Array(vec![plist::Value::String(format!(
                "{TEAM}.{BUNDLE}"
            ))]))
        );
        assert_eq!(
            signed.get("get-task-allow"),
            Some(&plist::Value::Boolean(true))
        );
        assert_eq!(
            signed.get("com.apple.developer.team-identifier"),
            Some(&plist::Value::String(TEAM.to_string()))
        );
        assert!(
            !signed
                .iter()
                .any(|(_, v)| v.as_string().is_some_and(|s| s.contains('*'))),
            "no wildcard value may reach codesign"
        );
    }

    #[test]
    fn signing_entitlements_preserve_requested_values() {
        // A requested value already covered by the grant is used verbatim.
        let profile = profile_fixture(TEAM, &format!("{TEAM}.*"), Some(vec![UDID]), false);
        let mut requested = plist::Dictionary::new();
        requested.insert(
            "keychain-access-groups".to_string(),
            plist::Value::Array(vec![plist::Value::String(format!("{TEAM}.dev.shared"))]),
        );
        let signed = signing_entitlements(&profile, &request(&requested), TEAM)
            .expect("entitlements concretize");
        assert_eq!(
            signed.get("keychain-access-groups"),
            Some(&plist::Value::Array(vec![plist::Value::String(format!(
                "{TEAM}.dev.shared"
            ))]))
        );
    }

    #[test]
    fn signing_entitlements_preserve_non_app_id_wildcards() {
        // Only App-ID-shaped `PREFIX.*` grants are substituted; a different
        // wildcard such as `applinks:*` reaches codesign verbatim.
        let mut profile = profile_fixture(TEAM, &format!("{TEAM}.*"), Some(vec![UDID]), false);
        profile
            .get_mut("Entitlements")
            .and_then(plist::Value::as_dictionary_mut)
            .expect("entitlements")
            .insert(
                "com.apple.developer.associated-domains".to_string(),
                plist::Value::Array(vec![plist::Value::String("applinks:*".to_string())]),
            );
        let signed = signing_entitlements(&profile, &request(&plist::Dictionary::new()), TEAM)
            .expect("entitlements concretize");
        assert_eq!(
            signed.get("com.apple.developer.associated-domains"),
            Some(&plist::Value::Array(vec![plist::Value::String(
                "applinks:*".to_string()
            )]))
        );
        // The App-ID-shaped wildcard is still concretized.
        assert_eq!(
            signed.get("keychain-access-groups"),
            Some(&plist::Value::Array(vec![plist::Value::String(format!(
                "{TEAM}.{BUNDLE}"
            ))]))
        );
    }
}
