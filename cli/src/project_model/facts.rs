//! Facts the CLI derives from Cargo, the Rust toolchain and the files of a
//! project — `cargo metadata`, `cargo tree`, the rustup toolchain a
//! directory selects, a target's standard-library directory, and the small
//! answers the build derives from them.
//!
//! Each fact is a pure function of its inputs, and a fact reports those
//! inputs while it resolves, through its [`Scope`]: the directories a
//! Cargo, rustup or rustc process ran in (which brings in the Cargo
//! configuration chain, rustup's selection and the toolchain identity), the
//! manifests and lockfiles a resolve read (which brings in their workspace
//! roots, those roots' `[patch]` targets and member globs), the files it
//! read itself, and the facts it asked in turn. The project's
//! `Water.toml`/`Water.lock` and the CLI build are inputs of every fact.
//!
//! [`Facts`] answers a question at most once per invocation and reuses an
//! answer across invocations only through an on-disk record that carries
//! the closed set of paths the resolve depended on — every file the
//! expansion of its manifests consulted, present or absent — and the state
//! each of them was observed in. A later ask observes exactly those paths
//! again, hashing files and listing globs, never parsing a manifest or
//! spawning a process, and replays the record only when every observation
//! is identical. Any difference resolves the fact again.
//!
//! Within one invocation every file is read and hashed once: a later
//! observation of the same path re-reads it only when its metadata stamp
//! (length, modification time, device and inode) changed, so the files
//! the CLI itself writes between two asks are observed again and the
//! files nothing touched cost a `stat`.
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
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
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
const RECORD_FORMAT: u32 = 3;

/// The environment variables whose values reach Cargo's or rustup's
/// decisions: every `CARGO*`, `RUSTUP*` and `RUSTC*` variable, and the
/// rustflags families.
const ENV_PREFIXES: [&str; 5] = ["CARGO", "RUSTUP", "RUSTC", "RUSTFLAGS", "RUSTDOCFLAGS"];

/// One question the CLI asks Cargo, the toolchain or the project's files.
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

    /// Answer the question, reporting to `scope` every input the answer is
    /// a function of. An input the resolve reads without reporting it is a
    /// bug: its change would replay a stale answer.
    fn resolve(&self, scope: &Scope<'_>) -> impl Future<Output = eyre::Result<Self::Value>> + Send;
}

/// The CLI build that wrote a record: its version and its executable's
/// file identity, so a rebuilt binary of the same version never replays
/// another build's records — while one binary reached through any
/// spelling of its path replays its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliIdentity {
    version: String,
    executable: FileStamp,
}

impl CliIdentity {
    fn current() -> eyre::Result<Self> {
        Self::of_invocation(&Host::current_exe().wrap_err("cannot locate the running CLI")?)
    }

    /// The identity of an invocation that reached the binary through
    /// `executable`. Canonicalizing resolves symlinks and `..`, and the
    /// path itself is not part of the identity — every spelling of one
    /// file is the same CLI.
    fn of_invocation(executable: &Path) -> eyre::Result<Self> {
        let canonical = std::fs::canonicalize(executable)
            .wrap_err_with(|| format!("cannot resolve the running CLI {}", executable.display()))?;
        Ok(Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            executable: FileStamp::of(&canonical)?
                .ok_or_else(|| eyre::eyre!("the running CLI {} is gone", canonical.display()))?,
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

    /// The identity an invocation reaching the binary through
    /// `executable` holds — a test's stand-in for the same CLI invoked
    /// through another path spelling.
    #[cfg(test)]
    pub(crate) fn invoked_as(executable: &Path) -> Self {
        Self::of_invocation(executable).expect("the invoked CLI has an identity")
    }
}

/// A file's identity by metadata: length, modification time and, on
/// Unix, device and inode — a replaced file changes at least one of
/// them, and every spelling of a path to it shares them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct FileStamp {
    len: u64,
    modified_ns: u128,
    device: u64,
    inode: u64,
}

impl FileStamp {
    /// `path`'s identity, following symlinks; `None` when nothing is
    /// there.
    fn of(path: &Path) -> io::Result<Option<Self>> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        #[cfg(unix)]
        let (device, inode) = (
            std::os::unix::fs::MetadataExt::dev(&metadata),
            std::os::unix::fs::MetadataExt::ino(&metadata),
        );
        #[cfg(not(unix))]
        let (device, inode) = (0, 0);
        Ok(Some(Self {
            len: metadata.len(),
            modified_ns: metadata
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            device,
            inode,
        }))
    }
}

/// A file's identity together with the path it was observed through: for
/// a resolved tool (`cargo`, `rustc`, `rustup`, a toolchain's binaries)
/// the path is itself part of the observation — a different `PATH`
/// resolution is a different input however the files relate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    path: PathBuf,
    file: FileStamp,
}

impl Stamp {
    /// `path`'s stamp, following symlinks; `None` when nothing is there.
    fn of(path: &Path) -> io::Result<Option<Self>> {
        Ok(FileStamp::of(path)?.map(|file| Self {
            path: path.to_path_buf(),
            file,
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
    /// A glob and the paths it matched.
    Matches(BTreeSet<PathBuf>),
}

/// The paths an answer depends on, closed under what was consulted to
/// find them: replaying an answer observes exactly these, and the
/// observation of every one of them equal means a resolve would consult
/// the same paths again and read the same contents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watch {
    /// The directories a Cargo, rustup or rustc process ran in: each
    /// brings in the Cargo configuration chain discovered from it, the
    /// rustup toolchain files above it, and the toolchain identity.
    toolchain: BTreeSet<PathBuf>,
    /// Files observed by content, absent ones included.
    files: BTreeSet<PathBuf>,
    /// Globs observed by what they match.
    globs: BTreeSet<PathBuf>,
}

impl Watch {
    fn merge(&mut self, other: &Self) {
        self.toolchain.extend(other.toolchain.iter().cloned());
        self.files.extend(other.files.iter().cloned());
        self.globs.extend(other.globs.iter().cloned());
    }
}

/// What a resolve reported reading, before its manifests are expanded.
#[derive(Default)]
struct Reads {
    watch: Watch,
    /// Manifests and lockfiles a Cargo resolve read; expanded into their
    /// workspace roots, patch targets and member globs once the resolve
    /// finishes.
    manifests: BTreeSet<PathBuf>,
}

/// The toolchain observation shared by every fact a Cargo, rustup or
/// rustc process answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ToolchainInputs {
    /// The [`ENV_PREFIXES`] variables of the resolving host.
    env: BTreeMap<String, String>,
    /// `cargo`, `rustc` and `rustup` as the host's `PATH` resolves them.
    programs: BTreeMap<String, Option<Stamp>>,
    /// The identity of every installed rustup toolchain.
    toolchains: BTreeMap<String, ToolchainStamp>,
}

/// The full observation a record is valid for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Inputs {
    cli: CliIdentity,
    /// Present when the watch names a toolchain directory.
    toolchain: Option<ToolchainInputs>,
    /// The custom toolchain `path`s the toolchain files name.
    custom_toolchains: BTreeMap<PathBuf, ToolchainStamp>,
    /// Every path observed: the project's `Water.toml` and `Water.lock`,
    /// the Cargo configuration chains, rustup's settings and toolchain
    /// files, and the watch's files and globs.
    files: BTreeMap<PathBuf, Observed>,
}

/// An [`Inputs`] observation plus the newest modification time among the
/// files it hashed, in nanoseconds since the epoch.
struct Observation {
    inputs: Inputs,
    newest_ns: Option<u128>,
}

/// A persisted answer: a [`RecordHeader`] line, then this body. Only the
/// header is read before the rest — a record written by another CLI build
/// is replaced, never parsed further.
#[derive(Serialize, Deserialize)]
struct Record<Q, V> {
    kind: String,
    question: Q,
    watch: Watch,
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
    watch: Arc<Watch>,
    inputs: Inputs,
    value: Arc<V>,
}

type Slot = Arc<async_lock::Mutex<Option<Arc<dyn Any + Send + Sync>>>>;

/// A file observed in this invocation, under the metadata stamp it had
/// when it was read.
struct Seen {
    stamp: Option<FileStamp>,
    observed: Observed,
}

/// What one invocation observed: every file's content digest under the
/// stamp it was read at, the TOML documents among them parsed once per
/// content, and the toolchain identity.
#[derive(Default)]
struct Observer {
    files: HashMap<PathBuf, Seen>,
    tables: HashMap<String, Option<Arc<toml::Table>>>,
    toolchain: Option<ToolchainInputs>,
    /// The Cargo configuration files discovered from each directory, walked
    /// once per invocation.
    config_chains: HashMap<PathBuf, Arc<[PathBuf]>>,
    /// What each pattern matched, listed once per invocation.
    globs: HashMap<PathBuf, BTreeSet<PathBuf>>,
}

/// The files a blocking derivation reads, each one an input of what it
/// derives.
pub trait InputFiles {
    /// `path`'s bytes; `None` when nothing is there.
    ///
    /// # Errors
    /// Returns an error when the file exists but cannot be read.
    fn read(&mut self, path: &Path) -> io::Result<Option<Vec<u8>>>;

    /// The paths `pattern` matches.
    fn glob(&mut self, pattern: &Path) -> BTreeSet<PathBuf>;
}

/// The filesystem itself, for a derivation whose answer no fact records.
pub struct Disk;

impl InputFiles for Disk {
    fn read(&mut self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        Observer::read_bytes(path)
    }

    fn glob(&mut self, pattern: &Path) -> BTreeSet<PathBuf> {
        glob_matches(pattern)
    }
}

/// The invocation's observer, noting every read and glob of a
/// derivation as an input of the answer it derives.
struct Recorder {
    observer: Arc<Mutex<Observer>>,
    watch: Watch,
}

impl InputFiles for Recorder {
    fn read(&mut self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        self.watch.files.insert(path.to_path_buf());
        lock(&self.observer).read(path)
    }

    fn glob(&mut self, pattern: &Path) -> BTreeSet<PathBuf> {
        self.watch.globs.insert(pattern.to_path_buf());
        lock(&self.observer).glob(pattern)
    }
}

impl Observer {
    /// `path`'s content observation: replayed while its stamp is the one
    /// it was read under, read and hashed otherwise. The stamp is taken
    /// before the read, so a write racing the read leaves a stamp that no
    /// longer matches and the next observation reads again.
    fn hash(&mut self, path: &Path) -> io::Result<(Observed, Option<u128>)> {
        let stamp = FileStamp::of(path)?;
        let modified = stamp.map(|stamp| stamp.modified_ns);
        if let Some(seen) = self.files.get(path)
            && seen.stamp == stamp
        {
            return Ok((seen.observed.clone(), modified));
        }
        let bytes = match stamp {
            Some(_) => Self::read_bytes(path)?,
            None => None,
        };
        Ok((self.remember(path, stamp, bytes.as_deref()), modified))
    }

    /// `path`'s bytes, recorded as its observation; `None` when nothing is
    /// there.
    fn read(&mut self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        let stamp = FileStamp::of(path)?;
        let bytes = match stamp {
            Some(_) => Self::read_bytes(path)?,
            None => None,
        };
        self.remember(path, stamp, bytes.as_deref());
        Ok(bytes)
    }

    /// `path` parsed as TOML, once per content; `None` when it is absent
    /// or does not parse.
    fn table(&mut self, path: &Path) -> io::Result<Option<Arc<toml::Table>>> {
        let stamp = FileStamp::of(path)?;
        if let Some(Seen {
            stamp: seen,
            observed: Observed::Sha256(digest),
        }) = self.files.get(path)
            && *seen == stamp
            && let Some(table) = self.tables.get(digest)
        {
            return Ok(table.clone());
        }
        let bytes = match stamp {
            Some(_) => Self::read_bytes(path)?,
            None => None,
        };
        let observed = self.remember(path, stamp, bytes.as_deref());
        let Observed::Sha256(digest) = observed else {
            return Ok(None);
        };
        let table = bytes
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|text| text.parse::<toml::Table>().ok())
            .map(Arc::new);
        self.tables.insert(digest, table.clone());
        Ok(table)
    }

    /// The configuration files Cargo discovers from `root`, as `walk`
    /// finds them the first time this invocation asks.
    fn config_chain(
        &mut self,
        root: &Path,
        walk: impl FnOnce(&Path) -> Vec<PathBuf>,
    ) -> Arc<[PathBuf]> {
        Arc::clone(
            self.config_chains
                .entry(root.to_path_buf())
                .or_insert_with(|| walk(root).into()),
        )
    }

    /// What `pattern` matches, listed the first time this invocation asks.
    fn glob(&mut self, pattern: &Path) -> BTreeSet<PathBuf> {
        self.globs
            .entry(pattern.to_path_buf())
            .or_insert_with(|| glob_matches(pattern))
            .clone()
    }

    fn read_bytes(path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn remember(
        &mut self,
        path: &Path,
        stamp: Option<FileStamp>,
        bytes: Option<&[u8]>,
    ) -> Observed {
        let observed = bytes.map_or(Observed::Absent, |bytes| {
            Observed::Sha256(sha256_hex(bytes))
        });
        self.files.insert(
            path.to_path_buf(),
            Seen {
                stamp,
                observed: observed.clone(),
            },
        );
        observed
    }
}

/// The inputs one resolve reports. A fact's [`Fact::resolve`] receives it
/// and reports, through it, everything its answer is a function of.
pub struct Scope<'a> {
    facts: &'a Facts,
    reads: Mutex<Reads>,
}

impl Scope<'_> {
    /// The host the fact resolves on.
    pub(crate) const fn host(&self) -> &Host {
        &self.facts.host
    }

    /// The answer came from a Cargo, rustup or rustc process run in
    /// `directory`.
    pub(crate) fn toolchain(&self, directory: &Path) {
        self.with_reads(|reads| reads.watch.toolchain.insert(directory.to_path_buf()));
    }

    /// A Cargo resolve read these manifests and lockfiles.
    pub(crate) fn manifests(&self, paths: impl IntoIterator<Item = PathBuf>) {
        self.with_reads(|reads| reads.manifests.extend(paths));
    }

    /// The answer read `path` — or found nothing there.
    pub(crate) fn file(&self, path: &Path) {
        self.with_reads(|reads| reads.watch.files.insert(path.to_path_buf()));
    }

    /// Read `path`'s bytes as an input of the answer; `None` when nothing
    /// is there.
    ///
    /// # Errors
    /// Returns an error when the file exists but cannot be read.
    pub(crate) async fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        self.file(path);
        let observer = Arc::clone(&self.facts.observer);
        let path = path.to_path_buf();
        smol::unblock(move || lock(&observer).read(&path)).await
    }

    /// Run the blocking `derive` on the blocking pool, every file it reads
    /// and every pattern it lists through [`InputFiles`] an input of the
    /// answer.
    pub(crate) async fn derive<T: Send + 'static>(
        &self,
        derive: impl FnOnce(&mut dyn InputFiles) -> T + Send + 'static,
    ) -> T {
        let mut recorder = Recorder {
            observer: Arc::clone(&self.facts.observer),
            watch: Watch::default(),
        };
        let (value, watch) = smol::unblock(move || {
            let value = derive(&mut recorder);
            (value, recorder.watch)
        })
        .await;
        self.with_reads(|reads| reads.watch.merge(&watch));
        value
    }

    /// `fact`'s answer, its inputs joining this answer's.
    ///
    /// # Errors
    /// Returns an error when `fact` cannot be answered.
    pub(crate) async fn answer<F: Fact>(&self, fact: &F) -> eyre::Result<Arc<F::Value>> {
        let (value, watch) = Box::pin(self.facts.answer_watched(fact)).await?;
        self.with_reads(|reads| reads.watch.merge(&watch));
        Ok(value)
    }

    /// `cargo metadata` as `command` configures it, as an input of the
    /// answer.
    ///
    /// # Errors
    /// Returns an error when Cargo fails or its inputs cannot be observed.
    pub(crate) async fn cargo_metadata(&self, command: &MetadataCommand) -> eyre::Result<Metadata> {
        let fact = CargoMetadata::new(command, self.facts.host.cwd());
        let json = self.answer(&fact).await?;
        Ok(MetadataCommand::parse(json.as_str())?)
    }

    fn with_reads<T>(&self, f: impl FnOnce(&mut Reads) -> T) -> T {
        f(&mut self.reads.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The fact cache of one project.
///
/// Cloning shares the invocation's answers and observations; nothing is
/// process-global.
#[derive(Clone)]
pub struct Facts {
    host: Host,
    project_root: PathBuf,
    records: PathBuf,
    cli: CliIdentity,
    slots: Arc<async_lock::Mutex<BTreeMap<String, Slot>>>,
    observer: Arc<Mutex<Observer>>,
}

impl std::fmt::Debug for Facts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Facts")
            .field("project_root", &self.project_root)
            .field("records", &self.records)
            .finish_non_exhaustive()
    }
}

fn lock(observer: &Mutex<Observer>) -> std::sync::MutexGuard<'_, Observer> {
    observer.lock().unwrap_or_else(PoisonError::into_inner)
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
            observer: Arc::default(),
        })
    }

    /// These facts as a CLI with identity `cli` sees them.
    #[cfg(test)]
    pub(crate) fn with_cli(mut self, cli: CliIdentity) -> Self {
        self.cli = cli;
        self.slots = Arc::default();
        self.observer = Arc::default();
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
    pub(crate) async fn answer<F: Fact>(&self, fact: &F) -> eyre::Result<Arc<F::Value>> {
        Ok(self.answer_watched(fact).await?.0)
    }

    #[expect(
        clippy::significant_drop_tightening,
        reason = "the question's slot stays locked across the resolve, so concurrent askers of one question share a single resolve"
    )]
    async fn answer_watched<F: Fact>(&self, fact: &F) -> eyre::Result<(Arc<F::Value>, Arc<Watch>)> {
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
            && self.observe(&answer.watch).await?.inputs == answer.inputs
        {
            return Ok((Arc::clone(&answer.value), Arc::clone(&answer.watch)));
        }

        let path = self.records.join(format!("{key}.json"));
        if let Some(record) = read_record::<F>(&path, &self.cli).await? {
            if record.kind != F::KIND || record.question != question {
                eyre::bail!(
                    "the fact record {} answers another question than its name; remove it",
                    path.display()
                );
            }
            if self.observe(&record.watch).await?.inputs == record.inputs {
                tracing::debug!(fact = F::KIND, record = %path.display(), "fact replayed");
                let value = Arc::new(record.value);
                let watch = Arc::new(record.watch);
                *slot = Some(Arc::new(Answer {
                    watch: Arc::clone(&watch),
                    inputs: record.inputs,
                    value: Arc::clone(&value),
                }));
                return Ok((value, watch));
            }
        }

        let started_ns = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let toolchain_before = self.observe_toolchain().await?;
        let scope = Scope {
            facts: self,
            reads: Mutex::default(),
        };
        let value = fact.resolve(&scope).await?;
        let reads = scope
            .reads
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        let watch = Arc::new(self.expand(reads).await?);
        let toolchain_settled =
            watch.toolchain.is_empty() || self.observe_toolchain().await? == toolchain_before;
        let after = self.observe(&watch).await?;
        let settled = toolchain_settled
            && after
                .newest_ns
                .is_none_or(|newest| newest + MTIME_SLACK.as_nanos() < started_ns);
        if settled {
            write_record(
                &path,
                &self.cli,
                &Record {
                    kind: F::KIND.to_owned(),
                    question,
                    watch: watch.as_ref().clone(),
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
            watch: Arc::clone(&watch),
            inputs: after.inputs,
            value: Arc::clone(&value),
        }));
        Ok((value, watch))
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

    /// Close a resolve's reads: every manifest it reported joins the watch
    /// with the workspace root it inherits from — every ancestor manifest
    /// consulted to find that root included — and the root's `[patch]`
    /// targets and member globs, which reach a resolve without appearing
    /// in it.
    async fn expand(&self, reads: Reads) -> eyre::Result<Watch> {
        let observer = Arc::clone(&self.observer);
        smol::unblock(move || {
            let mut observer = lock(&observer);
            let Reads {
                mut watch,
                manifests,
            } = reads;
            let mut roots = BTreeSet::new();
            for manifest in manifests {
                if manifest.file_name() == Some(OsStr::new("Cargo.toml"))
                    && let Some(package_dir) = manifest.parent()
                    && let Some(root) =
                        workspace_manifest_of(&mut observer, package_dir, &mut watch.files)?
                {
                    roots.insert(root);
                }
                watch.files.insert(manifest);
            }
            for root in roots {
                watch.files.insert(root.clone());
                let (Some(table), Some(base)) = (observer.table(&root)?, root.parent()) else {
                    continue;
                };
                for path in patch_paths(&table, base) {
                    watch.files.insert(path.join("Cargo.toml"));
                }
                watch.globs.extend(member_globs(&table, base));
            }
            drop(observer);
            io::Result::Ok(watch)
        })
        .await
        .wrap_err("cannot observe the inputs of a resolved fact")
    }

    /// The toolchain identity as the host shows it now, remembered as this
    /// invocation's observation.
    async fn observe_toolchain(&self) -> eyre::Result<ToolchainInputs> {
        let probe = self.toolchain_probe();
        let toolchain = smol::unblock(move || probe.observe())
            .await
            .wrap_err("cannot observe the Rust toolchain")?;
        lock(&self.observer).toolchain = Some(toolchain.clone());
        Ok(toolchain)
    }

    fn toolchain_probe(&self) -> ToolchainProbe {
        ToolchainProbe {
            host: self.host.clone(),
            rustup_home: self.rustup_home(),
        }
    }

    fn rustup_home(&self) -> Option<PathBuf> {
        self.host.env("RUSTUP_HOME").map_or_else(
            || self.host.home_dir().map(|home| home.join(".rustup")),
            |home| Some(PathBuf::from(home)),
        )
    }

    /// Observe every input `watch` names, plus the project's own.
    async fn observe(&self, watch: &Watch) -> eyre::Result<Observation> {
        let toolchain = if watch.toolchain.is_empty() {
            None
        } else {
            let remembered = lock(&self.observer).toolchain.clone();
            match remembered {
                Some(toolchain) => Some(toolchain),
                None => Some(self.observe_toolchain().await?),
            }
        };
        let probe = Probe {
            cli: self.cli.clone(),
            toolchain,
            rustup_home: self.rustup_home(),
            cargo_home: self.host.env_string("CARGO_HOME"),
            home: self.host.home_dir().map(Path::to_path_buf),
            project_root: self.project_root.clone(),
            watch: watch.clone(),
        };
        let observer = Arc::clone(&self.observer);
        smol::unblock(move || probe.observe(&mut lock(&observer)))
            .await
            .wrap_err("cannot observe the inputs of a recorded fact")
    }
}

/// Cargo's `$CARGO_HOME`: absolute, anchored at `cwd` when relative,
/// `~/.cargo` when unset — the same resolution `cargo_config2` performs.
fn cargo_home(cargo_home: Option<&str>, home: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    cargo_home
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| {
            if home.is_absolute() {
                home
            } else {
                cwd.join(home)
            }
        })
        .or_else(|| home.map(|home| home.join(".cargo")))
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

/// What [`Facts::observe_toolchain`] gathers, moved to the blocking pool.
struct ToolchainProbe {
    host: Host,
    rustup_home: Option<PathBuf>,
}

impl ToolchainProbe {
    fn observe(self) -> io::Result<ToolchainInputs> {
        let env = self
            .host
            .envs()
            .filter_map(|(key, value)| {
                let key = key.to_str()?;
                ENV_PREFIXES
                    .iter()
                    .any(|prefix| key.starts_with(prefix))
                    .then(|| (key.to_owned(), value.to_string_lossy().into_owned()))
            })
            .collect();
        let mut programs = BTreeMap::new();
        for program in ["cargo", "rustc", "rustup"] {
            let stamp = self
                .host
                .which_blocking(program)
                .ok()
                .as_deref()
                .map(Stamp::of)
                .transpose()?
                .flatten();
            programs.insert(program.to_owned(), stamp);
        }
        let mut toolchains = BTreeMap::new();
        if let Some(rustup_home) = &self.rustup_home {
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
        Ok(ToolchainInputs {
            env,
            programs,
            toolchains,
        })
    }
}

/// Everything [`Facts::observe`] gathered on the executor, moved to the
/// blocking pool for the filesystem walk.
struct Probe {
    cli: CliIdentity,
    toolchain: Option<ToolchainInputs>,
    rustup_home: Option<PathBuf>,
    cargo_home: Option<String>,
    home: Option<PathBuf>,
    project_root: PathBuf,
    watch: Watch,
}

impl Probe {
    fn observe(self, observer: &mut Observer) -> io::Result<Observation> {
        let mut files = Files {
            observer,
            observed: BTreeMap::new(),
            newest_ns: None,
        };
        let mut custom_toolchains = BTreeMap::new();

        for name in ["Water.toml", "Water.lock"] {
            files.hash(&self.project_root.join(name))?;
        }

        for cwd in &self.watch.toolchain {
            // The configuration chains Cargo discovers from the resolving
            // directory and from the project root — a managed crate's
            // build carries the project's chain through `--config` — and
            // every `[patch]` path they declare.
            for root in [cwd, &self.project_root] {
                let chain = files.observer.config_chain(root, |root| {
                    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
                    let cargo_home =
                        cargo_home(self.cargo_home.as_deref(), self.home.as_deref(), &root);
                    config_candidates(&root, cargo_home.as_deref())
                });
                for config in chain.iter() {
                    files.hash(config)?;
                    if let Some(table) = files.observer.table(config)?
                        && let Some(base) = config.parent().and_then(Path::parent)
                    {
                        for path in patch_paths(&table, base) {
                            files.hash(&path.join("Cargo.toml"))?;
                        }
                    }
                }
            }

            // rustup's selection: its settings (default toolchain,
            // directory overrides) and the toolchain files from the
            // resolving directory up, with the custom toolchain a file
            // names by `path`.
            if let Some(rustup_home) = &self.rustup_home {
                files.hash(&rustup_home.join("settings.toml"))?;
            }
            for directory in cwd.ancestors() {
                for name in ["rust-toolchain", "rust-toolchain.toml"] {
                    let file = directory.join(name);
                    files.hash(&file)?;
                    if let Some(custom) = files
                        .observer
                        .table(&file)?
                        .as_deref()
                        .and_then(|table| table.get("toolchain"))
                        .and_then(|toolchain| toolchain.get("path"))
                        .and_then(toml::Value::as_str)
                    {
                        let custom = directory.join(custom);
                        let stamp = toolchain_stamps(&custom)?;
                        custom_toolchains.insert(custom, stamp);
                    }
                }
            }
        }

        for file in &self.watch.files {
            files.hash(file)?;
        }
        for pattern in &self.watch.globs {
            let matches = files.observer.glob(pattern);
            files
                .observed
                .insert(pattern.clone(), Observed::Matches(matches));
        }

        Ok(Observation {
            inputs: Inputs {
                cli: self.cli,
                toolchain: self.toolchain,
                custom_toolchains,
                files: files.observed,
            },
            newest_ns: files.newest_ns,
        })
    }
}

/// The content observations of one [`Probe`].
struct Files<'a> {
    observer: &'a mut Observer,
    observed: BTreeMap<PathBuf, Observed>,
    newest_ns: Option<u128>,
}

impl Files<'_> {
    fn hash(&mut self, path: &Path) -> io::Result<()> {
        if self.observed.contains_key(path) {
            return Ok(());
        }
        let (observed, modified) = self.observer.hash(path)?;
        if let Some(modified) = modified
            && observed != Observed::Absent
        {
            self.newest_ns = Some(
                self.newest_ns
                    .map_or(modified, |newest| newest.max(modified)),
            );
        }
        self.observed.insert(path.to_path_buf(), observed);
        Ok(())
    }
}

/// What `pattern` matches now; a pattern that does not parse matches
/// nothing.
fn glob_matches(pattern: &Path) -> BTreeSet<PathBuf> {
    glob::glob(&pattern.to_string_lossy())
        .map(|paths| paths.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Every path Cargo looks for configuration at from `root`: `config` and
/// `config.toml` under each ancestor's `.cargo` and under `$CARGO_HOME`.
/// A candidate that does not exist is an input too — creating it changes
/// what Cargo reads.
fn config_candidates(root: &Path, cargo_home: Option<&Path>) -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = root.ancestors().map(|dir| dir.join(".cargo")).collect();
    if let Some(cargo_home) = cargo_home
        && !directories.iter().any(|dir| dir == cargo_home)
    {
        directories.push(cargo_home.to_path_buf());
    }
    directories
        .iter()
        .flat_map(|dir| [dir.join("config"), dir.join("config.toml")])
        .collect()
}

/// The globs in `[workspace] members` under `base`: a directory appearing
/// under one joins the workspace without any manifest or lockfile
/// changing.
fn member_globs(table: &toml::Table, base: &Path) -> Vec<PathBuf> {
    table
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .filter(|pattern| pattern.contains(['*', '?', '[']))
        .map(|pattern| base.join(pattern))
        .collect()
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
/// `[workspace]` table. Every manifest path consulted — absent ones
/// included — joins `consulted`: one appearing or changing can move the
/// root.
fn workspace_manifest_of(
    observer: &mut Observer,
    package_dir: &Path,
    consulted: &mut BTreeSet<PathBuf>,
) -> io::Result<Option<PathBuf>> {
    for ancestor in package_dir.ancestors() {
        let manifest_path = ancestor.join("Cargo.toml");
        consulted.insert(manifest_path.clone());
        let Some(manifest) = observer.table(&manifest_path)? else {
            continue;
        };
        if ancestor == package_dir
            && let Some(pointer) = manifest
                .get("package")
                .and_then(|package| package.get("workspace"))
                .and_then(toml::Value::as_str)
            && let Ok(root) = dunce::canonicalize(ancestor.join(pointer))
        {
            return Ok(Some(root.join("Cargo.toml")));
        }
        if manifest.contains_key("workspace") {
            return Ok(Some(manifest_path));
        }
    }
    Ok(None)
}

/// `cargo metadata` with the arguments, working directory and environment
/// overrides a [`MetadataCommand`] carries.
struct CargoMetadata<'a> {
    command: &'a MetadataCommand,
    question: CargoMetadataQuestion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        self.question.clone()
    }

    /// Cargo reads every path package's manifest, the manifest behind every
    /// `path` dependency any of them declares — optional and other-target
    /// ones included, which a resolve reads without listing — and the
    /// workspace's manifest and lockfile.
    async fn resolve(&self, scope: &Scope<'_>) -> eyre::Result<String> {
        scope.toolchain(&self.question.cwd);
        let json = scope.host().cargo_metadata_json(self.command).await?;
        let metadata = MetadataCommand::parse(&json)?;
        let workspace_root = metadata.workspace_root.as_std_path();
        scope.manifests([
            workspace_root.join("Cargo.toml"),
            workspace_root.join("Cargo.lock"),
        ]);
        for package in metadata
            .packages
            .iter()
            .filter(|package| package.source.is_none())
        {
            scope.manifests(
                std::iter::once(package.manifest_path.clone().into_std_path_buf()).chain(
                    package
                        .dependencies
                        .iter()
                        .filter_map(|dependency| dependency.path.as_ref())
                        .map(|path| path.as_std_path().join("Cargo.toml")),
                ),
            );
        }
        Ok(json)
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

    async fn resolve(&self, scope: &Scope<'_>) -> eyre::Result<String> {
        scope.toolchain(&self.cwd);
        Ok(crate::toolchain::rust::selected_rustup_toolchain(
            &scope.host().clone().with_cwd(&self.cwd),
        )
        .await?)
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

    async fn resolve(&self, scope: &Scope<'_>) -> eyre::Result<PathBuf> {
        scope.toolchain(&self.cwd);
        let output = scope
            .host()
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

    use super::{CliIdentity, Fact, Facts, RECORD_FORMAT, Scope};
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

    /// A fact derived from files alone: the manifests under the fixture
    /// workspace's member glob, each resolve logged as [`DERIVED`].
    struct MemberManifests {
        root: PathBuf,
        log: PathBuf,
    }

    impl Fact for MemberManifests {
        const KIND: &'static str = "test-member-manifests";
        type Question = PathBuf;
        type Value = Vec<String>;

        fn question(&self) -> PathBuf {
            self.root.clone()
        }

        async fn resolve(&self, scope: &Scope<'_>) -> eyre::Result<Vec<String>> {
            let (root, log) = (self.root.clone(), self.log.clone());
            scope
                .derive(move |files| {
                    use std::io::Write as _;
                    writeln!(
                        std::fs::OpenOptions::new().append(true).open(&log)?,
                        "{DERIVED}"
                    )?;
                    let mut manifests = Vec::new();
                    for path in files.glob(&root.join("ws/crates/*/Cargo.toml")) {
                        if let Some(bytes) = files.read(&path)? {
                            manifests.push(String::from_utf8_lossy(&bytes).into_owned());
                        }
                    }
                    Ok(manifests)
                })
                .await
        }
    }

    /// Ask the kinds of fact a build asks: the project's metadata, the
    /// toolchain its directory selects, a target's libdir and an answer
    /// derived from files.
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
        facts
            .answer(&MemberManifests {
                root: fixture.root.clone(),
                log: fixture.log.clone(),
            })
            .await?;
        Ok(())
    }

    /// An input change, the edit that makes it, and the facts it must
    /// re-resolve.
    type Change = (&'static str, Box<dyn Fn(&Fixture)>, &'static [&'static str]);

    const METADATA: &str = "cargo metadata";
    const DERIVED: &str = "derived member manifests";
    /// The facts a toolchain process answers.
    const ALL: [&str; 3] = [
        "cargo metadata",
        "rustup show active-toolchain",
        "rustc --print target-libdir",
    ];
    /// Every fact [`ask`] asks.
    const EVERY: [&str; 4] = [
        "cargo metadata",
        "rustup show active-toolchain",
        "rustc --print target-libdir",
        DERIVED,
    ];

    /// The facts a run of [`ask`] resolved, by log prefix, since `seen`
    /// lines.
    fn spawned_since(fixture: &Fixture, seen: usize) -> Vec<&'static str> {
        let spawns = fixture.spawns();
        EVERY
            .into_iter()
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
            assert_eq!(spawned_since(&fixture, 0), EVERY, "the first ask resolves");
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

    /// The same binary invoked through another spelling of its path —
    /// a `.` or `..` component, or a symlink — is the same CLI, so a
    /// fresh invocation through it replays the records instead of
    /// resolving again.
    #[test]
    fn an_invocation_through_another_path_spelling_replays_the_records() {
        smol::block_on(async {
            let fixture = Fixture::new();
            let host = fixture.host(&[]);
            ask(&fixture, &fixture.facts(&host))
                .await
                .expect("the facts resolve");
            let seen = fixture.spawns().len();

            let exe = std::env::current_exe().expect("the test binary has a path");
            let directory = exe.parent().expect("the binary has a directory");
            let name = exe.file_name().expect("the binary has a name");
            let spellings = [
                directory.join(".").join(name),
                directory
                    .join("..")
                    .join(directory.file_name().expect("the directory has a name"))
                    .join(name),
            ];
            #[cfg(unix)]
            let linked = {
                fixture.machine.dir("linked-bin");
                let link = fixture.path("linked-bin/water");
                std::os::unix::fs::symlink(&exe, &link).expect("the symlink is created");
                Some(link)
            };
            #[cfg(not(unix))]
            let linked: Option<PathBuf> = None;

            for spelling in spellings.into_iter().chain(linked) {
                let facts = fixture
                    .facts(&host)
                    .with_cli(CliIdentity::invoked_as(&spelling));
                ask(&fixture, &facts)
                    .await
                    .expect("the facts replay under the respelled invocation");
            }
            assert_eq!(
                fixture.spawns().len(),
                seen,
                "a respelled invocation resolves nothing: {:?}",
                fixture.spawns()
            );
        });
    }

    /// Each input change, and which facts it must re-resolve: the inputs
    /// of a resolve re-resolve only the fact read from them; the
    /// configuration and toolchain inputs every toolchain fact shares
    /// re-resolve those three, and the project's own inputs every fact.
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
                &[METADATA, DERIVED],
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
                &EVERY,
            ),
            (
                "a new Water.lock",
                Box::new(|f| f.write("ws/app/Water.lock", "version = 1\n")),
                &EVERY,
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
            assert_eq!(spawned_since(&fixture, seen), EVERY, "the CLI changed");
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
            let records = std::fs::read_dir(fixture.path(&format!("records/v{RECORD_FORMAT}")))
                .expect("records exist")
                .map(|entry| entry.expect("record entry").path())
                .collect::<Vec<_>>();
            assert_eq!(records.len(), EVERY.len(), "one record per fact");
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
