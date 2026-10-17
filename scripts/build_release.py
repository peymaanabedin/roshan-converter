#!/usr/bin/env python3
"""Build the single Windows x64 EXE from the included, pinned offline sources.

This build recipe runs on Linux with Rust 1.99.1 and llvm-mingw 20261006.
The resulting EXE runs on Windows 10/11 x64 without build tools or extra DLLs.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-gnullvm"

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--llvm-mingw", type=Path, required=True)
    p.add_argument("--native-prefix", type=Path, help="Reuse built native/windows directory")
    p.add_argument("--video-prefix", type=Path, help="Reuse a Windows video payload directory containing bin/ffmpeg.exe and bin/ffprobe.exe")
    p.add_argument("--cmake", default="cmake")
    p.add_argument("--cargo", default="cargo")
    p.add_argument("--jobs", type=int, default=4)
    a=p.parse_args()
    llvm=a.llvm_mingw.resolve()
    native=(a.native_prefix or ROOT/"native-prefix/windows").resolve()
    if a.native_prefix is None:
        subprocess.run([sys.executable,str(ROOT/"native-build/build_native.py"),
            "--target","windows","--llvm-mingw",str(llvm),"--cmake",a.cmake,
            "--jobs",str(a.jobs),"--output",str(ROOT/"native-prefix")],check=True)
    for name in ["libheif.a","libde265.a","libx265.a","liblcms2.a"]:
        if not (native/"lib"/name).is_file(): p.error(f"Missing {native/'lib'/name}")
    video=(a.video_prefix or ROOT/"video-native/windows").resolve()
    if not all((video/"bin"/name).is_file() for name in ["ffmpeg.exe","ffprobe.exe"]):
        if a.video_prefix: p.error(f"Video payloads are missing in {video/'bin'}")
        subprocess.run([sys.executable,str(ROOT/"video-native/build_video.py"),"--target","windows",
            "--llvm-mingw",str(llvm),"--cmake",a.cmake,"--jobs",str(a.jobs)],check=True)
    env=dict(os.environ)
    tmp=ROOT/"target/build-tmp";tmp.mkdir(parents=True,exist_ok=True)
    env.update({
        "ROSHAN_NATIVE_DIR":str(native),
        "ROSHAN_FFMPEG_DIR":str(video/"bin"),
        "ROSHAN_CXX_RUNTIME":"libc++",
        "ROSHAN_WINDRES":str(llvm/"bin/x86_64-w64-mingw32-windres"),
        "CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER":str(llvm/"bin/x86_64-w64-mingw32-clang++"),
        "CC_x86_64_pc_windows_gnullvm":str(llvm/"bin/x86_64-w64-mingw32-clang"),
        "CXX_x86_64_pc_windows_gnullvm":str(llvm/"bin/x86_64-w64-mingw32-clang++"),
        "AR_x86_64_pc_windows_gnullvm":str(llvm/"bin/llvm-ar"),
        "LD_LIBRARY_PATH":str(llvm/"lib")+(os.pathsep+env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else ""),
        "PATH":str(llvm/"bin")+os.pathsep+env.get("PATH",""),
        "TMPDIR":str(tmp),
    })
    # Encoded flags also work when the toolchain's absolute path contains spaces.
    env.pop("RUSTFLAGS",None)
    env["CARGO_ENCODED_RUSTFLAGS"]="\x1f".join(["-L",f"native={llvm/'x86_64-w64-mingw32/lib'}","-C","target-feature=+crt-static"])
    subprocess.run([a.cargo,"build","--release","--locked","--offline","--target",TARGET],cwd=ROOT,env=env,check=True)
    target_root=Path(env.get("CARGO_TARGET_DIR",ROOT/"target")).resolve()
    exe=target_root/TARGET/"release/Roshan-HEIC.exe"
    objdump=llvm/"bin/llvm-objdump"
    inspected=subprocess.run([str(objdump),"-p",str(exe)],env=env,text=True,capture_output=True,check=True).stdout
    imports=re.findall(r"DLL Name:\s*(\S+)",inspected)
    allowed={"kernel32.dll","ntdll.dll","user32.dll","gdi32.dll","ole32.dll","combase.dll",
             "comdlg32.dll","uxtheme.dll","comctl32.dll","shell32.dll","bcryptprimitives.dll",
             "bcrypt.dll","advapi32.dll","shcore.dll","msvcrt.dll","ucrtbase.dll"}
    unexpected=[x for x in imports if x.lower() not in allowed and not x.lower().startswith("api-ms-win-")]
    if not imports or unexpected: raise RuntimeError(f"Unexpected runtime imports: {unexpected or inspected}")
    release=ROOT/"release";release.mkdir(exist_ok=True)
    destination=release/"Roshan-HEIC.exe"
    # Keep a previously running Wine/Windows image intact while publishing a
    # newer build; do not truncate a PE file which a QA process may have mapped.
    fd,staged=tempfile.mkstemp(prefix=".release-",suffix=".exe",dir=release)
    os.close(fd)
    try:
        shutil.copy2(exe,staged)
        os.replace(staged,destination)
    finally:
        if os.path.exists(staged):os.unlink(staged)
    (release/"windows-imports.json").write_text(json.dumps({"target":TARGET,"imports":imports,"third_party_dlls":[]},indent=2)+"\n")
    print(f"Built {destination} ({destination.stat().st_size:,} bytes). Only Windows system DLLs imported.")

if __name__=="__main__": main()
