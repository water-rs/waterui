//! Facts the CLI derives from Cargo and the Rust toolchain — `cargo
//! metadata`, `cargo tree`, the rustup toolchain a directory selects and a
//! target's standard-library directory.
//!
//! Each fact is a pure function of its inputs: the manifests and lockfile
//! of the resolve it came from, the Cargo configuration that applies, the
//! project's `Water.toml`/`Water.lock`, the toolchain and the CLI itself.
//! [`Facts`] answers a question at most once per invocation and reuses an
//! answer across invocations only through an on-disk record that carries
//! the observed state of every one of those inputs: a later ask observes
//! them again — reading and hashing files, never spawning a process — and
//! replays the record only when the observation is identical. Any
//! difference resolves the fact again.
//!
//! The toolchain identity is observed without asking rustup or rustc: the
//! `cargo`/`rustc`/`rustup` executables the host's `PATH` resolves, every
//! `rust-toolchain`/`rust-toolchain.toml` from the working directory up,
//! rustup's `settings.toml` (default toolchain and directory overrides),
//! the toolchain-related environment, and the identity of every toolchain
//! installed under `$RUSTUP_HOME/toolchains` (plus a toolchain file's
//! custom `path`): its `rustc` and `cargo` stamps and the build-hashed file
//! name of its `rustc_driver` library. Whatever rustup would select is in
//! that set, so a toolchain install, update, removal or re-selection
//! changes the observation.
//!
//! A record is written only when every file it observed was last modified
//! before the resolve began (by more than [`MTIME_SLACK`], the coarsest
//! common filesystem timestamp granularity) and the toolchain observation
//! did not change across the resolve, so an input edited while Cargo was
//! reading it can never be recorded under its new contents.
//!
//! An unreadable record is an error, never a silent recompute: records are
//! written atomically (staged and renamed), live in a directory named by
//! [`RECORD_FORMAT`], and are replaced unread whenever the CLI build that
//! wrote them differs, so a record this CLI cannot parse means a
//! write/read asymmetry or outside tampering — recomputing over it would
//! hide that bug as a cache that never hits.

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use cargo_metadata::{Metadata, MetadataCommand};
use eyre::WrapErr as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use target_lexicon::Triple;

use crate::toolchain::Host;

/// How far before a resolve every observed input must have been modified
/// for its record to be written: two seconds, the timestamp granularity of
/// FAT and the coarsest of the filesystems a project can live on.
const MTIME_SLACK: Duration = Duration::from_secs(2);

/// The version of the record layout — the [`RecordHeader`] line, and
/// [`CliIdentity`] inside it, that every CLI build must be able to read
/// before it decides a record is another build's. Records live in a
/// directory named by it, so a change to that shape bumps it and never
/// meets the old layout.
const RECORD_FORMAT: u32 = 1;

/// The environment variables whose values reach Cargo's or rustup's
/// decisions: every `CARGO*`, `RUSTUP*` and `RUSTC*` variable, and the
/// rustflags families.
const ENV_PREFIXES: [&str; 5] = ["CARGO", "RUSTUP", "RUSTC", "RUSTFLAGS", "RUSTDOCFLAGS"];

/// One question the CLI asks Cargo or the toolchain.
pub trait Fact: Send + Sync {
    /// The record family, part of the record's file name.
    const KIND: &'static str;
    /// Everything that distinguishes this question from another of the
    /// same kind — arguments, working directory, environment overrides.
    type Question: Serialize + DeserializeOwned + PartialEq + Send + Sync + 'static;
    /// The answer.
    type Value: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// This question in its persisted form.
    fn question(&self) -> Self::Question;

    /// The directory the resolving process runs in: rustup selects the
    /// toolchain from it and Cargo discovers its configuration from it.
    fn cwd(&self) -> &Path;

    /// Ask Cargo or the toolchain.
    fn resolve(&self, host: &Host) -> impl Future<Output = eyre::Result<Self::Value>> + Send;

    /// The manifests and lockfiles `value` was resolved from — empty for a
    /// fact that reads none. Their workspace roots, those roots' `[patch]`
    /// targets and member globs are observed alongside them.
    ///
    /// # Errors
    /// Returns an error when `value` cannot be interpreted.
    fn seeds(&self, value: &Self::Value) -> eyre::Result<BTreeSet<PathBuf>>;
}

/// The CLI build that wrote a record: its version and its executable's
/// stamp, so a rebuilt binary of the same version never replays another
/// build's records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliIdentity {
    version: String,
    executable: Stamp,
}

impl CliIdentity {
    fn current() -> eyre::Result<Self> {
        let executable = Host::current_exe().wrap_err("cannot locate the running CLI")?;
        Ok(Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            executable: Stamp::of(&executable)?
                .ok_or_else(|| eyre::eyre!("the running CLI {} is gone", executable.display()))?,
        })
    }

    /// An identity with the given version and the running executable's
    /// stamp — a test's stand-in for a released CLI of another version.
    #[cfg(test)]
    pub(crate) fn with_version(version: &str) -> Self {
        let mut identity = Self::current().expect("the test binary has an identity");
        version.clone_into(&mut identity.version);
        identity
    }
}

/// A file's identity by metadata: path, length, modification time and, on
/// Unix, inode — replaced binaries change at least one of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    path: PathBuf,
    len: u64,
    modified_ns: u128,
    inode: u64,
}

impl Stamp {
    /// `path`'s stamp, following symlinks; `None` when nothing is there.
    fn of(path: &Path) -> io::Result<Option<Self>> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&metadata);
        #[cfg(not(unix))]
        let inode = 0;
        Ok(Some(Self {
            path: path.to_path_buf(),
            len: metadata.len(),
            modified_ns: metadata
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            inode,
        }))
    }
}

/// What an observed path held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Observed {
    /// Nothing exists at the path.
    Absent,
    /// A file with this SHA-256.
    Sha256(String),
    /// A workspace member glob and the directories it matched.
    Matches(BTreeSet<PathBuf>),
}

/// The full observation a record is valid for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Inputs {
    cli: CliIdentity,
    /// The [`ENV_PREFIXES`] variables of the resolving host.
    env: BTreeMap<String, String>,
    /// `cargo`, `rustc` and `rustup` as the host's `PATH` resolves them.
    programs: BTreeMap<String, Option<Stamp>>,
    /// The identity of every installed rustup toolchain and of every
    /// custom toolchain `path` a toolchain file names.
    toolchains: BTreeMap<String, ToolchainStamp>,
    /// Every file observed by content: the project's `Water.toml` and
    /// `Water.lock`, the Cargo configuration chains, rustup's settings and
    /// toolchain files, the seeds and their closure.
    files: BTreeMap<PathBuf, Observed>,
}

/// An [`Inputs`] observation plus the newest modification time among the
/// files it hashed.
struct Observation {
    inputs: Inputs,
    newest: Option<SystemTime>,
}

/// A persisted answer: a [`RecordHeader`] line, then this body. Only the
/// header is read before the rest — a record written by another CLI build
/// is replaced, never parsed further.
#[derive(Serialize, Deserialize)]
struct Record<Q, V> {
    kind: String,
    question: Q,
    seeds: BTreeSet<PathBuf>,
    inputs: Inputs,
    value: V,
}

/// The first line of a record file: the CLI build that wrote it.
#[derive(Serialize, Deserialize)]
struct RecordHeader {
    cli: CliIdentity,
}

/// The in-memory form of a record, kept per question for the invocation.
struct Answer<V> {
    seeds: BTreeSet<PathBuf>,
    inputs: Inputs,
    value: Arc<V>,
}

type Slot = Arc<async_lock::Mutex<Option<Arc<dyn Any + Send + Sync>>>>;

/// The fact cache of one project.
///
/// Cloning shares the invocation's answers; nothing is process-global.
#[derive(Clone)]
pub struct Facts {
    host: Host,
    project_root: PathBuf,
    records: PathBuf,
    cli: CliIdentity,
    slots: Arc<async_lock::Mutex<BTreeMap<String, Slot>>>,
}

impl std::fmt::Debug for Facts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Facts")
            .field("project_root", &self.project_root)
            .field("records", &self.records)
            .finish_non_exhaustive()
    }
}

impl Facts {
    /// The facts of the project at `project_root`, recorded under
    /// `records`.
    ///
    /// # Errors
    /// Returns an error when the running CLI cannot be identified.
    pub(crate) fn new(host: &Host, project_root: &Path, records: &Path) -> eyre::Result<Self> {
        Ok(Self {
            host: host.clone(),
            project_root: project_root.to_path_buf(),
            records: records.join(format!("v{RECORD_FORMAT}")),
            cli: CliIdentity::current()?,
            slots: Arc::default(),
        })
    }

    /// These facts as a CLI with identity `cli` sees them.
    #[cfg(test)]
    pub(crate) fn with_cli(mut self, cli: CliIdentity) -> Self {
        self.cli = cli;
        self.slots = Arc::default();
        self
    }

    /// The host facts are resolved on.
    pub(crate) const fn host(&self) -> &Host {
        &self.host
    }

    /// `fact`'s answer: from this invocation's memory or a record while
    /// every input still observes the same, else resolved afresh.
    ///
    /// # Errors
    /// Returns an error when the fact cannot be resolved, its inputs cannot
    /// be observed, or a record of this CLI cannot be read.
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the question's slot stays locked across the resolve, so concurrent askers of one question share a single resolve"
    )]
    pub(crate) async fn answer<F: Fact>(&self, fact: &F) -> eyre::Result<Arc<F::Value>> {
        let question = fact.question();
        let key = record_key(F::KIND, &question)?;
        let slot = Arc::clone(
            self.slots
                .lock()
                .await
                .entry(key.clone())
                .or_insert_with(Slot::default),
        );
        let mut slot = slot.lock().await;

        if let Some(answer) = slot
            .as_ref()
            .map(|answer| {
                Arc::clone(answer)
                    .downcast::<Answer<F::Value>>()
                    .map_err(|_| eyre::eyre!("fact {key} is memoized with another value type"))
            })
            .transpose()?
            && self.observe(fact.cwd(), &answer.seeds).await?.inputs == answer.inputs
        {
            return Ok(Arc::clone(&answer.value));
        }

        let path = self.records.join(format!("{key}.json"));
        if let Some(record) = read_record::<F>(&path, &self.cli).await? {
            if record.kind != F::KIND || record.question != question {
                eyre::bail!(
                    "the fact record {} answers another question than its name; remove it",
                    path.display()
                );
            }
            if self.observe(fact.cwd(), &record.seeds).await?.inputs == record.inputs {
                tracing::debug!(fact = F::KIND, record = %path.display(), "fact replayed");
                let value = Arc::new(record.value);
                *slot = Some(Arc::new(Answer {
                    seeds: record.seeds,
                    inputs: record.inputs,
                    value: Arc::clone(&value),
                }));
                return Ok(value);
            }
        }

        let started = SystemTime::now();
        let before = self.observe(fact.cwd(), &BTreeSet::new()).await?;
        let value = fact.resolve(&self.host).await?;
        let seeds = fact.seeds(&value)?;
        let after = self.observe(fact.cwd(), &seeds).await?;
        let settled = after
            .newest
            .is_none_or(|newest| newest + MTIME_SLACK < started)
            && before.inputs.programs == after.inputs.programs
            && before.inputs.toolchains == after.inputs.toolchains
            && before.inputs.env == after.inputs.env;
        if settled {
            write_record(
                &path,
                &self.cli,
                &Record {
                    kind: F::KIND.to_owned(),
                    question,
                    seeds: seeds.clone(),
                    inputs: after.inputs.clone(),
                    value: &value,
                },
            )
            .await?;
            tracing::debug!(fact = F::KIND, record = %path.display(), "fact resolved and recorded");
        } else {
            tracing::debug!(
                fact = F::KIND,
                "fact resolved; an input changed around the resolve, so it is not recorded"
            );
        }
        let value = Arc::new(value);
        *slot = Some(Arc::new(Answer {
            seeds,
            inputs: after.inputs,
            value: Arc::clone(&value),
        }));
        Ok(value)
    }

    /// `cargo metadata` as `command` configures it.
    ///
    /// The record observes what the resolve reads — manifests, lockfile,
    /// configuration, toolchain — not the source files Cargo scans to
    /// auto-discover a package's `targets`, so a replayed document's
    /// `targets` lists are not current; callers read the resolve, never
    /// them.
    ///
    /// # Errors
    /// Returns an error when Cargo fails or its inputs cannot be observed.
    pub(crate) async fn cargo_metadata(&self, command: &MetadataCommand) -> eyre::Result<Metadata> {
        let fact = CargoMetadata::new(command, self.host.cwd());
        let json = self.answer(&fact).await?;
        Ok(MetadataCommand::parse(json.as_str())?)
    }

    /// The rustup toolchain `directory` selects —
    /// `rustup show active-toolchain` there.
    ///
    /// # Errors
    /// Returns an error when rustup resolves no toolchain.
    pub(crate) async fn active_toolchain(&self, directory: &Path) -> eyre::Result<String> {
        let fact = ActiveToolchain {
            cwd: directory.to_path_buf(),
        };
        Ok(self.answer(&fact).await?.as_ref().clone())
    }

    /// `rustc --print target-libdir --target <triple>` under the rustup
    /// toolchain `toolchain`.
    ///
    /// # Errors
    /// Returns an error when rustc fails or prints nothing.
    pub(crate) async fn target_libdir(
        &self,
        triple: &Triple,
        toolchain: &str,
    ) -> eyre::Result<PathBuf> {
        let fact = TargetLibdir {
            cwd: self.host.cwd().to_path_buf(),
            triple: triple.to_string(),
            toolchain: toolchain.to_owned(),
        };
        Ok(self.answer(&fact).await?.as_ref().clone())
    }

    /// Observe every input a fact resolved in `cwd` from `seeds` depends
    /// on.
    async fn observe(&self, cwd: &Path, seeds: &BTreeSet<PathBuf>) -> eyre::Result<Observation> {
        let mut programs = BTreeMap::new();
        for program in ["cargo", "rustc", "rustup"] {
            let resolved = self.host.which(program).await.ok();
            programs.insert(program.to_owned(), resolved);
        }
        let probe = Probe {
            cli: self.cli.clone(),
            env: self
                .host
                .envs()
                .filter_map(|(key, value)| {
                    let key = key.to_str()?;
                    ENV_PREFIXES
                        .iter()
                        .any(|prefix| key.starts_with(prefix))
                        .then(|| (key.to_owned(), value.to_string_lossy().into_owned()))
                })
                .collect(),
            programs,
            rustup_home: self.host.env("RUSTUP_HOME").map_or_else(
                || self.host.home_dir().map(|home| home.join(".rustup")),
                |home| Some(PathBuf::from(home)),
            ),
            cargo_home: cargo_home(&self.host, cwd),
            cwd: cwd.to_path_buf(),
            project_root: self.project_root.clone(),
            seeds: seeds.clone(),
        };
        smol::unblock(move || probe.observe())
            .await
            .wrap_err("cannot observe the inputs of a recorded fact")
    }
}

/// Cargo's `$CARGO_HOME`: absolute, anchored at `cwd` when relative,
/// `~/.cargo` when unset — the same resolution `cargo_config2` performs.
fn cargo_home(host: &Host, cwd: &Path) -> Option<PathBuf> {
    host.env_string("CARGO_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| {
            if home.is_absolute() {
                home
            } else {
                cwd.join(home)
            }
        })
        .or_else(|| host.home_dir().map(|home| home.join(".cargo")))
}

/// The record file stem of a question: its kind plus a digest of its
/// serialized form.
fn record_key<Q: Serialize>(kind: &str, question: &Q) -> eyre::Result<String> {
    let serialized = serde_json::to_vec(question)?;
    Ok(format!("{kind}-{}", &sha256_hex(&serialized)[..32]))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// The record at `path` when this CLI wrote it; `None` when there is none
/// or another CLI build wrote it.
async fn read_record<F: Fact>(
    path: &Path,
    cli: &CliIdentity,
) -> eyre::Result<Option<Record<F::Question, F::Value>>> {
    let bytes = match smol::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("cannot read {}", path.display()));
        }
    };
    let path = path.to_path_buf();
    let cli = cli.clone();
    smol::unblock(move || {
        let corrupt = || {
            format!(
                "the fact record {} is corrupt; remove it (or run `water clean`) to resolve the fact again",
                path.display()
            )
        };
        let newline = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| eyre::eyre!("the record has no header line"))
            .wrap_err_with(corrupt)?;
        let header: RecordHeader =
            serde_json::from_slice(&bytes[..newline]).wrap_err_with(corrupt)?;
        if header.cli != cli {
            return Ok(None);
        }
        serde_json::from_slice(&bytes[newline + 1..])
            .map(Some)
            .wrap_err_with(corrupt)
    })
    .await
}

/// Persist `record` at `path`, staged and renamed so a reader never sees a
/// torn record.
async fn write_record<Q: Serialize + Sync, V: Serialize + Sync>(
    path: &Path,
    cli: &CliIdentity,
    record: &Record<Q, &V>,
) -> eyre::Result<()> {
    // JSON escapes every newline inside strings, so the header's own
    // newline is the only one before the body.
    let mut bytes = serde_json::to_vec(&RecordHeader { cli: cli.clone() })?;
    bytes.push(b'\n');
    serde_json::to_writer(&mut bytes, record)?;
    let path = path.to_path_buf();
    smol::unblock(move || {
        use std::io::Write as _;
        let directory = path
            .parent()
            .ok_or_else(|| eyre::eyre!("{} has no parent directory", path.display()))?;
        std::fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&bytes)?;
        file.persist(&path)?;
        eyre::Ok(())
    })
    .await
    .wrap_err("cannot record a resolved fact")
}

/// Everything [`Facts::observe`] gathered on the executor, moved to the
/// blocking pool for the filesystem walk.
struct Probe {
    cli: CliIdentity,
    env: BTreeMap<String, String>,
    programs: BTreeMap<String, Option<PathBuf>>,
    rustup_home: Option<PathBuf>,
    cargo_home: Option<PathBuf>,
    cwd: PathBuf,
    project_root: PathBuf,
    seeds: BTreeSet<PathBuf>,
}

impl Probe {
    fn observe(self) -> io::Result<Observation> {
        let mut files = Files::default();
        let mut toolchains = BTreeMap::new();

        for name in ["Water.toml", "Water.lock"] {
            files.hash(&self.project_root.join(name))?;
        }

        // The configuration chains Cargo discovers from the resolving
        // directory and from the project root — a managed crate's build
        // carries the project's chain through `--config` — and every
        // `[patch]` path they declare.
        for root in [&self.cwd, &self.project_root] {
            let root = dunce::canonicalize(root).unwrap_or_else(|_| root.clone());
            for config in cargo_config2::Walk::with_cargo_home(&root, self.cargo_home.clone()) {
                files.hash(&config)?;
                if let Some(table) = read_table(&config)
                    && let Some(base) = config.parent().and_then(Path::parent)
                {
                    for path in patch_paths(&table, base) {
                        files.hash(&path.join("Cargo.toml"))?;
                    }
                }
            }
        }

        // rustup's selection: its settings (default toolchain, directory
        // overrides), the toolchain files from the resolving directory up,
        // and the executables of every toolchain it could select.
        if let Some(rustup_home) = &self.rustup_home {
            files.hash(&rustup_home.join("settings.toml"))?;
            match std::fs::read_dir(rustup_home.join("toolchains")) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry?;
                        toolchains.insert(
                            entry.file_name().to_string_lossy().into_owned(),
                            toolchain_stamps(&entry.path())?,
                        );
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        for directory in self.cwd.ancestors() {
            for name in ["rust-toolchain", "rust-toolchain.toml"] {
                let file = directory.join(name);
                if file.is_file() {
                    files.hash(&file)?;
                    if let Some(custom) = read_table(&file)
                        .as_ref()
                        .and_then(|table| table.get("toolchain"))
                        .and_then(|toolchain| toolchain.get("path"))
                        .and_then(toml::Value::as_str)
                    {
                        let custom = directory.join(custom);
                        toolchains.insert(
                            format!("path:{}", custom.display()),
                            toolchain_stamps(&custom)?,
                        );
                    }
                }
            }
        }

        // The seeds, the workspace root each manifest among them inherits
        // from, and what those roots reach without appearing in a resolve:
        // their `[patch]`/`[replace]` targets and their member globs.
        let mut roots = BTreeSet::new();
        for seed in &self.seeds {
            files.hash(seed)?;
            if seed.file_name() == Some(OsStr::new("Cargo.toml"))
                && let Some(root) = seed.parent().and_then(workspace_manifest_of)
            {
                roots.insert(root);
            }
        }
        for root in roots {
            files.hash(&root)?;
            let (Some(table), Some(base)) = (read_table(&root), root.parent()) else {
                continue;
            };
            for path in patch_paths(&table, base) {
                files.hash(&path.join("Cargo.toml"))?;
            }
            files.member_globs(&table, base);
        }

        let mut programs = BTreeMap::new();
        for (name, path) in self.programs {
            let stamp = path.as_deref().map(Stamp::of).transpose()?.flatten();
            programs.insert(name, stamp);
        }

        Ok(Observation {
            inputs: Inputs {
                cli: self.cli,
                env: self.env,
                programs,
                toolchains,
                files: files.observed,
            },
            newest: files.newest,
        })
    }
}

/// The content observations of one [`Probe`].
#[derive(Default)]
struct Files {
    observed: BTreeMap<PathBuf, Observed>,
    newest: Option<SystemTime>,
}

impl Files {
    fn hash(&mut self, path: &Path) -> io::Result<()> {
        if self.observed.contains_key(path) {
            return Ok(());
        }
        let observed = match std::fs::read(path) {
            Ok(bytes) => {
                let modified = std::fs::metadata(path)?.modified()?;
                self.newest = Some(self.newest.map_or(modified, |newest| newest.max(modified)));
                Observed::Sha256(sha256_hex(&bytes))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Observed::Absent,
            Err(error) => return Err(error),
        };
        self.observed.insert(path.to_path_buf(), observed);
        Ok(())
    }

    /// Record what each glob in `[workspace] members` matches under `base`:
    /// a directory appearing under a glob joins the workspace without any
    /// manifest or lockfile changing.
    fn member_globs(&mut self, table: &toml::Table, base: &Path) {
        let patterns = table
            .get("workspace")
            .and_then(|workspace| workspace.get("members"))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .filter(|pattern| pattern.contains(['*', '?', '[']));
        for pattern in patterns {
            let full = base.join(pattern);
            let matches = glob::glob(&full.to_string_lossy())
                .map(|paths| paths.filter_map(Result::ok).collect())
                .unwrap_or_default();
            self.observed.insert(full, Observed::Matches(matches));
        }
    }
}

/// A toolchain directory's identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ToolchainStamp {
    rustc: Option<Stamp>,
    cargo: Option<Stamp>,
    /// The file names of the compiler's `rustc_driver` library. The name
    /// carries the hash of the compiler build, so two different compilers
    /// never share it however their files were written.
    drivers: BTreeSet<String>,
}

/// The identity of the toolchain installed at `toolchain`.
fn toolchain_stamps(toolchain: &Path) -> io::Result<ToolchainStamp> {
    let bin = toolchain.join("bin");
    let exe = std::env::consts::EXE_SUFFIX;
    let mut drivers = BTreeSet::new();
    // `lib/` on Unix, `bin/` beside `rustc.exe` on Windows.
    for directory in [toolchain.join("lib"), bin.clone()] {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if name.contains("rustc_driver-") {
                drivers.insert(name);
            }
        }
    }
    Ok(ToolchainStamp {
        rustc: Stamp::of(&bin.join(format!("rustc{exe}")))?,
        cargo: Stamp::of(&bin.join(format!("cargo{exe}")))?,
        drivers,
    })
}

fn read_table(path: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse().ok())
}

/// The directories a manifest's or configuration's `[patch.<source>]` and
/// `[replace]` entries name by `path`, resolved against `base`.
fn patch_paths(table: &toml::Table, base: &Path) -> Vec<PathBuf> {
    let patches = table
        .get("patch")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(toml::Table::values)
        .filter_map(toml::Value::as_table)
        .flat_map(toml::Table::values);
    let replacements = table
        .get("replace")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(toml::Table::values);
    patches
        .chain(replacements)
        .filter_map(|entry| entry.get("path").and_then(toml::Value::as_str))
        .map(|path| base.join(path))
        .collect()
}

/// The workspace root manifest a package directory inherits
/// `[workspace.*]` settings from, per Cargo's own rule: an explicit
/// `[package] workspace = "<path>"` pointer when the package's manifest
/// carries one, else the nearest ancestor `Cargo.toml` carrying a
/// `[workspace]` table.
fn workspace_manifest_of(package_dir: &Path) -> Option<PathBuf> {
    for ancestor in package_dir.ancestors() {
        let manifest_path = ancestor.join("Cargo.toml");
        let Some(manifest) = read_table(&manifest_path) else {
            continue;
        };
        if ancestor == package_dir
            && let Some(pointer) = manifest
                .get("package")
                .and_then(|package| package.get("workspace"))
                .and_then(toml::Value::as_str)
            && let Ok(root) = dunce::canonicalize(ancestor.join(pointer))
        {
            return Some(root.join("Cargo.toml"));
        }
        if manifest.contains_key("workspace") {
            return Some(manifest_path);
        }
    }
    None
}

/// `cargo metadata` with the arguments, working directory and environment
/// overrides a [`MetadataCommand`] carries.
struct CargoMetadata<'a> {
    command: &'a MetadataCommand,
    question: CargoMetadataQuestion,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CargoMetadataQuestion {
    args: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, Option<String>>,
}

impl<'a> CargoMetadata<'a> {
    fn new(command: &'a MetadataCommand, host_cwd: &Path) -> Self {
        let invocation = command.cargo_command();
        let cwd = invocation
            .get_current_dir()
            .map_or_else(|| host_cwd.to_path_buf(), Path::to_path_buf);
        Self {
            command,
            question: CargoMetadataQuestion {
                args: invocation
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
                cwd,
                env: invocation
                    .get_envs()
                    .map(|(key, value)| {
                        (
                            key.to_string_lossy().into_owned(),
                            value.map(|value| value.to_string_lossy().into_owned()),
                        )
                    })
                    .collect(),
            },
        }
    }
}

impl Fact for CargoMetadata<'_> {
    const KIND: &'static str = "cargo-metadata";
    type Question = CargoMetadataQuestion;
    /// The JSON document Cargo printed.
    type Value = String;

    fn question(&self) -> Self::Question {
        CargoMetadataQuestion {
            args: self.question.args.clone(),
            cwd: self.question.cwd.clone(),
            env: self.question.env.clone(),
        }
    }

    fn cwd(&self) -> &Path {
        &self.question.cwd
    }

    async fn resolve(&self, host: &Host) -> eyre::Result<String> {
        Ok(host.cargo_metadata_json(self.command).await?)
    }

    /// Every path package's manifest, the manifest behind every `path`
    /// dependency any of them declares — optional and other-target ones
    /// included, which a resolve reads without listing — and the
    /// workspace's manifest and lockfile.
    fn seeds(&self, value: &String) -> eyre::Result<BTreeSet<PathBuf>> {
        let metadata = MetadataCommand::parse(value)?;
        let workspace_root = metadata.workspace_root.as_std_path();
        let mut seeds = BTreeSet::from([
            workspace_root.join("Cargo.toml"),
            workspace_root.join("Cargo.lock"),
        ]);
        for package in metadata
            .packages
            .iter()
            .filter(|package| package.source.is_none())
        {
            seeds.insert(package.manifest_path.clone().into_std_path_buf());
            seeds.extend(
                package
                    .dependencies
                    .iter()
                    .filter_map(|dependency| dependency.path.as_ref())
                    .map(|path| path.as_std_path().join("Cargo.toml")),
            );
        }
        Ok(seeds)
    }
}

/// `rustup show active-toolchain` in a directory.
struct ActiveToolchain {
    cwd: PathBuf,
}

impl Fact for ActiveToolchain {
    const KIND: &'static str = "rustup-active-toolchain";
    type Question = PathBuf;
    type Value = String;

    fn question(&self) -> PathBuf {
        self.cwd.clone()
    }

    fn cwd(&self) -> &Path {
        &self.cwd
    }

    async fn resolve(&self, host: &Host) -> eyre::Result<String> {
        Ok(
            crate::toolchain::rust::selected_rustup_toolchain(&host.clone().with_cwd(&self.cwd))
                .await?,
        )
    }

    fn seeds(&self, _: &String) -> eyre::Result<BTreeSet<PathBuf>> {
        Ok(BTreeSet::new())
    }
}

/// `rustc --print target-libdir --target <triple>` under a rustup
/// toolchain.
struct TargetLibdir {
    cwd: PathBuf,
    triple: String,
    toolchain: String,
}

impl Fact for TargetLibdir {
    const KIND: &'static str = "rustc-target-libdir";
    type Question = (PathBuf, String, String);
    type Value = PathBuf;

    fn question(&self) -> Self::Question {
        (
            self.cwd.clone(),
            self.triple.clone(),
            self.toolchain.clone(),
        )
    }

    fn cwd(&self) -> &Path {
        &self.cwd
    }

    async fn resolve(&self, host: &Host) -> eyre::Result<PathBuf> {
        let output = host
            .with_env("RUSTUP_TOOLCHAIN", &self.toolchain)
            .with_cwd(&self.cwd)
            .run(
                "rustc",
                ["--print", "target-libdir", "--target", self.triple.as_str()],
            )
            .await?;
        let libdir = output.trim();
        if libdir.is_empty() {
            eyre::bail!(
                "`rustc --print target-libdir --target {}` returned an empty path",
                self.triple
            );
        }
        Ok(PathBuf::from(libdir))
    }

    fn seeds(&self, _: &PathBuf) -> eyre::Result<BTreeSet<PathBuf>> {
        Ok(BTreeSet::new())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
        str::FromStr as _,
    };

    use cargo_metadata::MetadataCommand;
    use target_lexicon::Triple;

    use super::{CliIdentity, Facts};
    use crate::toolchain::{Host, testing::TestMachine};

    /// A machine with fake `cargo`, `rustc` and `rustup`, a rustup home
    /// with one installed toolchain, and a workspace whose application
    /// reaches a path dependency outside the workspace, an unused
    /// `[patch]` target and a member glob.
    struct Fixture {
        machine: TestMachine,
        root: PathBuf,
        log: PathBuf,
    }

    /// The fixed modification time every fixture input is set to, far
    /// enough in the past that a record is written for it.
    const SETTLED: i64 = 1_700_000_000;

    impl Fixture {
        fn new() -> Self {
            let machine = TestMachine::new();
            for tool in ["cargo", "rustc", "rustup"] {
                machine.install(tool);
            }
            let root = dunce::canonicalize(machine.root()).expect("machine root canonicalizes");
            let fixture = Self {
                log: machine.file("invocations.log", ""),
                machine,
                root,
            };
            fixture.write(
                "ws/Cargo.toml",
                "[workspace]\nmembers = [\"app\", \"crates/*\"]\nresolver = \"2\"\n\n\
                 [patch.crates-io]\npatched = { path = \"../patched\" }\n",
            );
            fixture.write("ws/Cargo.lock", "# this file is @generated\nversion = 4\n");
            fixture.write(
                "ws/app/Cargo.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\ndep = { path = \"../../outside/dep\" }\n",
            );
            fixture.write(
                "ws/app/Water.toml",
                "[package]\nname = \"App\"\nbundle_identifier = \"dev.waterui.app\"\n",
            );
            fixture.write(
                "ws/crates/a/Cargo.toml",
                "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            );
            fixture.write(
                "outside/dep/Cargo.toml",
                "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            );
            fixture.write(
                "patched/Cargo.toml",
                "[package]\nname = \"patched\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            );
            fixture.write("rustup/settings.toml", "default_toolchain = \"stable\"\n");
            fixture.write("rustup/toolchains/stable-fake/bin/rustc", "rustc 1");
            fixture.write(
                "rustup/toolchains/stable-fake/lib/librustc_driver-1111.dylib",
                "",
            );
            fixture.write("rustup/toolchains/stable-fake/bin/cargo", "cargo 1");
            fixture.machine.dir("cargo-home");
            fixture.machine.respond(
                "RUSTUP_ACTIVE_TOOLCHAIN",
                "stable-x86_64-unknown-fake (default)",
            );
            fixture
                .machine
                .respond("RUSTC_PRINT_target-libdir", "/fake/rustlib/lib");
            fixture
                .machine
                .respond("CARGO_METADATA", &fixture.metadata_json());
            fixture.settle();
            fixture
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        fn app(&self) -> PathBuf {
            self.path("ws/app")
        }

        fn write(&self, relative: &str, contents: &str) {
            self.machine.file(relative, contents);
        }

        fn append(&self, relative: &str) {
            let path = self.path(relative);
            let text = std::fs::read_to_string(&path).expect("the input reads");
            std::fs::write(&path, format!("{text}# edited\n")).expect("the input writes");
        }

        /// Set every fixture input's modification time to [`SETTLED`].
        fn settle(&self) {
            let time = filetime::FileTime::from_unix_time(SETTLED, 0);
            for top in ["ws", "outside", "patched", "rustup", "cargo-home"] {
                let mut pending = vec![self.path(top)];
                while let Some(path) = pending.pop() {
                    if path.is_dir() {
                        pending.extend(
                            std::fs::read_dir(&path)
                                .expect("fixture dir lists")
                                .map(|entry| entry.expect("fixture entry").path()),
                        );
                    } else {
                        filetime::set_file_mtime(&path, time).expect("fixture mtime");
                    }
                }
            }
        }

        fn host(&self, extra: &[(&str, &str)]) -> Host {
            let mut vars: Vec<(OsString, OsString)> = vec![
                ("WATERUI_FAKE_LOG".into(), self.log.clone().into()),
                ("RUSTUP_HOME".into(), self.path("rustup").into()),
                ("CARGO_HOME".into(), self.path("cargo-home").into()),
            ];
            vars.extend(
                extra
                    .iter()
                    .map(|(key, value)| ((*key).into(), (*value).into())),
            );
            self.machine.host(vars).with_cwd(self.app())
        }

        /// A fresh invocation's facts: nothing memoized, records on disk.
        fn facts(&self, host: &Host) -> Facts {
            Facts::new(host, &self.app(), &self.path("records")).expect("the CLI identifies")
        }

        fn spawns(&self) -> Vec<String> {
            std::fs::read_to_string(&self.log)
                .expect("invocation log")
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn metadata_json(&self) -> String {
            let package = |name: &str, dir: &Path, dependencies: serde_json::Value| {
                serde_json::json!({
                    "name": name,
                    "version": "0.1.0",
                    "id": format!("path+file://{}#{name}@0.1.0", dir.display()),
                    "license": null, "license_file": null, "description": null,
                    "source": null,
                    "dependencies": dependencies,
                    "manifest_path": dir.join("Cargo.toml"),
                    "categories": [], "keywords": [], "readme": null,
                    "repository": null, "homepage": null, "documentation": null,
                    "edition": "2024", "links": null, "default_run": null,
                    "rust_version": null, "metadata": null, "features": {},
                    "targets": [], "publish": null, "authors": []
                })
            };
            let dep = self.path("outside/dep");
            let app = package(
                "app",
                &self.app(),
                serde_json::json!([{
                    "name": "dep", "source": null, "req": "*", "kind": null,
                    "rename": null, "optional": false, "uses_default_features": true,
                    "features": [], "target": null, "registry": null, "path": dep
                }]),
            );
            serde_json::json!({
                "packages": [app, package("dep", &dep, serde_json::json!([]))],
                "workspace_members": [],
                "workspace_default_members": [],
                "resolve": null,
                "target_directory": self.path("ws/target"),
                "version": 1,
                "workspace_root": self.path("ws"),
                "metadata": null
            })
            .to_string()
        }
    }

    /// Ask the three kinds of fact a build asks: the project's metadata,
    /// the toolchain its directory selects and a target's libdir.
    async fn ask(fixture: &Fixture, facts: &Facts) -> eyre::Result<()> {
        let mut command = MetadataCommand::new();
        command.current_dir(fixture.app());
        facts.cargo_metadata(&command).await?;
        let toolchain = facts.active_toolchain(&fixture.app()).await?;
        assert_eq!(toolchain, "stable-x86_64-unknown-fake");
        let triple = Triple::from_str("aarch64-apple-darwin").expect("triple");
        assert_eq!(
            facts.target_libdir(&triple, &toolchain).await?,
            PathBuf::from("/fake/rustlib/lib")
        );
        Ok(())
    }

    /// An input change, the edit that makes it, and the facts it must
    /// re-resolve.
    type Change = (&'static str, Box<dyn Fn(&Fixture)>, &'static [&'static str]);

    const METADATA: &str = "cargo metadata";
    const ALL: [&str; 3] = [
        "cargo metadata",
        "rustup show active-toolchain",
        "rustc --print target-libdir",
    ];

    /// The tools a run of [`ask`] spawned, by prefix, since `seen` lines.
    fn spawned_since(fixture: &Fixture, seen: usize) -> Vec<&'static str> {
        let spawns = fixture.spawns();
        ALL.into_iter()
            .filter(|tool| spawns[seen..].iter().any(|line| line.starts_with(tool)))
            .collect()
    }

    #[test]
    fn a_warm_invocation_spawns_no_cargo_or_toolchain_process() {
        smol::block_on(async {
            let fixture = Fixture::new();
            let host = fixture.host(&[]);

            let facts = fixture.facts(&host);
            ask(&fixture, &facts).await.expect("the facts resolve");
            assert_eq!(spawned_since(&fixture, 0), ALL, "the first ask resolves");
            let resolved = fixture.spawns().len();

            // The same invocation asks again: answered from memory.
            ask(&fixture, &facts).await.expect("the facts answer again");
            // A new invocation: answered from the records.
            ask(&fixture, &fixture.facts(&host))
                .await
                .expect("the facts replay");
            assert_eq!(
                fixture.spawns().len(),
                resolved,
                "a warm ask spawns nothing: {:?}",
                fixture.spawns()
            );
        });
    }

    /// Each input change, and which facts it must re-resolve: the inputs
    /// of a resolve re-resolve only the fact read from them; the
    /// configuration, project and toolchain inputs every fact shares
    /// re-resolve all three.
    fn input_changes() -> Vec<Change> {
        vec![
            (
                "application manifest",
                Box::new(|f| f.append("ws/app/Cargo.toml")),
                &[METADATA],
            ),
            (
                "lockfile",
                Box::new(|f| f.append("ws/Cargo.lock")),
                &[METADATA],
            ),
            (
                "path dependency outside the workspace",
                Box::new(|f| f.append("outside/dep/Cargo.toml")),
                &[METADATA],
            ),
            (
                "workspace root manifest",
                Box::new(|f| f.append("ws/Cargo.toml")),
                &[METADATA],
            ),
            (
                "unused [patch] target",
                Box::new(|f| f.append("patched/Cargo.toml")),
                &[METADATA],
            ),
            (
                "a new member under a workspace glob",
                Box::new(|f| {
                    f.write(
                        "ws/crates/b/Cargo.toml",
                        "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
                    );
                }),
                &[METADATA],
            ),
            (
                "a project cargo config",
                Box::new(|f| f.write("ws/app/.cargo/config.toml", "[build]\njobs = 2\n")),
                &ALL,
            ),
            (
                "the CARGO_HOME config",
                Box::new(|f| f.write("cargo-home/config.toml", "[net]\noffline = true\n")),
                &ALL,
            ),
            (
                "Water.toml",
                Box::new(|f| f.append("ws/app/Water.toml")),
                &ALL,
            ),
            (
                "a new Water.lock",
                Box::new(|f| f.write("ws/app/Water.lock", "version = 1\n")),
                &ALL,
            ),
            (
                "a rust-toolchain.toml above the project",
                Box::new(|f| {
                    f.write(
                        "ws/rust-toolchain.toml",
                        "[toolchain]\nchannel = \"beta\"\n",
                    );
                }),
                &ALL,
            ),
            (
                "rustup settings",
                Box::new(|f| f.append("rustup/settings.toml")),
                &ALL,
            ),
            (
                "an updated toolchain",
                Box::new(|f| {
                    // The update's files have the old files' sizes and
                    // (once settled) times: only the build-hashed
                    // driver name tells the two compilers apart.
                    let lib = f.path("rustup/toolchains/stable-fake/lib");
                    std::fs::rename(
                        lib.join("librustc_driver-1111.dylib"),
                        lib.join("librustc_driver-2222.dylib"),
                    )
                    .expect("the driver is replaced");
                    f.write("rustup/toolchains/stable-fake/bin/rustc", "rustc 2");
                }),
                &ALL,
            ),
            (
                "a newly installed toolchain",
                Box::new(|f| f.write("rustup/toolchains/nightly-fake/bin/rustc", "rustc n")),
                &ALL,
            ),
        ]
    }

    #[test]
    fn every_input_change_resolves_again() {
        smol::block_on(async {
            let fixture = Fixture::new();
            let host = fixture.host(&[]);
            ask(&fixture, &fixture.facts(&host))
                .await
                .expect("the facts resolve");

            for (name, change, expected) in input_changes() {
                change(&fixture);
                fixture.settle();
                let seen = fixture.spawns().len();
                ask(&fixture, &fixture.facts(&host))
                    .await
                    .unwrap_or_else(|error| panic!("{name}: {error:?}"));
                assert_eq!(spawned_since(&fixture, seen), expected, "{name} changed");
                let seen = fixture.spawns().len();
                ask(&fixture, &fixture.facts(&host))
                    .await
                    .unwrap_or_else(|error| panic!("{name}: {error:?}"));
                assert_eq!(
                    spawned_since(&fixture, seen),
                    Vec::<&str>::new(),
                    "{name}: the re-resolved facts were recorded"
                );
            }

            // The environment Cargo and rustup read.
            let rustflags = fixture.host(&[("RUSTFLAGS", "--cfg fixture")]);
            let seen = fixture.spawns().len();
            ask(&fixture, &fixture.facts(&rustflags))
                .await
                .expect("the facts resolve under RUSTFLAGS");
            assert_eq!(spawned_since(&fixture, seen), ALL, "RUSTFLAGS changed");

            // Another CLI build never replays this one's records.
            let seen = fixture.spawns().len();
            let other_cli = fixture
                .facts(&host)
                .with_cli(CliIdentity::with_version("0.0.0-other"));
            ask(&fixture, &other_cli)
                .await
                .expect("the facts resolve for another CLI");
            assert_eq!(spawned_since(&fixture, seen), ALL, "the CLI changed");
        });
    }

    /// A resolve whose inputs were modified around it is answered but not
    /// recorded: Cargo may have read the input before its last write.
    #[test]
    fn inputs_modified_around_the_resolve_are_not_recorded() {
        smol::block_on(async {
            let fixture = Fixture::new();
            let host = fixture.host(&[]);
            fixture.append("ws/app/Cargo.toml");
            ask(&fixture, &fixture.facts(&host))
                .await
                .expect("the facts resolve");
            let seen = fixture.spawns().len();
            ask(&fixture, &fixture.facts(&host))
                .await
                .expect("the facts resolve again");
            assert_eq!(spawned_since(&fixture, seen), [METADATA]);
        });
    }

    /// A record this CLI wrote but cannot read is an error naming it, never
    /// a silent recompute that would hide a write/read asymmetry.
    #[test]
    fn a_corrupt_record_is_an_error() {
        smol::block_on(async {
            let fixture = Fixture::new();
            let host = fixture.host(&[]);
            ask(&fixture, &fixture.facts(&host))
                .await
                .expect("the facts resolve");
            let records = std::fs::read_dir(fixture.path("records/v1"))
                .expect("records exist")
                .map(|entry| entry.expect("record entry").path())
                .collect::<Vec<_>>();
            assert_eq!(records.len(), 3, "one record per fact");
            for record in &records {
                let bytes = std::fs::read(record).expect("record reads");
                let header_end = bytes.iter().position(|byte| *byte == b'\n').unwrap();
                let mut truncated = bytes[..=header_end].to_vec();
                truncated.extend_from_slice(b"{\"kind\":");
                std::fs::write(record, truncated).expect("record truncates");
            }
            let error = ask(&fixture, &fixture.facts(&host))
                .await
                .expect_err("a corrupt record fails the ask");
            assert!(
                format!("{error:?}").contains("is corrupt"),
                "the error names the corrupt record: {error:?}"
            );
        });
    }
}
