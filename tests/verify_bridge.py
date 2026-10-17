#!/usr/bin/env python3
"""Compile and run the C++ bridge/math checks against a native Linux codec build.

The native dependency prefix must contain the pinned libheif, libde265, x265 and lcms2
headers and static libraries. Inputs are explicitly supplied local test photos;
this script never downloads anything. It produces no converted photo files.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def run(command):
    result = subprocess.run([str(x) for x in command], check=False, text=True, capture_output=True)
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="")
    result.check_returncode()
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-dir", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--cxx", default="g++")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    prefix = args.native_dir.resolve()
    args.build_dir.mkdir(parents=True, exist_ok=True)
    manifest = json.loads((root / "tests/fixture_manifest.json").read_text())
    inputs = []
    for item in manifest["files"]:
        path = args.fixtures / item["file"]
        data = path.read_bytes()
        if len(data) != item["bytes"] or hashlib.sha256(data).hexdigest() != item["sha256"]:
            raise RuntimeError(f"Fixture hash mismatch: {path}")
        inputs.append(path.resolve())

    compile_flags = [args.cxx, "-std=c++17", "-O2", "-pipe", "-Wall", "-Wextra", "-Wpedantic",
                     "-DLIBHEIF_STATIC_BUILD", "-I" + str(prefix / "include"),
                     "-I" + str(root / "native")]
    link_flags = ["-L" + str(prefix / "lib"), "-lheif", "-lde265", "-lx265", "-llcms2", "-lpthread", "-lm", "-ldl"]
    math = args.build_dir.resolve() / "colour_math"
    bridge = args.build_dir.resolve() / "check_bridge"
    run(compile_flags + [root / "tests/colour_math.cpp"] + link_flags + ["-o", math])
    math_output = run([math])
    run(compile_flags + [root / "native/bridge.cpp", root / "tests/check_bridge.cpp"] + link_flags + ["-o", bridge])
    output = run([bridge] + inputs)
    if f"libheif={manifest['decoder_version']}\n" not in output:
        raise RuntimeError("The codec version does not match this regression manifest")
    parsed = []
    for line in output.splitlines():
        match = re.match(r"(.+) \[(\d+)/(\d+)\] (\d+)x(\d+) flags=(\d+) bytes=(\d+) icc=(\d+) exif=(\d+)", line)
        if match:
            name, frame, count, width, height, flags, nbytes, icc, exif = match.groups()
            parsed.append({"file": Path(name).name, "frame": int(frame), "count": int(count),
                           "width": int(width), "height": int(height), "flags": int(flags),
                           "pixels_len": int(nbytes), "icc_len": int(icc), "exif_len": int(exif)})
    expected_count = sum(len(item["expected_bridge"]) for item in manifest["files"])
    if len(parsed) != expected_count:
        raise RuntimeError("A fixture failed to decode or returned the wrong number of frames")
    for item in manifest["files"]:
        frames = [x for x in parsed if x["file"] == item["file"]]
        if len(frames) != len(item["expected_bridge"]):
            raise RuntimeError(f"Incorrect frame count: {item['file']}")
        for actual, expected in zip(frames, item["expected_bridge"]):
            for key in ("width", "height", "flags"):
                if actual[key] != expected[key]:
                    raise RuntimeError(f"{item['file']} frame{actual['frame']}: {key}={actual[key]}, expected={expected[key]}")
            if actual["pixels_len"] != actual["width"] * actual["height"] * 3 or actual["icc_len"] < 128:
                raise RuntimeError(f"Invalid RGB8 buffer or missing output ICC: {item['file']}")

    report = {"result": "passed", "decoder_version": manifest["decoder_version"],
              "input_files": len(manifest["files"]), "decoded_still_images": len(parsed),
              "color_math": math_output.strip(), "images": parsed,
              "limits": "This verifies the native Linux codec/bridge and pixel math; it is not a Windows UI or display-calibration test."}
    (args.build_dir / "bridge_report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: {len(manifest['files'])} real HEIC inputs, {len(parsed)} still images, color math, and profile/buffer checks.")


if __name__ == "__main__":
    main()
