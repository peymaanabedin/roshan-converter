#!/usr/bin/env python3
"""Build the pinned native libraries used by Roshan HEIC, without shared codecs.

Python 3.12+, CMake 3.16.3+, Make (or Ninja) and a C/C++ compiler are required.
All third-party source archives are verified before extraction.  A complete
source bundle contains the archives, so rebuilding requires no network access.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

HERE = Path(__file__).resolve().parent


def verify_extract(spec: dict, archives: Path, sources: Path, allow_download: bool) -> Path:
    archive = archives / spec["filename"]
    if not archive.is_file():
        if not allow_download:
            raise RuntimeError(f"Missing source archive {archive}; supply it or use --download")
        archive.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(spec["url"], timeout=90) as response:
            data = response.read()
        archive.write_bytes(data)
    with archive.open("rb") as archive_file:
        digest = hashlib.file_digest(archive_file, "sha256").hexdigest()
    if digest != spec["sha256"]:
        raise RuntimeError(f"SHA-256 mismatch for {archive}: {digest}")
    source = sources / spec["directory"]
    if not source.is_dir():
        sources.mkdir(parents=True, exist_ok=True)
        with tarfile.open(archive) as package:
            package.extractall(sources, filter="data")
    return source


def execute(command: list[str], log_file: Path) -> None:
    log_file.parent.mkdir(parents=True, exist_ok=True)
    with log_file.open("a", encoding="utf-8") as log:
        log.write("\n" + subprocess.list2cmdline(command) + "\n")
        log.flush()
        process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    if process.returncode:
        lines = log_file.read_text(encoding="utf-8", errors="replace").splitlines()
        raise RuntimeError(f"Build failed; see {log_file}\n" + "\n".join(lines[-55:]))


def build_target(target: str, args: argparse.Namespace, sources: dict[str, Path]) -> dict:
    output = args.output.resolve()
    prefix = output / target
    build_root = output / "build" / target
    common = [args.cmake, "-G", args.generator,
              "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_SHARED_LIBS=OFF",
              f"-DCMAKE_INSTALL_PREFIX={prefix}", "-DCMAKE_INSTALL_LIBDIR=lib",
              "-DCMAKE_POSITION_INDEPENDENT_CODE=ON"]
    if target == "windows":
        if not args.llvm_mingw:
            raise RuntimeError("--llvm-mingw is required to cross-compile for Windows")
        toolchain = args.llvm_mingw.resolve()
        tools = toolchain / "bin"
        common += ["-DCMAKE_SYSTEM_NAME=Windows", "-DCMAKE_SYSTEM_PROCESSOR=x86_64",
                   f"-DCMAKE_C_COMPILER={tools / 'x86_64-w64-mingw32-clang'}",
                   f"-DCMAKE_CXX_COMPILER={tools / 'x86_64-w64-mingw32-clang++'}",
                   f"-DCMAKE_RC_COMPILER={tools / 'x86_64-w64-mingw32-windres'}",
                   f"-DCMAKE_AR={tools / 'llvm-ar'}",
                   f"-DCMAKE_RANLIB={tools / 'llvm-ranlib'}",
                   f"-DCMAKE_FIND_ROOT_PATH={prefix};{toolchain / 'x86_64-w64-mingw32'}",
                   "-DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM=NEVER",
                   "-DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=ONLY",
                   "-DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=ONLY",
                   "-DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=ONLY",
                   "-DCMAKE_C_FLAGS=-DWINVER=0x0A00 -D_WIN32_WINNT=0x0A00",
                   "-DCMAKE_CXX_FLAGS=-DWINVER=0x0A00 -D_WIN32_WINNT=0x0A00 -DLIBDE265_STATIC_BUILD",
                   "-DCMAKE_EXE_LINKER_FLAGS=-static"]
    else:
        common += ["-DCMAKE_CXX_FLAGS=-DLIBDE265_STATIC_BUILD"]
    if args.make_program:
        common += [f"-DCMAKE_MAKE_PROGRAM={Path(args.make_program).resolve()}"]

    disabled_codecs = ["X264", "OpenH264_DECODER", "AOM_DECODER", "AOM_ENCODER",
                       "DAV1D", "RAV1E", "SvtEnc", "JPEG_DECODER", "JPEG_ENCODER",
                       "OpenJPEG_DECODER", "OpenJPEG_ENCODER", "FFMPEG_DECODER", "KVAZAAR",
                       "UVG266", "VVDEC", "VVENC", "OPENJPH_ENCODER", "UNCOMPRESSED_CODEC",
                       "LIBSHARPYUV", "LIBSHARPYUV_INTERNAL", "HEADER_COMPRESSION", "WEBCODECS",
                       "EXAMPLES", "GDK_PIXBUF", "FUZZERS"]
    specifications = [
        ("libde265", ["-DENABLE_DECODER=OFF", "-DENABLE_ENCODER=OFF", "-DENABLE_SDL=OFF",
                      "-DENABLE_SHERLOCK265=OFF", "-DENABLE_INTERNAL_DEVELOPMENT_TOOLS=OFF",
                      "-DWITH_FUZZERS=OFF"]),
        ("lcms2", ["-DLCMS2_BUILD_STATIC=ON", "-DLCMS2_BUILD_SHARED=OFF",
                   "-DLCMS2_BUILD_TOOLS=OFF", "-DLCMS2_BUILD_TESTS=OFF",
                   "-DLCMS2_WITH_JPEG=OFF", "-DLCMS2_WITH_TIFF=OFF", "-DLCMS2_WITH_ZLIB=OFF",
                   "-DLCMS2_WITH_FASTFLOAT=OFF", "-DLCMS2_WITH_THREADED_PLUGIN=OFF"]),
        ("x265", ["-DENABLE_SHARED=OFF", "-DENABLE_CLI=OFF", "-DHIGH_BIT_DEPTH=OFF",
                  "-DENABLE_LIBNUMA=OFF", "-DENABLE_LIBVMAF=OFF", "-DENABLE_HDR10_PLUS=OFF",
                  "-DEXPORT_C_API=ON", "-DENABLE_PIC=ON", "-DENABLE_TESTS=OFF",
                  "-DENABLE_ALPHA=OFF", "-DENABLE_MULTIVIEW=OFF", "-DENABLE_SCC_EXT=OFF",
                  f"-DENABLE_ASSEMBLY={'ON' if args.nasm else 'OFF'}",
                  *(["-DCMAKE_CXX_FLAGS=-DLIBDE265_STATIC_BUILD -pipe"] if target == "linux" else []),
                  *([f"-DNASM_EXECUTABLE={args.nasm.resolve()}"] if args.nasm else [])]),
        ("libheif", ["-DENABLE_PLUGIN_LOADING=OFF", "-DWITH_LIBDE265=ON", "-DWITH_LIBDE265_PLUGIN=OFF",
                     "-DWITH_X265=ON", "-DWITH_X265_PLUGIN=OFF",
                     "-DBUILD_TESTING=OFF", "-DBUILD_DOCUMENTATION=OFF", "-DBUILD_DEVELOPMENT_TOOLS=OFF",
                     f"-DLIBDE265_INCLUDE_DIR={prefix / 'include'}",
                     f"-DLIBDE265_LIBRARY={prefix / 'lib/libde265.a'}",
                     f"-DX265_INCLUDE_DIR={prefix / 'include'}",
                     f"-DX265_LIBRARY={prefix / 'lib/libx265.a'}",
                     *[f"-DWITH_{codec}=OFF" for codec in disabled_codecs]])
    ]
    for name, options in specifications:
        build_dir = build_root / name
        logfile = build_root / f"{name}.log"
        print(f"[{target}] Building {name}; log: {logfile}", flush=True)
        source_dir = sources[name] / "source" if name == "x265" else sources[name]
        execute(common + ["-S", str(source_dir), "-B", str(build_dir)] + options, logfile)
        execute([args.cmake, "--build", str(build_dir), "--parallel", str(args.jobs)], logfile)
        # Do not install an incomplete archive if an interrupted compiler left
        # a zero-byte object behind in a reused build directory.
        empty_objects = [p for p in build_dir.rglob("*")
                         if p.suffix in {".o", ".obj"} and p.is_file() and p.stat().st_size == 0]
        if empty_objects:
            raise RuntimeError(f"Empty compiler object found; remove it and rebuild: {empty_objects[0]}")
        execute([args.cmake, "--install", str(build_dir)], logfile)

    libraries = {}
    for filename in ["libde265.a", "liblcms2.a", "libx265.a", "libheif.a"]:
        file = prefix / "lib" / filename
        if not file.is_file():
            raise RuntimeError(f"Expected static library was not installed: {file}")
        with file.open("rb") as library_file:
            digest = hashlib.file_digest(library_file, "sha256").hexdigest()
        libraries[filename] = {"bytes": file.stat().st_size, "sha256": digest}
    # A real link pulls in x265's full encoder API and catches incomplete or
    # incompatible archive members before reporting the build as successful.
    cache = (build_root / "libheif/CMakeCache.txt").read_text()
    compiler = next(line.split("=", 1)[1] for line in cache.splitlines()
                    if line.startswith("CMAKE_CXX_COMPILER:") and "=" in line)
    smoke = build_root / ("native-smoke.exe" if target == "windows" else "native-smoke")
    smoke_command = [compiler, "-std=c++17", "-O2", "-DLIBHEIF_STATIC_BUILD", "-DLIBDE265_STATIC_BUILD",
                     "-I" + str(prefix / "include"), "-x", "c++", str(HERE / "native_smoke.c"), "-x", "none",
                     *[str(prefix / "lib" / name) for name in ["libheif.a", "libde265.a", "libx265.a", "liblcms2.a"]]]
    smoke_command += ["-static", "-ladvapi32"] if target == "windows" else ["-lpthread", "-lm", "-ldl"]
    execute(smoke_command + ["-o", str(smoke)], build_root / "native-smoke.log")
    if target == "linux":
        execute([str(smoke)], build_root / "native-smoke.log")
    report = {"target": target, "prefix": str(prefix), "libraries": libraries,
              "codecs": ["libde265 (HEVC decoder, statically built in)",
                         "x265 (8-bit HEVC encoder, lossless 4:4:4, statically built in)"],
              "x265_assembly": bool(args.nasm),
              "native_encoder_link_smoke": "passed",
              "plugin_loading": False}
    (prefix / "build-report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"[{target}] Complete: {prefix}", flush=True)
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["windows", "linux", "both"], default="windows")
    parser.add_argument("--archives", type=Path, default=HERE / "archives")
    parser.add_argument("--sources", type=Path, default=HERE / "sources")
    parser.add_argument("--output", type=Path, default=HERE.parent / "native")
    parser.add_argument("--llvm-mingw", type=Path)
    parser.add_argument("--cmake", default=shutil.which("cmake") or "cmake")
    parser.add_argument("--generator", default="Unix Makefiles")
    parser.add_argument("--make-program")
    parser.add_argument("--nasm", type=Path, help="Optional NASM executable for x265 runtime-dispatched SIMD")
    parser.add_argument("--jobs", type=int, default=max(1, min(os.cpu_count() or 2, 4)))
    parser.add_argument("--download", action="store_true", help="Fetch missing pinned archives")
    args = parser.parse_args()
    temporary_dir = args.output.resolve() / "build" / "tmp"
    temporary_dir.mkdir(parents=True, exist_ok=True)
    os.environ.setdefault("TMPDIR", str(temporary_dir))
    if args.llvm_mingw:
        # Also works in restricted build containers where ELF $ORIGIN lookup
        # through /proc/self/exe is unavailable. It only affects build tools.
        compiler_lib = str(args.llvm_mingw.resolve() / "lib")
        existing_path = os.environ.get("LD_LIBRARY_PATH", "")
        os.environ["LD_LIBRARY_PATH"] = compiler_lib + (os.pathsep + existing_path if existing_path else "")
    sources = {entry["name"]: verify_extract(entry, args.archives.resolve(), args.sources.resolve(), args.download)
               for entry in json.loads((HERE / "dependencies.json").read_text())}
    targets = ["windows", "linux"] if args.target == "both" else [args.target]
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(targets)) as executor:
        futures = [executor.submit(build_target, target, args, sources) for target in targets]
        for future in concurrent.futures.as_completed(futures):
            future.result()


if __name__ == "__main__":
    main()
