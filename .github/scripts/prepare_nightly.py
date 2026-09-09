import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def prepare():
    revision = git("rev-parse", "HEAD")
    if revision != os.environ["GITHUB_SHA"]:
        raise RuntimeError("The checkout is not the revision tested by this run")
    repository = os.environ["GITHUB_REPOSITORY"]
    date = git("show", "-s", "--format=%cs", "HEAD").replace("-", "")
    tag = f"nightly-{date}-{revision[:12]}"
    pages = json.loads(subprocess.check_output([
        "gh", "api", f"repos/{repository}/releases", "--paginate", "--slurp",
    ], text=True))
    releases = [
        release for page in pages for release in page
        if not release["draft"] and release["prerelease"]
        and re.fullmatch(r"nightly-\d{8}-[0-9a-f]{12}", release["tag_name"])
    ]
    if any(release["tag_name"] == tag for release in releases):
        return {"eligible": "false", "reason": "This revision is already certified"}
    if releases:
        latest = max(releases, key=lambda release: release["published_at"])
        latest_revision = git("rev-parse", f'{latest["tag_name"]}^{{commit}}')
        comparison = subprocess.run([
            "git", "merge-base", "--is-ancestor", latest_revision, revision,
        ])
        if comparison.returncode == 1:
            return {"eligible": "false", "reason": "This run does not advance the certified revision"}
        comparison.check_returncode()
    submodules = {}
    for line in subprocess.check_output(["git", "submodule", "status", "--recursive"], text=True).splitlines():
        if not line.startswith(" "):
            raise RuntimeError(f"Submodule is not at its recorded revision: {line}")
        commit, path, *_ = line.split()
        submodules[path] = commit
    lockfiles = {}
    for path in [Path("Cargo.lock"), *(Path(path) / "Cargo.lock" for path in submodules)]:
        if path.is_file():
            lockfiles[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
    scaffold = tomllib.loads(Path("cli/Cargo.toml").read_text())["package"]["metadata"]["waterui-scaffold"]
    manifest = {
        "schema_version": 1,
        "channel": "nightly",
        "repository": repository,
        "revision": revision,
        "tag": tag,
        "run_id": int(os.environ["GITHUB_RUN_ID"]),
        "run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"]),
        "submodules": submodules,
        "lockfiles": lockfiles,
        "scaffold": scaffold,
    }
    Path("framework.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return {"eligible": "true", "tag": tag, "revision": revision}


if __name__ == "__main__":
    outputs = prepare()
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        for key, value in outputs.items():
            output.write(f"{key}={value}\n")
    if outputs["eligible"] == "false":
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary:
            summary.write(outputs["reason"] + "\n")
