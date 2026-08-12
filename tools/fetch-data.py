#!/usr/bin/env python3
"""Build src/features.json from the real web-features dataset.

baselinefit ships a snapshot of Baseline data so that an audit needs no network
access. This script is the only way that snapshot is allowed to change: it
downloads the published `web-features` npm package, reads `package/data.json`
out of the tarball, and copies the Baseline fields verbatim for exactly the
feature ids named in tools/detectors.json.

Nothing about a feature's name, status or dates is ever written by hand.
tools/detectors.json holds only the text patterns baselinefit greps for; if an
id in it is missing from the dataset, or has been superseded by a "moved" or
"split" record, this script fails rather than inventing an entry.

Usage:
    python3 tools/fetch-data.py                 # latest published version
    python3 tools/fetch-data.py --version 3.34.3
    python3 tools/fetch-data.py --check         # fail if src/features.json is stale

Standard library only; no pip install required.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import io
import json
import pathlib
import shutil
import ssl
import subprocess
import sys
import tarfile
import urllib.error
import urllib.request

REGISTRY = "https://registry.npmjs.org/web-features"
PACKAGE = "web-features"
SCHEMA = 1

ROOT = pathlib.Path(__file__).resolve().parent.parent
DETECTORS_PATH = ROOT / "tools" / "detectors.json"
OUTPUT_PATH = ROOT / "src" / "features.json"

# Detector kinds src/matcher.rs understands. Kept in step with MatchKind.
KINDS = {
    "at-rule",
    "property",
    "property-stem",
    "declaration",
    "selector",
    "function",
    "unit",
    "ident",
    "package",
    "supports",
    "supports-selector",
}
DIRECTIONS = {"risk", "removable"}
WHENS = {"satisfied", "unsatisfied", "always"}


def fetch(url: str) -> bytes:
    """Download `url`.

    urllib is tried first because it is available everywhere. Some Python
    installations (notably the python.org macOS builds before
    `Install Certificates.command` has been run) have no CA bundle and fail the
    TLS handshake; on those, fall back to curl, which uses the system trust
    store. Certificate verification is never disabled.
    """
    try:
        with urllib.request.urlopen(url, timeout=120) as response:  # noqa: S310
            return response.read()
    except urllib.error.URLError as err:
        if not isinstance(err.reason, ssl.SSLError):
            raise
        curl = shutil.which("curl")
        if curl is None:
            raise
        print(f"urllib TLS failed ({err.reason}); retrying with curl", file=sys.stderr)
        done = subprocess.run(  # noqa: S603
            [curl, "--fail", "--silent", "--show-error", "--location", "--max-time", "180", url],
            capture_output=True,
            check=True,
        )
        return done.stdout


def resolve_version(requested: str | None) -> tuple[str, str]:
    """Return (version, tarball url)."""
    suffix = "/latest" if requested is None else f"/{requested}"
    meta = json.loads(fetch(REGISTRY + suffix))
    return meta["version"], meta["dist"]["tarball"]


def load_dataset(tarball_url: str) -> tuple[dict, str]:
    """Return (data.json contents, sha256 of the tarball)."""
    blob = fetch(tarball_url)
    digest = hashlib.sha256(blob).hexdigest()
    with tarfile.open(fileobj=io.BytesIO(blob), mode="r:gz") as tar:
        member = tar.extractfile("package/data.json")
        if member is None:
            raise SystemExit("package/data.json is missing from the tarball")
        return json.load(member), digest


def validate_detectors(feature_id: str, spec: dict) -> None:
    detectors = spec.get("detectors")
    if not detectors:
        raise SystemExit(f"{feature_id}: no detectors")
    seen_directions = set()
    for detector in detectors:
        direction = detector.get("direction")
        if direction not in DIRECTIONS:
            raise SystemExit(f"{feature_id}: bad direction {direction!r}")
        kind = detector.get("kind")
        if kind not in KINDS:
            raise SystemExit(f"{feature_id}: bad kind {kind!r}")
        when = detector.get("when")
        if when is not None and when not in WHENS:
            raise SystemExit(f"{feature_id}: bad when {when!r}")
        if not detector.get("text"):
            raise SystemExit(f"{feature_id}: detector without text")
        if not detector.get("what"):
            raise SystemExit(f"{feature_id}: detector without a 'what' label")
        seen_directions.add(direction)
    for direction in seen_directions:
        key = f"{direction}_action"
        if not spec.get(key):
            raise SystemExit(f"{feature_id}: has {direction} detectors but no {key}")
    for key in ("risk_action", "removable_action"):
        if spec.get(key) and key.split("_", 1)[0] not in seen_directions:
            raise SystemExit(f"{feature_id}: {key} with no matching detector")


def build(dataset: dict, detectors: dict, version: str, tarball_url: str, digest: str) -> dict:
    features_in = dataset["features"]
    out_features = []

    for feature_id in sorted(detectors["features"]):
        spec = detectors["features"][feature_id]
        validate_detectors(feature_id, spec)

        entry = features_in.get(feature_id)
        if entry is None:
            raise SystemExit(
                f"'{feature_id}' is not in web-features {version}. "
                "Detector ids must be real dataset ids; nothing is hand-authored."
            )
        kind = entry.get("kind", "feature")
        if kind != "feature":
            raise SystemExit(
                f"'{feature_id}' is a '{kind}' record in web-features {version}, not a feature. "
                f"Follow its redirect and update tools/detectors.json."
            )

        status = entry["status"]
        baseline = status.get("baseline")
        if baseline not in ("high", "low", False):
            raise SystemExit(f"'{feature_id}': unexpected status.baseline {baseline!r}")
        if baseline in ("high", "low") and not status.get("baseline_low_date"):
            raise SystemExit(f"'{feature_id}': baseline {baseline} without baseline_low_date")
        if baseline == "high" and not status.get("baseline_high_date"):
            raise SystemExit(f"'{feature_id}': baseline high without baseline_high_date")

        spec_urls = entry.get("spec") or []
        if isinstance(spec_urls, str):
            spec_urls = [spec_urls]
        caniuse = entry.get("caniuse") or []
        if isinstance(caniuse, str):
            caniuse = [caniuse]

        out = {
            "id": feature_id,
            "name": entry["name"],
            "description": entry["description"],
            "baseline": "false" if baseline is False else baseline,
            "baseline_low_date": status.get("baseline_low_date"),
            "baseline_high_date": status.get("baseline_high_date"),
            "spec": spec_urls[0] if spec_urls else None,
            "caniuse": caniuse,
            # The full list of @mdn/browser-compat-data keys lives upstream; the
            # count is kept so a reviewer can see the feature was matched to a
            # real, non-empty dataset record.
            "compat_features_count": len(entry.get("compat_features") or []),
        }
        for key in ("risk_action", "removable_action"):
            if spec.get(key):
                out[key] = spec[key]
        out["detectors"] = [
            {
                "direction": d["direction"],
                "when": d.get(
                    "when", "unsatisfied" if d["direction"] == "risk" else "satisfied"
                ),
                "kind": d["kind"],
                "text": d["text"],
                "what": d["what"],
                **({"action": d["action"]} if d.get("action") else {}),
                **({"scope": d["scope"]} if d.get("scope") else {}),
            }
            for d in spec["detectors"]
        ]
        out_features.append(out)

    today = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    return {
        "schema": SCHEMA,
        "dataset": {
            "package": PACKAGE,
            "version": version,
            "source_url": tarball_url,
            "sha256": digest,
            "retrieved_on": today,
            "generator": "tools/fetch-data.py",
            "note": (
                "Baseline statuses and dates below are copied verbatim from "
                f"{PACKAGE}@{version}. This is a snapshot: re-run the generator to refresh it."
            ),
        },
        "features": out_features,
    }


def render(document: dict) -> str:
    return json.dumps(document, indent=2, ensure_ascii=False, sort_keys=False) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", help="pin a web-features version (default: latest)")
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; exit 1 if the snapshot differs from a fresh build",
    )
    args = parser.parse_args()

    detectors = json.loads(DETECTORS_PATH.read_text(encoding="utf-8"))
    if detectors.get("schema") != SCHEMA:
        raise SystemExit(f"tools/detectors.json: expected schema {SCHEMA}")

    version, tarball_url = resolve_version(args.version)
    dataset, digest = load_dataset(tarball_url)
    document = build(dataset, detectors, version, tarball_url, digest)
    text = render(document)

    if args.check:
        current = OUTPUT_PATH.read_text(encoding="utf-8") if OUTPUT_PATH.exists() else ""
        # retrieved_on always moves, so compare everything else.
        def strip_date(s: str) -> str:
            return "\n".join(l for l in s.splitlines() if '"retrieved_on"' not in l)

        if strip_date(current) != strip_date(text):
            print("src/features.json is out of date; re-run tools/fetch-data.py", file=sys.stderr)
            return 1
        print(f"src/features.json matches {PACKAGE}@{version}")
        return 0

    OUTPUT_PATH.write_text(text, encoding="utf-8")
    print(
        f"wrote {OUTPUT_PATH.relative_to(ROOT)}: "
        f"{len(document['features'])} features from {PACKAGE}@{version} "
        f"(sha256 {digest[:16]}...)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
