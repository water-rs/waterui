#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# ///
"""Report the GPU adapters visible inside the bench container as JSON.

Adapter inventory comes from `vulkaninfo --summary` (vulkan-tools, pinned
in docker/Dockerfile) — the maintained probe the image already carries —
and DRI render nodes are mapped to adapters through sysfs so the runner
can restrict /dev/dri mounts to one adapter when it must attribute "the
renderer actually used" rather than just "any adapter present".

Output is one JSON document:
    {"vulkan": [{name, type, vendor, device, uuid, dri}],
     "dri":     ["/dev/dri/renderD128", ...]}

Exits non-zero with a message when vulkaninfo fails or reports no GPU —
a probe that cannot enumerate adapters must never silently pass the
software-adapter guard.

`--self-test` parses the committed fixtures instead of probing the host;
it is the CPU-only correctness check (no GPU work on software devices).
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent / "fixtures"

GPU_BLOCK = re.compile(
    r"^GPU(?P<idx>\d+):\s*\n"
    r"(?P<body>(?:^[ \t]+.*\n?)+)",
    re.MULTILINE,
)
FIELD = re.compile(r"^\s*(?P<k>\w+)\s*=\s*(?P<v>.*?)\s*$", re.MULTILINE)


def parse_vulkan_summary(text: str) -> list[dict]:
    """One dict per `GPU<n>:` block in `vulkaninfo --summary` output."""
    out = []
    for m in GPU_BLOCK.finditer(text):
        f = {mm.group("k"): mm.group("v")
             for mm in FIELD.finditer(m.group("body"))}
        out.append({
            "name": f.get("deviceName", "?"),
            "type": (f.get("deviceType", "")
                     .removeprefix("PHYSICAL_DEVICE_TYPE_").lower() or "?"),
            "vendor": f.get("vendorID", "?"),
            "device": f.get("deviceID", "?"),
            "uuid": f.get("deviceUUID", ""),
        })
    return out


def dri_nodes() -> list[dict]:
    """/dev/dri render nodes with the vendor/device ids sysfs reports."""
    nodes = []
    for node in sorted(Path("/dev/dri").glob("renderD*")):
        sysdir = Path("/sys/class/drm") / node.name / "device"
        vendor = _read(sysdir / "vendor")
        device = _read(sysdir / "device")
        nodes.append({"node": str(node), "vendor": vendor, "device": device,
                      "pci": _pci_addr(sysdir)})
    return nodes


def _read(p: Path) -> str:
    try:
        return p.read_text().strip()
    except OSError:
        return ""


def _pci_addr(sysdir: Path) -> str:
    """PCI BDF (0000:03:00.0) the render node's device symlink resolves to."""
    try:
        return sysdir.resolve().name
    except OSError:
        return ""


def probe() -> dict:
    q = subprocess.run(
        ["vulkaninfo", "--summary"],
        capture_output=True, text=True,
        env={"PATH": "/usr/bin:/bin", "XDG_RUNTIME_DIR": "/tmp"},
        timeout=60,
    )
    if q.returncode != 0:
        raise SystemExit(
            f"vulkaninfo --summary failed ({q.returncode}): "
            f"{q.stderr.strip() or q.stdout.strip()}")
    adapters = parse_vulkan_summary(q.stdout)
    if not adapters:
        raise SystemExit(
            "vulkaninfo --summary reported no GPU devices — probe cannot "
            "establish which renderer a measurement would use")
    nodes = dri_nodes()
    # Attribute a render node to an adapter by vendor/device id; CPU-type
    # software rasterizers (llvmpipe/lavapipe) have no PCI device and match
    # nothing — by design, they can never be the mounted adapter.
    for a in adapters:
        match = [n for n in nodes
                 if n["vendor"] == a["vendor"] and n["device"] == a["device"]]
        a["dri"] = [n["node"] for n in match]
        a["pci"] = match[0]["pci"] if match else ""
    return {"vulkan": adapters, "dri": [n["node"] for n in nodes]}


def self_test() -> None:
    """Parse committed/synthesized summaries; verify classification."""
    lavapipe = (FIXTURES / "vulkaninfo-lavapipe.txt").read_text()
    a = parse_vulkan_summary(lavapipe)
    assert len(a) == 1, f"expected 1 adapter, got {a}"
    assert a[0]["type"] == "cpu", a
    assert "llvmpipe" in a[0]["name"], a

    # Multi-adapter variant: same fixture, one block rewritten to a
    # discrete GPU plus the untouched lavapipe block — checks the parser
    # yields every adapter, not just the first.
    hw = lavapipe.replace("PHYSICAL_DEVICE_TYPE_CPU",
                          "PHYSICAL_DEVICE_TYPE_DISCRETE_GPU") \
                 .replace("llvmpipe (LLVM 19.1.7, 256 bits)",
                          "Radeon RX 7900 XT (RADV NAVI31)") \
                 .replace("0x10005", "0x1002").replace("0x0000", "0x744e")
    multi = hw + "\n" + lavapipe
    a = parse_vulkan_summary(multi)
    assert len(a) == 2, f"expected 2 adapters, got {a}"
    assert a[0]["type"] == "discrete_gpu" and a[1]["type"] == "cpu", a
    print("self-test ok: lavapipe + multi-adapter fixtures parse correctly")


def main() -> None:
    if "--self-test" in sys.argv[1:]:
        self_test()
        return
    print(json.dumps(probe(), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
