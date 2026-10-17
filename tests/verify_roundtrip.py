#!/usr/bin/env python3
"""Build/run the synthetic HEIC encoder and RGBA bridge checks, without network.

For Linux supply the native Linux prefix. For Windows cross compilation also
supply --target windows --cxx /path/to/clang++ --runner /path/to/wine.
An optional --runner-env JSON contains environment variables for the runner.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-dir", type=Path, required=True)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--cxx", default="g++")
    parser.add_argument("--target", choices=["linux", "windows"], default="linux")
    parser.add_argument("--runner")
    parser.add_argument("--runner-env", type=Path)
    parser.add_argument("--llvm-readobj")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    prefix = args.native_dir.resolve()
    build = args.build_dir.resolve()
    build.mkdir(parents=True, exist_ok=True)
    binary = build / ("heic-roundtrip.exe" if args.target == "windows" else "heic-roundtrip")
    source = root / "native/bridge.cpp"
    test = root / "tests/heic_roundtrip.cpp"
    command = [args.cxx, "-std=c++17", "-O2", "-pipe", "-Wall", "-Wextra", "-Wpedantic",
               "-DLIBHEIF_STATIC_BUILD", "-DLIBDE265_STATIC_BUILD", "-I" + str(prefix / "include"),
               str(source), str(test), "-L" + str(prefix / "lib"),
               "-lheif", "-lde265", "-lx265", "-llcms2"]
    command += ["-static", "-ladvapi32"] if args.target == "windows" else ["-lpthread", "-lm", "-ldl"]
    command += ["-o", str(binary)]
    compile_env = os.environ.copy()
    compile_env.setdefault("TMPDIR", str(build))
    subprocess.run(command, check=True, env=compile_env)
    runner_env = os.environ.copy()
    if args.runner_env:
        runner_env.update(json.loads(args.runner_env.read_text()))
        runner_env.pop("DISPLAY", None)
    run_command = ([args.runner] if args.runner else []) + [str(binary)]
    run = subprocess.run(run_command, check=False, env=runner_env, text=True,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90)
    (build / "roundtrip.stdout.txt").write_text(run.stdout)
    (build / "roundtrip.stderr.txt").write_text(run.stderr)
    print(run.stdout, end="")
    if run.returncode != 0:
        raise RuntimeError(f"Roundtrip test exited {run.returncode}: {run.stderr}")
    if "PASS HEIC V2: 9 synthetic" not in run.stdout:
        raise RuntimeError("The test did not report all expected cases")
    imports = []
    if args.llvm_readobj:
        inspection = subprocess.run([args.llvm_readobj, "--coff-imports", str(binary)], check=True,
                                    text=True, stdout=subprocess.PIPE, env=compile_env).stdout
        (build / "windows-imports.txt").write_text(inspection)
        imports = [line.strip().split(": ", 1)[1] for line in inspection.splitlines() if line.strip().startswith("Name: ")]
        for name in imports:
            if name.lower() not in {"kernel32.dll", "advapi32.dll"} and not name.lower().startswith("api-ms-win-crt-"):
                raise RuntimeError(f"Unexpected non-system DLL import: {name}")
    report = {"result": "passed", "target": args.target, "runner": args.runner or "native",
              "synthetic_roundtrips": 9, "input_precision": "RGB8/RGBA8", "codec": "x265 4.3 via libheif 1.23.6",
              "heic_mode": "lossless full-range RGB/GBR identity, 4:4:4, optional straight alpha",
              "checks": ["sample-exact RGB and alpha, including hidden RGB at alpha=0", "odd and tiny dimensions",
                         "ICC bytes unchanged", "EXIF TIFF bytes unchanged", "8-bit depth and identity matrix",
                         "independent concurrent encoders", "linear white alpha compositing", "invalid ICC/buffer rejection",
                         "deterministic sRGB ICC serialization"],
              "source_sha256": sha(source), "header_sha256": sha(root / "native/bridge.h"),
              "test_sha256": sha(test), "binary_sha256": sha(binary),
              "native_libraries": {name: sha(prefix / "lib" / name) for name in ["libheif.a", "libde265.a", "libx265.a", "liblcms2.a"]},
              "system_dll_imports": imports,
              "stdout": run.stdout, "stderr": run.stderr,
              "limits": "This is a synthetic codec/FFI test, not a Windows GUI or display-calibration test. HEIC losslessness applies to supplied 8-bit pixels after any source HDR/precision conversion."}
    (build / "roundtrip_report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: recorded full roundtrip test report")


if __name__ == "__main__":
    main()
