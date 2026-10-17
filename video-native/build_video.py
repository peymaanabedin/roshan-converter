#!/usr/bin/env python3
"""Build the pinned offline FFmpeg/ffprobe payloads and all codec dependencies.

Source archives are verified locally. No build stage downloads dependencies.
Requires Python 3.12+, CMake, Make, Meson, Ninja, NASM, pkg-config and a C/C++
compiler; cross-compiling the Windows payload also requires llvm-mingw.
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
import sys
import tarfile

ROOT = Path(__file__).resolve().parent


def run(command: list[str], cwd: Path, env: dict[str, str], log_path: Path) -> None:
    cwd.mkdir(parents=True, exist_ok=True)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("a", encoding="utf-8") as log:
        log.write("\n" + subprocess.list2cmdline(command) + "\n")
        log.flush()
        result = subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = "\n".join(log_path.read_text(errors="replace").splitlines()[-45:])
        raise RuntimeError(f"Build failed; see {log_path}\n{tail}")


def compile_objects(command: list[str], cwd: Path, env: dict[str, str], log_path: Path) -> None:
    """Reject incomplete object writes and allow an incremental regeneration.

    Some remote workspace filesystems can leave an empty output after a
    successful compiler process. An empty object is never a valid input to a
    static library; do not install one or hide a genuine compiler failure.
    """
    def remove_empty() -> int:
        empty = [p for p in cwd.rglob("*") if p.suffix in (".o", ".obj")
                 and p.is_file() and p.stat().st_size == 0]
        for path in empty:
            path.unlink()
            path.with_suffix(".lo").unlink(missing_ok=True)
        return len(empty)

    for _ in range(3):
        remove_empty()
        try:
            run(command, cwd, env, log_path)
        except RuntimeError:
            if remove_empty():
                continue
            raise
        if not remove_empty():
            return
    raise RuntimeError(f"Repeated incomplete compiler output under {cwd}")


def prepare_sources() -> dict[str, Path]:
    result = {}
    for entry in json.loads((ROOT / "dependencies.json").read_text()):
        archive = ROOT / "archives" / entry["filename"]
        with archive.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        if digest != entry["sha256"]:
            raise RuntimeError(f"Source hash mismatch: {archive}")
        with tarfile.open(archive) as tar:
            dirname = tar.getmembers()[0].name.split("/")[0]
            path = ROOT / "sources" / dirname
            if not path.is_dir():
                tar.extractall(ROOT / "sources", filter="data")
        result[entry["name"]] = path
    return result


def make_env(args: argparse.Namespace, target: str) -> dict[str, str]:
    env = os.environ.copy()
    if args.environment:
        env.update(json.loads(args.environment.read_text()))
    prefix = ROOT / target
    env["PKG_CONFIG_LIBDIR"] = str(prefix / "lib/pkgconfig")
    env["PKG_CONFIG_PATH"] = str(prefix / "lib/pkgconfig")
    env["SOURCE_DATE_EPOCH"] = "1789704000"
    env["TZ"] = "UTC"
    # Pipe compiler output directly to the assembler. This also avoids slow or
    # partially materialized temporary assembly files on remote workspaces.
    env["CFLAGS"] = "-O3 -pipe"
    env["CXXFLAGS"] = "-O3 -pipe"
    tmp = ROOT / "build/tmp"
    tmp.mkdir(parents=True, exist_ok=True)
    env["TMPDIR"] = str(tmp)
    if target == "windows":
        toolchain = args.llvm_mingw.resolve()
        env["PATH"] = str(toolchain / "bin") + os.pathsep + env["PATH"]
        env["LD_LIBRARY_PATH"] = str(toolchain / "lib") + os.pathsep + env.get("LD_LIBRARY_PATH", "")
        env["CC"] = str(toolchain / "bin/x86_64-w64-mingw32-clang")
        env["CXX"] = str(toolchain / "bin/x86_64-w64-mingw32-clang++")
        env["AR"] = str(toolchain / "bin/llvm-ar")
        env["RANLIB"] = str(toolchain / "bin/llvm-ranlib")
        env["STRIP"] = str(toolchain / "bin/llvm-strip")
        env["RC"] = str(toolchain / "bin/x86_64-w64-mingw32-windres")
        env["LDFLAGS"] = "-static"
        env["STL_LIBS"] = "-lc++ -lc++abi -lunwind -lwinpthread"
    else:
        env.update(CC="gcc", CXX="g++", AR="ar", RANLIB="ranlib", STRIP="strip", STL_LIBS="-lstdc++ -lm")
    return env


def build(target: str, args: argparse.Namespace, source: dict[str, Path]) -> dict:
    env = make_env(args, target)
    prefix = ROOT / target
    builddir = ROOT / "build" / target
    prefix.mkdir(exist_ok=True)
    builddir.mkdir(parents=True, exist_ok=True)
    configure_cross = ["--host=x86_64-w64-mingw32"] if target == "windows" else []
    cmake_cross = []
    if target == "windows":
        cmake_cross = ["-DCMAKE_SYSTEM_NAME=Windows", "-DCMAKE_SYSTEM_PROCESSOR=x86_64",
                       f"-DCMAKE_C_COMPILER={env['CC']}", f"-DCMAKE_CXX_COMPILER={env['CXX']}",
                       f"-DCMAKE_AR={env['AR']}", f"-DCMAKE_RANLIB={env['RANLIB']}",
                       "-DCMAKE_EXE_LINKER_FLAGS=-static"]

    def autotools(name: str, options: list[str]) -> None:
        directory = builddir / name
        logfile = builddir / (name + ".log")
        print(f"[{target}] {name}", flush=True)
        run([str(source[name] / "configure"), f"--prefix={prefix}"] + options, directory, env, logfile)
        compile_objects(["make", "-j", str(args.jobs)], directory, env, logfile)
        run(["make", "install"], directory, env, logfile)

    stages = ["x264", "opus", "libvpx", "zimg", "dav1d", "ffmpeg"]
    start = stages.index(args.from_stage)

    if start <= 0:
        autotools("x264", ["--enable-static", "--disable-cli", "--disable-opencl", "--disable-avs",
                           "--disable-lavf", "--disable-swscale", "--disable-ffms",
                           "--bit-depth=all", "--chroma-format=all"] + configure_cross)

    if start <= 1:
        print(f"[{target}] opus", flush=True)
        directory = builddir / "opus"
        logfile = builddir / "opus.log"
        run([args.cmake, "-S", str(source["opus"]), "-B", str(directory), "-G", "Unix Makefiles",
             f"-DCMAKE_INSTALL_PREFIX={prefix}", "-DCMAKE_INSTALL_LIBDIR=lib", "-DCMAKE_BUILD_TYPE=Release",
             "-DCMAKE_C_FLAGS=-pipe", "-DCMAKE_CXX_FLAGS=-pipe",
             "-DBUILD_SHARED_LIBS=OFF", "-DOPUS_BUILD_SHARED_LIBRARY=OFF", "-DOPUS_BUILD_PROGRAMS=OFF",
             "-DOPUS_BUILD_TESTING=OFF", "-DOPUS_DRED=OFF", "-DOPUS_OSCE=OFF"] + cmake_cross,
            directory, env, logfile)
        compile_objects([args.cmake, "--build", str(directory), "--parallel", str(args.jobs)], directory, env, logfile)
        run([args.cmake, "--install", str(directory)], directory, env, logfile)

    if start <= 2:
        vpx_env = env.copy()
        vpx_env["AS"] = "nasm"
        vpx_target = "x86_64-win64-gcc" if target == "windows" else "x86_64-linux-gcc"
        directory = builddir / "libvpx"
        logfile = builddir / "libvpx.log"
        print(f"[{target}] libvpx", flush=True)
        vpx_options = [f"--target={vpx_target}", "--disable-shared", "--enable-static", "--disable-examples",
                       "--disable-tools", "--disable-docs", "--disable-unit-tests", "--disable-webm-io",
                       "--disable-libyuv", "--disable-vp8", "--enable-vp9-highbitdepth", "--as=nasm"]
        if target == "windows":
            vpx_env["CROSS"] = str(args.llvm_mingw.resolve() / "bin/x86_64-w64-mingw32-")
            vpx_env["LD"] = env["CC"]
        run([str(source["libvpx"] / "configure"), f"--prefix={prefix}"] + vpx_options,
            directory, vpx_env, logfile)
        compile_objects(["make", "-j", str(args.jobs)], directory, vpx_env, logfile)
        run(["make", "install"], directory, vpx_env, logfile)

    if start <= 3:
        autotools("zimg", ["--enable-static", "--disable-shared", "--disable-testapp", "--disable-example",
                           "--disable-unit-test", "CFLAGS=-O3 -pipe", "CXXFLAGS=-O3 -pipe -include exception"] + configure_cross)

    if start <= 4:
        directory = builddir / "dav1d"
        logfile = builddir / "dav1d.log"
        print(f"[{target}] dav1d", flush=True)
        meson_options = []
        if target == "windows":
            cross = builddir / "dav1d-cross.ini"
            cross.write_text("[binaries]\n" + "\n".join(
                f"{k} = '{env[v]}'" for k, v in [("c", "CC"), ("cpp", "CXX"), ("ar", "AR"), ("strip", "STRIP"), ("windres", "RC")])
                + "\n[host_machine]\nsystem = 'windows'\ncpu_family = 'x86_64'\ncpu = 'x86_64'\nendian = 'little'\n"
                + "[properties]\nneeds_exe_wrapper = true\n[built-in options]\nc_link_args = ['-static']\n")
            meson_options = ["--cross-file", str(cross)]
        run(["meson", "setup", str(directory), str(source["dav1d"]), f"--prefix={prefix}", "--libdir=lib",
             "--buildtype=release", "--default-library=static", "-Denable_tools=false", "-Denable_tests=false",
             "-Denable_examples=false", "-Denable_docs=false"] + meson_options, builddir, env, logfile)
        compile_objects(["meson", "compile", "-C", str(directory), "-j", str(args.jobs)], directory, env, logfile)
        run(["meson", "install", "-C", str(directory)], builddir, env, logfile)

    print(f"[{target}] FFmpeg + ffprobe", flush=True)
    directory = builddir / "ffmpeg"
    logfile = builddir / "ffmpeg.log"
    decoders = "h264,hevc,mpeg4,mpeg1video,mpeg2video,vp8,vp9,libdav1d,prores,mjpeg,ffv1,aac,aac_latm,mp3,mp3float,mp2,mp2float,opus,libopus,vorbis,ac3,eac3,flac,alac,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_f32le,pcm_f64le,pcm_u8,pcm_alaw,pcm_mulaw,adpcm_ima_wav,adpcm_ms,rawvideo,wrapped_avframe"
    encoders = "libx264,libx264rgb,libvpx_vp9,libopus,aac,mpeg4,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,pcm_f64le,rawvideo,ffv1,wrapped_avframe"
    demuxers = "mov,avi,matroska,mpegvideo,mpegps,mpegts,h264,hevc,ivf,ogg,wav,flac,mp3,aac,rawvideo,image2,image2pipe"
    muxers = "mp4,mov,avi,webm,matroska,ivf,ogg,wav,null,framehash,framemd5,rawvideo,ffmetadata"
    parsers = "h264,hevc,mpeg4video,mpegvideo,vp8,vp9,av1,mjpeg,aac,aac_latm,mpegaudio,opus,vorbis,ac3,flac"
    filters = "anull,aformat,aresample,asetpts,atrim,concat,copy,crop,format,fps,hflip,null,pad,scale,setdar,setsar,setpts,transpose,trim,vflip,zscale,tonemap,buffer,buffersink,abuffer,abuffersink,testsrc,testsrc2,color,sine,anullsrc,silencedetect"
    bsfs = "aac_adtstoasc,h264_mp4toannexb,hevc_mp4toannexb,extract_extradata,vp9_superframe,vp9_superframe_split,vp9_raw_reorder,av1_frame_merge,av1_frame_split,filter_units,null"
    options = ["--disable-everything", "--disable-autodetect", "--enable-gpl", "--enable-version3",
               "--disable-nonfree", "--disable-shared", "--enable-static", "--disable-debug", "--disable-doc",
               "--enable-small", "--disable-network", "--disable-ffplay", "--enable-ffmpeg", "--enable-ffprobe",
               "--enable-libx264", "--enable-libvpx", "--enable-libopus", "--enable-libdav1d", "--enable-libzimg",
               "--enable-avfilter", "--enable-avdevice", "--enable-swscale", "--enable-swresample",
               "--enable-protocol=file,pipe", "--enable-indev=lavfi", "--pkg-config-flags=--static",
               f"--enable-decoder={decoders}", f"--enable-encoder={encoders}", f"--enable-demuxer={demuxers}",
               f"--enable-muxer={muxers}", f"--enable-parser={parsers}", f"--enable-filter={filters}",
               f"--enable-bsf={bsfs}", f"--cc={env['CC']}", f"--cxx={env['CXX']}",
               f"--ar={env['AR']}", f"--ranlib={env['RANLIB']}", f"--strip={env['STRIP']}",
               f"--extra-cflags=-I{prefix / 'include'}", f"--extra-ldflags=-L{prefix / 'lib'}",
               f"--extra-libs={env['STL_LIBS']}", "--extra-version=roshan-offline"]
    if target == "windows":
        options += ["--target-os=mingw32", "--arch=x86_64", "--enable-cross-compile", "--extra-ldflags=-static",
                    f"--windres={env['RC']}"]
    run([str(source["ffmpeg"] / "configure"), f"--prefix={prefix}"] + options, directory, env, logfile)
    compile_objects(["make", "-j", str(args.jobs), "ffmpeg" + (".exe" if target == "windows" else ""),
         "ffprobe" + (".exe" if target == "windows" else "")], directory, env, logfile)
    (prefix / "bin").mkdir(exist_ok=True)
    outputs = {}
    for program in ["ffmpeg", "ffprobe"]:
        filename = program + (".exe" if target == "windows" else "")
        dest = prefix / "bin" / filename
        shutil.copy2(directory / filename, dest)
        run([env["STRIP"], str(dest)], builddir, env, logfile)
        outputs[filename] = {"bytes": dest.stat().st_size, "sha256": hashlib.sha256(dest.read_bytes()).hexdigest()}
    report = {"target": target, "outputs": outputs, "network": False, "allowed_protocols": ["file", "pipe"],
              "configure": options, "encoders_requested": encoders.split(","), "decoders_requested": decoders.split(",")}
    (ROOT / "reports" / (target + "-build.json")).write_text(json.dumps(report, indent=2) + "\n")
    print(f"[{target}] COMPLETE: {prefix / 'bin'}", flush=True)
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["linux", "windows", "both"], default="both")
    parser.add_argument("--llvm-mingw", type=Path)
    parser.add_argument("--cmake", default="cmake")
    parser.add_argument("--environment", type=Path)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--from-stage", choices=["x264", "opus", "libvpx", "zimg", "dav1d", "ffmpeg"], default="x264", help="Resume using already installed earlier dependencies")
    args = parser.parse_args()
    if args.target != "linux" and not args.llvm_mingw:
        parser.error("Windows builds require --llvm-mingw")
    source = prepare_sources()
    if not (source["zimg"] / "configure").exists():
        # The official Opus release carries the standard GNU config helpers;
        # provide them when a relocated Autotools package lacks its symlinks.
        for helper in ("config.guess", "config.sub"):
            destination = source["zimg"] / helper
            if not destination.exists():
                shutil.copy2(source["opus"] / helper, destination)
        run(["sh", "autogen.sh"], source["zimg"], make_env(args, "linux"), ROOT / "build/zimg-autogen.log")
    targets = ["windows", "linux"] if args.target == "both" else [args.target]
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(targets)) as executor:
        futures = [executor.submit(build, target, args, source) for target in targets]
        for future in concurrent.futures.as_completed(futures):
            future.result()


if __name__ == "__main__":
    main()
