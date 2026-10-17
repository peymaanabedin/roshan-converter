ROSHAN - NATIVE IMAGE CODEC BUILD (V2)
====================================

This directory contains the exact upstream source archives used for the
fully offline image codec build, SHA-256 checksums, complete license texts,
and the reproducible native-library build script. Runtime source archives
are verified before extraction and are included for offline rebuilding.

Pinned dependencies
-------------------
libheif    1.23.6  HEIF parsing, transforms, color planes and HEIC packaging
libde265   1.1.3   Built-in HEVC decoder (8/10/12-bit inputs)
x265       4.3     Built-in 8-bit HEVC encoder, including lossless 4:4:4
Little CMS 2.19.1  ICC color management and linear alpha compositing

Official release URLs, exact archive sizes/hashes and license identifiers
are recorded in dependencies.json. x265 moved its official development
repository from Bitbucket to https://github.com/Multicorewareinc/x265 in
2026. Its 4.3 release was published on 2026-07-31. The archive is the
maintainer's x265_4.3.tar.gz release asset, not a generated Git snapshot.
No upstream source files were changed. Only application bridge/build/test
sources in this project are application-specific.

The application's JPEG, PNG and WebP paths use Rust dependencies; the video
codecs and FFmpeg build are separately documented under video-native/.

Build environment used
----------------------
Windows: x86_64 llvm-mingw 20261006, UCRT flavor (Clang 23.1.3), on Linux.
Linux: system GCC, used as a native verification target.
Both: CMake 4.1.2, GNU Make, and NASM 2.16.01 for x265 SIMD assembly.
The script requires Python 3.12+, CMake and a compatible C/C++ compiler.
See the project-level instructions for Rust and the pinned toolchain.

Build the static libraries
--------------------------
From the project root, with NASM and llvm-mingw installed:

  python3 native-build/build_native.py \
    --target windows \
    --llvm-mingw /absolute/path/to/llvm-mingw \
    --nasm /absolute/path/to/nasm \
    --output native

This installs headers in native/windows/include and these static archives
in native/windows/lib:

  libheif.a
  libde265.a
  libx265.a
  liblcms2.a

Use --target linux for native tests, or --target both to build the two
platforms concurrently. --jobs applies independently to each target.
Use --cmake for an explicit CMake binary; Ninja is supported through
--generator Ninja --make-program /absolute/path/to/ninja.

--nasm enables x265's runtime-dispatched SIMD. Omitting it builds its
portable C++ primitives and requires no assembler, at reduced encoding
speed. The release uses SIMD. A build report records this choice.

If the bundled archives were deliberately removed, --download fetches
only the four pinned official tarballs, then verifies their SHA-256s.
A normal build with the included archives does not access the network.
The script fails before installation if an empty compiler object exists
in a reused build directory; delete that incomplete object and rebuild.

Configuration
-------------
All four runtime libraries are static. libde265 and x265 are built into
libheif, with runtime plugin loading disabled. All alternative codec
providers, examples, external compression, tests and tools are disabled.
x265 is an 8-bit-only static library without CLI, shared library, external
NUMA/VMAF dependencies, dynamic HDR10+, multiview or screen-content extras.
HEIC transparency uses libheif auxiliary alpha images and does not require
x265's separate multilayer alpha extension. The encoder uses fast preset,
genuine lossless mode, full-range GBR identity color matrix, 4:4:4 planes,
one frame thread and a pool of two worker threads. Little CMS uses only
its MIT core, with optional GPL plugins disabled.

The existing libde265 decoder still handles high-bit-depth source HEIC.
The bridge renders PQ/HLG to SDR and quantizes higher precision to 8 bits
with explicit flags. Lossless HEIC output preserves those supplied 8-bit
pixels; it cannot undo a prior source precision/HDR conversion. Lossless
RGB 4:4:4 HEIC can be larger and less widely supported by hardware-only
viewers than common lossy 4:2:0 HEIC.

Linking
-------
Compile the C++ bridge with LIBHEIF_STATIC_BUILD and LIBDE265_STATIC_BUILD.
Link application objects before heif, de265, x265 and lcms2:

  x86_64-w64-mingw32-clang++ application-objects... \
    native/windows/lib/libheif.a native/windows/lib/libde265.a \
    native/windows/lib/libx265.a native/windows/lib/liblcms2.a \
    -static -ladvapi32 -o Roshan.exe

The matching llvm-mingw C++ runtime is supplied by its compiler driver.
A driver that does not add the C++ runtime must link static c++, c++abi,
unwind and winpthread, with the llvm-mingw library search path. Do not mix
these Windows libc++ archives with a GCC libstdc++ runtime.

On Linux link stdc++, pthread, m and dl after the four native libraries.
No libnuma or libvmaf library is needed. Windows imports are KERNEL32,
ADVAPI32 and UCRT API-set DLLs supplied by supported Windows versions.
No external HEIC extension, codec DLL, C++ runtime installer, Python, Rust,
browser engine or network service is needed by the delivered executable.

The script supplies a writable build TMPDIR and LD_LIBRARY_PATH for the
compiler in restricted build containers. They are build settings only.
GCC's -pipe is used for x265 to avoid large temporary assembly files.

Validation
----------
native_smoke.c checks library initialization, HEVC decoder discovery,
x265 lossless-encoder discovery, ICC creation and primary-image decoding.
Application bridge tests under tests/ provide stronger checks:

  python tests/verify_roundtrip.py --native-dir native/linux \
    --build-dir ../heic-roundtrip-check

  python tests/verify_bridge.py --native-dir native/linux \
    --fixtures ../heic-test-fixtures --build-dir ../heic-decode-check

The synthetic test needs no photos or downloads. It covers nine sample-
exact RGB/RGBA encode/decode cases including alpha=0 hidden RGB, odd/tiny
sizes, custom RGB ICC, unchanged normalized TIFF EXIF, linear white alpha,
invalid input rejection and concurrent independent encoders. The same
source is also cross-compiled and executed under Wine 11. Its complete
DLL import table shows only the Windows system dependencies above.
Wine's emulated NUMA affinity API may emit nonfatal x265 diagnostics;
all synthetic pixel tests and the process exit status are verified.

The public-photo regression suite covers 14 real HEIC files and 16 still
images, including 10/12-bit, HDR, alpha, rotation, crop and multiple images.
See tests/README.txt, its fixture manifest, and the recorded JSON reports.
No upstream photos or generated photo pixels belong in the source bundle.

Licenses and modified builds
----------------------------
libheif and libde265: LGPL-3.0-or-later.
x265: GPL-2.0-or-later (commercial licensing is not used in this build).
Little CMS core: MIT.
llvm-mingw runtimes: LLVM and MinGW-w64 notices in licenses/.

The distributed application is subject to its project GPL license and
includes corresponding source, dependency archives and build instructions.
Keep these notices and sources in the source distribution. They enable
rebuilding or relinking the application with modified dependencies.
The script verifies pristine archive hashes before extraction. Extracted
sources under native-build/sources may be modified for a custom build.
Delete affected generated build directories before changing compilers.

The source archive and build helpers are for development/rebuilding. They
are not installed or accessed by the single-EXE application at runtime.
