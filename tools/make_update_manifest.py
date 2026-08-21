#!/usr/bin/env python3
"""Compose the Tauri updater manifest (latest.json) from staged release artifacts.

Run by the release CI job after all builds are staged:

    python3 make_update_manifest.py --version 0.8.7 --tag v0.8.7 \
        --repo akaspyrean/clashedge --dist dist/ --out dist/latest.json

Looks for the updater artifacts by their stable names:

    ClashEdge-portable-win64.zip(.sig)   -> platforms["windows-x86_64"]

URLs point at the TAG-pinned GitHub download path (releases/download/<tag>/<asset>),
never at `latest/` — a manifest must reference exactly the artifacts it shipped with.
"""

from __future__ import annotations

import argparse
import datetime
import json
import pathlib
import sys

ARTIFACTS = {
    "ClashEdge-portable-win64.zip": "windows-x86_64",
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", required=True, help="bare version, e.g. 0.8.7")
    ap.add_argument("--tag", required=True, help="git tag, e.g. v0.8.7")
    ap.add_argument("--repo", required=True, help="owner/name, e.g. akaspyrean/clashedge")
    ap.add_argument("--dist", required=True, type=pathlib.Path, help="staged artifacts dir")
    ap.add_argument("--out", required=True, type=pathlib.Path)
    ap.add_argument("--notes", default="", help="release notes line shown in the update prompt")
    args = ap.parse_args()

    platforms: dict[str, dict[str, str]] = {}
    for asset, platform in ARTIFACTS.items():
        artifact = args.dist / asset
        sig = args.dist / (asset + ".sig")
        if not artifact.exists():
            print(f"warning: {asset} not in {args.dist} — skipping {platform}", file=sys.stderr)
            continue
        if not sig.exists():
            print(f"warning: {asset} has no .sig — skipping {platform}", file=sys.stderr)
            continue
        platforms[platform] = {
            "signature": sig.read_text().strip(),
            "url": f"https://github.com/{args.repo}/releases/download/{args.tag}/{asset}",
        }

    if not platforms:
        print("error: no signed updater artifacts found — refusing to write an empty manifest", file=sys.stderr)
        return 1

    manifest = {
        "version": args.version,
        "notes": args.notes,
        "pub_date": datetime.datetime.now(datetime.timezone.utc)
        .isoformat(timespec="seconds")
        .replace("+00:00", "Z"),
        "platforms": platforms,
    }
    args.out.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {args.out} ({', '.join(sorted(platforms))})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())