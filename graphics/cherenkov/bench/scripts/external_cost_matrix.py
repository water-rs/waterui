"""Shared external-cost matrix (#168).

Device drivers keep transport and thermal probing. The grid, the
argument list and the report checks live here.
"""

CELLS = (("1080p", "sdr"), ("1080p", "pq"), ("4k", "sdr"), ("4k", "pq"))
# ABAB, then the reversed BABA control.
ORDERS = (("e", "c", "e", "c"), ("c", "e", "c", "e"))
FRAMES = 240
WARMUP = 30
RATE = 120
OK_THERMAL = ("nominal", "fair")

_SIZE = {"1080p": (1920, 1080), "4k": (3840, 2160)}
_HEX = set("0123456789abcdef")


def order_name(order):
    """`abab` or `baba` for one of `ORDERS`."""
    if order == ORDERS[0]:
        return "abab"
    if order == ORDERS[1]:
        return "baba"
    raise AssertionError(order)


def args_for(path, size, transfer, out):
    """One `external-cost` argument list. `out` is the report path."""
    return [
        "external-cost",
        "--path",
        path,
        "--size",
        size,
        "--transfer",
        transfer,
        "--frames",
        str(FRAMES),
        "--warmup",
        str(WARMUP),
        "--rate",
        str(RATE),
        "--out",
        out,
    ]


def verify(report, size, transfer, path):
    """The report is this cell: every required stamp present, none invented.

    A missing stamp is a dropped frame. Zero is not a stand-in for one,
    so `dropped_frames` must be 0 and every sample's required timestamps
    must be present.
    """
    want_path = {"e": "external", "c": "copy-convert"}[path]
    want_layout = {"sdr": "nv12", "pq": "p010"}[transfer]
    want_transfer = "bt709-sdr" if transfer == "sdr" else "bt2020-pq"
    if report.get("path") != want_path:
        raise AssertionError(report.get("path"))
    if report.get("layout") != want_layout:
        raise AssertionError(report.get("layout"))
    if report.get("transfer") != want_transfer:
        raise AssertionError(report.get("transfer"))
    got = (report.get("width"), report.get("height"))
    if got != _SIZE[size]:
        raise AssertionError(got)
    if report.get("measured_frames") != FRAMES:
        raise AssertionError(report.get("measured_frames"))
    samples = report.get("samples")
    if not isinstance(samples, list) or len(samples) != FRAMES:
        raise AssertionError("sample count {}".format(
            None if not isinstance(samples, list) else len(samples)
        ))
    if report.get("dropped_frames") != 0:
        raise AssertionError("dropped_frames {}".format(report.get("dropped_frames")))
    for index, sample in enumerate(samples):
        if sample.get("gpu_seconds") is None:
            raise AssertionError("sample {} missing gpu_seconds".format(index))
        if path == "c" and (
            sample.get("handoff_seconds") is None
            or sample.get("convert_seconds") is None
        ):
            raise AssertionError("sample {} missing path-c stamps".format(index))
    total = report.get("total_seconds")
    if (
        not isinstance(total, list)
        or len(total) != 3
        or any(item is None for item in total)
    ):
        raise AssertionError("total_seconds {}".format(total))
    sha = report.get("git_sha")
    if not isinstance(sha, str) or len(sha) != 40 or any(c not in _HEX for c in sha):
        raise AssertionError("git_sha {}".format(sha))
    form = report.get("import_form")
    if path == "e":
        if form not in ("planes", "external-format", "rgb"):
            raise AssertionError("import_form {}".format(form))
    elif form is not None:
        raise AssertionError("path c import_form {}".format(form))
    return report
