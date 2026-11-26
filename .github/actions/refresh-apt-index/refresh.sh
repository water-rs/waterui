#!/usr/bin/env bash
# Refreshes the package index for Ubuntu's own archive and nothing else; see
# action.yml beside this file for why.
set -euo pipefail
if [ -f /etc/apt/sources.list.d/ubuntu.sources ]; then
  list=/etc/apt/sources.list.d/ubuntu.sources
elif [ -s /etc/apt/sources.list ]; then
  list=/etc/apt/sources.list
else
  echo "::error::no Ubuntu archive source file on this runner"
  exit 1
fi
# The hosted runner's mirror list selects Azure's archive, whose cold
# downloads have held setup for over 40 minutes (#3439). Resolve package
# indexes and archives through Ubuntu's official HTTPS endpoints, routing
# `*-security` suites to `security.ubuntu.com` so freshly published security
# packages do not 404 while `archive.ubuntu.com` mirrors are still syncing.
sudo python3 - "$list" <<'PY'
import pathlib
import re
import sys

ARCHIVE_URI = "https://archive.ubuntu.com/ubuntu"
SECURITY_URI = "https://security.ubuntu.com/ubuntu"
UBUNTU_URI_RE = re.compile(
    r"^(?:mirror\+file:/etc/apt/apt-mirrors(?:-security)?\.txt"
    r"|https?://(?:(?:azure\.)?archive|security)\.ubuntu\.com/ubuntu/?)$"
)
LIST_LINE_RE = re.compile(
    r"^(?P<prefix>\s*deb(?:-src)?\s+(?:\[[^\]]+\]\s+)?)"
    r"(?P<uri>\S+)"
    r"(?P<sep>\s+)"
    r"(?P<suite>\S+)"
    r"(?P<rest>.*)$"
)


def rewrite_list(text: str) -> str:
    out: list[str] = []
    for line in text.splitlines(keepends=True):
        nl = "\n" if line.endswith("\n") else ""
        body = line[:-1] if nl else line
        match = LIST_LINE_RE.match(body)
        if match and UBUNTU_URI_RE.match(match.group("uri")):
            uri = SECURITY_URI if match.group("suite").endswith("-security") else ARCHIVE_URI
            body = f"{match.group('prefix')}{uri}{match.group('sep')}{match.group('suite')}{match.group('rest')}"
        out.append(body + nl)
    return "".join(out)


def parse_stanza_fields(stanza: list[str]) -> list[tuple[str | None, list[str]]]:
    entries: list[tuple[str | None, list[str]]] = []
    for line in stanza:
        if line.startswith("#"):
            entries.append((None, [line]))
        elif line[:1] in (" ", "\t") and entries and entries[-1][0] is not None:
            entries[-1][1].append(line)
        elif ":" in line:
            key = line.split(":", 1)[0].strip().lower()
            entries.append((key, [line]))
        else:
            entries.append((None, [line]))
    return entries


def field_tokens(lines: list[str]) -> list[str]:
    first = lines[0].split(":", 1)[1]
    return " ".join([first, *lines[1:]]).split()


def render_stanza(
    entries: list[tuple[str | None, list[str]]],
    uri: str,
    suites: list[str] | None,
) -> list[str]:
    rendered: list[str] = []
    for key, lines in entries:
        if key == "uris":
            rendered.append(f"URIs: {uri}\n")
        elif key == "suites" and suites is not None:
            rendered.append(f"Suites: {' '.join(suites)}\n")
        else:
            rendered.extend(lines)
    return rendered


def rewrite_stanza(stanza: list[str]) -> list[list[str]]:
    entries = parse_stanza_fields(stanza)
    uris = [tok for key, lines in entries if key == "uris" for tok in field_tokens(lines)]
    if not uris or not all(UBUNTU_URI_RE.match(u) for u in uris):
        return [stanza]
    suites = [tok for key, lines in entries if key == "suites" for tok in field_tokens(lines)]
    security_suites = [s for s in suites if s.endswith("-security")]
    other_suites = [s for s in suites if not s.endswith("-security")]
    if security_suites and other_suites:
        return [
            render_stanza(entries, ARCHIVE_URI, other_suites),
            render_stanza(entries, SECURITY_URI, security_suites),
        ]
    if security_suites:
        return [render_stanza(entries, SECURITY_URI, None)]
    if other_suites:
        return [render_stanza(entries, ARCHIVE_URI, None)]
    is_security_uri = any("security" in u for u in uris)
    return [render_stanza(entries, SECURITY_URI if is_security_uri else ARCHIVE_URI, None)]


def rewrite_sources(text: str) -> str:
    out: list[str] = []
    stanza: list[str] = []

    def flush() -> None:
        if not stanza:
            return
        rewritten = rewrite_stanza(stanza)
        for index, block in enumerate(rewritten):
            if index > 0:
                if out and not out[-1].endswith("\n"):
                    out.append("\n")
                out.append("\n")
            out.extend(block)
        stanza.clear()

    for line in text.splitlines(keepends=True):
        if line.strip() == "":
            flush()
            out.append(line)
        else:
            stanza.append(line)
    flush()
    return "".join(out)


path = pathlib.Path(sys.argv[1])
original = path.read_text(encoding="utf-8")
updated = rewrite_sources(original) if path.suffix == ".sources" else rewrite_list(original)
if updated != original:
    path.write_text(updated, encoding="utf-8")
PY
# A stalled mirror connection otherwise holds the step until the job's own
# timeout; each fetch gives up after 30 s and the whole refresh after 5 min.
# `--error-on=any` fails the refresh when an index cannot be fetched; apt
# otherwise exits 0 with an empty index, and the job dies later at the
# install with "Unable to locate package".
if ! timeout 300 sudo apt-get update --error-on=any \
  -o Acquire::Retries=10 \
  -o Acquire::http::Timeout=30 \
  -o Acquire::https::Timeout=30 \
  -o Dir::Etc::SourceList="$list" \
  -o Dir::Etc::SourceParts=/dev/null; then
  echo "::error::refreshing the Ubuntu apt index failed or exceeded its 5 minute deadline"
  exit 1
fi
