#!/usr/bin/env python3
"""Compose the signed rule-set manifest (rules-manifest.json).

Run in the *external rules repository* (akaspyrean/external) after its rule files
and geodata have been refreshed, from a checkout of the exact commit being published:

    python3 make-rules-manifest.py --repo-dir . --commit "$(git rev-parse HEAD)" \
        --out rules-manifest.json

Then sign it with the SAME minisign key as the portable-manifest (the client verifies
both with the one embedded public key) and publish the pair at the repo root:

    rules-manifest.json
    rules-manifest.json.minisig      # unwrapped 4-line minisign text, as for portable-manifest

The client (src-tauri/src/geodata/rules.rs) fetches
https://raw.githubusercontent.com/akaspyrean/external/main/rules-manifest.json(.minisig),
verifies the signature, refuses an older `version` than it has installed (rollback
protection), checks every file's path / URL prefix / SHA256 / size, validates content and
only then replaces the local files transactionally.

Output schema:

    {
      "version": 202610081200,            # monotonic (YYYYMMDDHHMM, UTC)
      "released_at": 1791460800,          # unix seconds, informational
      "files": [
        {"path": "rules/ai.yaml",
         "url":  "https://raw.githubusercontent.com/akaspyrean/external/<commit>/rules/ai.yaml",
         "sha256": "<hex>", "size": 12345},
        {"path": "GeoIP.dat",
         "url":  ".../<commit>/geodata/GeoIP.dat", ...}
      ]
    }

URLs always contain the full commit SHA (immutable), never a branch name.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import pathlib
import re
import sys

RAW_BASE = "https://raw.githubusercontent.com/akaspyrean/external"
RULE_NAME = re.compile(r"^[a-z0-9_-]{1,64}\.yaml$")
# repo path -> client target path (Data root names the core actually reads)
GEO_FILES = {
    "geodata/GeoIP.dat": "GeoIP.dat",
    "geodata/GeoSite.dat": "GeoSite.dat",
    "geodata/Country.mmdb": "Country.mmdb",
}


def sha256_of(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo-dir", required=True, type=pathlib.Path)
    ap.add_argument("--commit", required=True, help="full 40-hex commit SHA being published")
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path("rules-manifest.json"))
    ap.add_argument("--no-geodata", action="store_true", help="only list rules/*.yaml")
    args = ap.parse_args()

    if not re.fullmatch(r"[0-9a-f]{40}", args.commit):
        print("error: --commit must be a full 40-hex SHA (never a branch name)", file=sys.stderr)
        return 1

    files = []
    rules_dir = args.repo_dir / "rules"
    for f in sorted(rules_dir.glob("*.yaml")):
        if not RULE_NAME.match(f.name):
            print(f"error: rule file name not allowed by the client whitelist: {f.name}", file=sys.stderr)
            return 1
        files.append((f"rules/{f.name}", f"rules/{f.name}", f))
    if not files:
        print(f"error: no rules/*.yaml under {args.repo_dir}", file=sys.stderr)
        return 1
    if not args.no_geodata:
        for repo_path, target in GEO_FILES.items():
            f = args.repo_dir / repo_path
            if f.exists():
                files.append((target, repo_path, f))

    now = datetime.datetime.now(datetime.timezone.utc)
    manifest = {
        "version": int(now.strftime("%Y%m%d%H%M")),
        "released_at": int(now.timestamp()),
        "files": [
            {
                "path": target,
                "url": f"{RAW_BASE}/{args.commit}/{repo_path}",
                "sha256": sha256_of(f),
                "size": f.stat().st_size,
            }
            for target, repo_path, f in files
        ],
    }
    args.out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.out}: v{manifest['version']}, {len(files)} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
