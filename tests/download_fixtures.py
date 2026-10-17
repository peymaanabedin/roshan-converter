#!/usr/bin/env python3
"""Fetch pinned public regression inputs for local testing, checking SHA-256.

No input image is included in the application/source distribution. Running this
script deliberately downloads upstream test images; it is not used by the app.
"""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True,
                        help="Local test-data directory, outside the distributable source tree")
    parser.add_argument("--only", nargs="+", help="Fetch only the specified manifest filenames")
    args = parser.parse_args()
    manifest = json.loads(Path(__file__).with_name("fixture_manifest.json").read_text())
    wanted = set(args.only) if args.only else None
    if wanted and wanted - {x["file"] for x in manifest["files"]}:
        parser.error("--only contains a filename absent from the manifest")
    args.out.mkdir(parents=True, exist_ok=True)
    for item in manifest["files"]:
        if wanted and item["file"] not in wanted:
            continue
        path = args.out / item["file"]
        if path.exists():
            data = path.read_bytes()
            if len(data) == item["bytes"] and hashlib.sha256(data).hexdigest() == item["sha256"]:
                print(f"VERIFIED cached {item['file']}")
                continue
        request = urllib.request.Request(item["source"], headers={"User-Agent": "Roshan-HEIC-regression-tests/1.0"})
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read(item["bytes"] + 1)
        if len(data) != item["bytes"] or hashlib.sha256(data).hexdigest() != item["sha256"]:
            raise RuntimeError(f"Upstream bytes do not match the pinned fixture: {item['file']}")
        temp = path.with_name(path.name + ".part")
        temp.write_bytes(data)
        temp.replace(path)
        print(f"VERIFIED downloaded {item['file']}: {len(data)} bytes")


if __name__ == "__main__":
    main()
