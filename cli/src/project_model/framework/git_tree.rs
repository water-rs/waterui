//! Git-owned channel facts, read from one fetched object database.

use eyre::{Result, WrapErr as _, bail, eyre};

use crate::toolchain::Host;

#[derive(Debug)]
pub(super) struct FrameworkTree {
    directory: tempfile::TempDir,
    pub(super) revision: String,
}

impl FrameworkTree {
    pub(super) async fn fetch(host: &Host, repository: &str, revision: &str) -> Result<Self> {
        super::validate_rev(revision)?;
        let directory = tempfile::Builder::new()
            .prefix("waterui-framework-")
            .tempdir_in(host.temp_dir())?;
        let git = host.clone().with_cwd(directory.path());
        git.run("git", ["init", "-q"]).await?;
        git.run("git", ["remote", "add", "origin", repository])
            .await?;
        if revision.len() == 40 {
            git.run("git", ["fetch", "-q", "--depth=1", "origin", revision])
                .await
                .wrap_err_with(|| {
                    format!("could not fetch revision {revision} from {repository}")
                })?;
        } else {
            // Upload-pack accepts full object IDs, not abbreviations. The
            // complete dev history is needed to resolve an abbreviated pin.
            git.run(
                "git",
                [
                    "fetch",
                    "-q",
                    "origin",
                    "refs/heads/dev:refs/remotes/origin/dev",
                ],
            )
            .await?;
        }
        let revision = git
            .run(
                "git",
                ["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
            )
            .await
            .wrap_err_with(|| {
                format!("--rev {revision} does not name a unique commit in {repository}")
            })?
            .trim()
            .to_owned();
        super::validate_revision(&revision)?;
        Ok(Self {
            directory,
            revision,
        })
    }

    pub(super) async fn ensure_dev_ancestor(&self, host: &Host) -> Result<()> {
        let git = host.clone().with_cwd(self.directory.path());
        let shallow = git
            .run("git", ["rev-parse", "--is-shallow-repository"])
            .await?;
        let shallow = match shallow.trim() {
            "true" => true,
            "false" => false,
            value => bail!("git returned an invalid shallow-repository state: {value}"),
        };
        let mut args = vec!["fetch", "-q"];
        if shallow {
            args.push("--depth=1");
        }
        args.extend(["origin", "refs/heads/dev:refs/remotes/origin/dev"]);
        git.run("git", args).await?;
        let head = git
            .run(
                "git",
                ["rev-parse", "--verify", "refs/remotes/origin/dev^{commit}"],
            )
            .await?;
        let head = head.trim();
        super::validate_revision(head)?;
        if self.is_ancestor(&git, head).await? {
            return Ok(());
        }
        if shallow {
            // A shallow boundary can hide a real ancestor. Only a complete
            // history can prove rejection; do not treat exit 1 as final yet.
            git.run(
                "git",
                ["fetch", "-q", "--unshallow", "origin", "refs/heads/dev"],
            )
            .await?;
            if self.is_ancestor(&git, head).await? {
                return Ok(());
            }
        }
        bail!(
            "--rev {} is not an ancestor of the framework's dev head {head}",
            self.revision
        )
    }

    async fn is_ancestor(&self, git: &Host, head: &str) -> Result<bool> {
        let output = git
            .output("git", ["merge-base", "--is-ancestor", &self.revision, head])
            .await?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => bail!(
                "git ancestry check failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        }
    }

    pub(super) async fn file(&self, host: &Host, path: &str) -> Result<Vec<u8>> {
        let output = host
            .clone()
            .with_cwd(self.directory.path())
            .output("git", ["show", &format!("{}:{path}", self.revision)])
            .await?;
        if !output.status.success() {
            bail!(
                "could not read {path} at {}: {}",
                self.revision,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(output.stdout)
    }

    async fn entry(&self, host: &Host, path: &str) -> Result<Option<(String, String)>> {
        let output = host
            .clone()
            .with_cwd(self.directory.path())
            .run(
                "git",
                [
                    "--literal-pathspecs",
                    "ls-tree",
                    "-z",
                    &self.revision,
                    "--",
                    path,
                ],
            )
            .await?;
        let entry = output.strip_suffix('\0').unwrap_or(&output);
        if entry.contains('\0') {
            bail!("git returned multiple tree entries for {path}");
        }
        let header = entry.split_once('\t').map_or(entry, |(header, _)| header);
        let fields: Vec<_> = header.split_whitespace().collect();
        match fields.as_slice() {
            [] => Ok(None),
            [_, kind, object] => {
                super::validate_revision(object)?;
                Ok(Some(((*kind).to_owned(), (*object).to_owned())))
            }
            _ => Err(eyre!("invalid git tree entry for {path}: {output}")),
        }
    }

    pub(super) async fn optional_file(&self, host: &Host, path: &str) -> Result<Option<Vec<u8>>> {
        match self.entry(host, path).await? {
            None => Ok(None),
            Some((kind, _)) if kind == "blob" => self.file(host, path).await.map(Some),
            Some((kind, _)) => bail!("expected a file at {path}, found a git {kind}"),
        }
    }

    pub(super) async fn submodule_pin(&self, host: &Host, path: &str) -> Result<Option<String>> {
        Ok(self
            .entry(host, path)
            .await?
            .and_then(|(kind, object)| (kind == "commit").then_some(object)))
    }
}
