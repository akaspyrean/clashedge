#!/usr/bin/env python3
"""Compose the Portable Updater manifest (portable-manifest.json).

Run by the release CI job after all builds are staged:

    python3 make_update_manifest.py --version 0.8.10 --tag v0.8.10 \
        --repo akaspyrean/clashedge --dist dist/ \
        --out dist/portable-manifest.json

Output schema (Portable Updater's own format, NOT the Tauri updater
`platforms` structure):

    {
      "version": "0.8.10",
      "url": "https://github.com/<repo>/releases/download/<tag>/ClashEdge-portable-win64.zip",
      "sha256": "<lowercase hex SHA256 of the zip>",
      "notes": "..."
    }

The zip hash is computed from the actual staged artifact in --dist, never
taken on trust from any side channel. URLs point at the TAG-pinned GitHub
download path (releases/download/<tag>/<asset>), never at `latest/` — a
manifest must reference exactly the artifact it shipped with.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys

ZIP_ASSET = "ClashEdge-portable-win64.zip"


def sha256_of(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", required=True, help="bare version, e.g. 0.8.10")
    ap.add_argument("--tag", required=True, help="git tag, e.g. v0.8.10")
    ap.add_argument("--repo", required=True, help="owner/name, e.g. akaspyrean/clashedge")
    ap.add_argument("--dist", required=True, type=pathlib.Path, help="staged artifacts dir")
    ap.add_argument(
        "--out",
        type=pathlib.Path,
        default=pathlib.Path("portable-manifest.json"),
        help="manifest output path (default: portable-manifest.json)",
    )
    ap.add_argument("--notes", default="", help="release notes line shown in the update prompt")
    args = ap.parse_args()

    artifact = args.dist / ZIP_ASSET
    if not artifact.exists():
        print(f"error: {ZIP_ASSET} not found in {args.dist} — refusing to write a manifest", file=sys.stderr)
        return 1

    manifest = {
        "version": args.version,
        "url": f"https://github.com/{args.repo}/releases/download/{args.tag}/{ZIP_ASSET}",
        "sha256": sha256_of(artifact),
        "notes": args.notes,
    }
    args.out.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {args.out} ({ZIP_ASSET}, sha256={manifest['sha256'][:16]}...)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
