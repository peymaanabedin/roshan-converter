# Offline video payloads

This directory builds the private FFmpeg and ffprobe executables embedded in
the Rust application. The Windows distribution remains one executable. The
application extracts the embedded programs locally for a conversion; users do
not need an FFmpeg installation, codec pack, account or internet connection.

## Pinned source and licensing

`dependencies.json` records the exact official source URL, source version,
archive SHA-256 and license for FFmpeg 9.0.2, x264, libvpx 1.17.0, Opus 1.6.1,
dav1d 1.5.4 and zimg 3.0.6. The complete, pristine archives are supplied in
`archives/`. Upstream FFmpeg's detached signature and public key are retained;
the signature verification report records the official key fingerprint.

The two FFmpeg executables are built with `--enable-gpl --enable-version3` and
are distributed under GPL version 3 or later. They never use the nonfree build
option. Full component notices are in `licenses/`, also combined into
`THIRD_PARTY_NOTICES.txt` for inclusion in the application's license viewer.
The complete upstream archives retain all per-file copyright, licensing and
patent notices. No functional source patches are applied. zimg's generated
Autotools files are produced locally; its standard GNU config helpers can be
copied from the official Opus release archive by the build script.

This software is based in part on the work of the Independent JPEG Group.

## Intended capabilities

| Destination | Video encoder | Audio encoder |
| --- | --- | --- |
| MP4 or MOV | x264 H.264, including 4:4:4 and odd dimensions | FFmpeg AAC |
| AVI | FFmpeg MPEG-4 Part 2; FFV1 available for exact dimensions/lossless pixels | PCM signed 16-bit |
| WebM | libvpx VP9, including 10-bit/12-bit support | Opus |

Input decoding includes H.264, HEVC, MPEG-1/2/4, VP8/VP9, AV1 through dav1d,
ProRes, MJPEG and FFV1. Audio decoding includes AAC, MP3, MP2, PCM, Opus,
Vorbis, AC-3/E-AC-3, FLAC and ALAC. The exact enabled components and tests are
recorded in `reports/` after a successful build. Container extensions alone do
not imply that every historical codec or encrypted stream is decodable.

PCM 24-bit, 32-bit and floating-point encoders are also included so the app can
retain appropriate audio precision. Actual encoder selection and compatible
compressed-stream copying are handled by the Rust conversion policy.

The zscale and tonemap filters provide the color transforms needed for
explicit HDR-to-SDR conversion. Rotation, mirroring, crop, scaling, pixel
format conversion and audio resampling filters are included. Conversion
policy, warnings, stream selection and atomic file publication belong to the
Rust engine. The codec payload itself does not silently decide them.

All network code is disabled in the FFmpeg build. Only the `file` and `pipe`
protocols are enabled. Synthetic lavfi sources are included for verification;
no camera, microphone, screen capture or other device inputs are enabled.

## Rebuilding

Use a Linux x86-64 build host with Python 3.12 or later, a C/C++ compiler, Make,
CMake, Meson, Ninja, NASM, pkg-config, Autoconf, Automake, Libtool and standard
shell utilities. To build Windows payloads, also unpack the pinned LLVM-MinGW
toolchain described in the project's top-level `BUILD.txt`.
Build tools are needed only by developers, never by application users.

From the project source directory:

```sh
python3 video-native/build_video.py --target both \
  --llvm-mingw /absolute/path/to/llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64 \
  --jobs 2
```

`--target linux` builds the native verification tools only. `--target windows`
builds the Windows payloads only. `--cmake /absolute/path/to/cmake` can select a
specific CMake. `--from-stage ffmpeg` resumes after dependencies have already
been installed; other named dependency stages are also accepted.
`--environment /path/to/environment.json` optionally supplies
environment variables for relocated developer tools; ordinary build hosts do
not need it. The supplied source archives are verified before use, and no build
step downloads dependencies. Start with empty `build/`, `sources/`, `linux/`
and `windows/` directories for a clean rebuild.

Expected outputs:

* `windows/bin/ffmpeg.exe` and `windows/bin/ffprobe.exe`: static Windows x64 payloads.
* `linux/bin/ffmpeg` and `linux/bin/ffprobe`: Linux verification programs.
* `reports/windows-build.json` and `reports/linux-build.json`: exact options,
  binary byte sizes and SHA-256 hashes.

Only Windows system DLLs may be imported by the Windows outputs. Codec and C++
runtime libraries are linked statically. The Linux executables may depend on
the build host's ordinary Linux C/C++ runtime; they are developer QA tools and
are not included in the Windows runtime dependency set.

## Source package contents

Retain `archives/`, `licenses/`, `reports/`, `dependencies.json`, this README,
`THIRD_PARTY_NOTICES.txt` and `build_video.py` in the source distribution.
Also retain `toolchain.json` and the `verify_*.py` verification scripts.
Include the Windows binaries when a ready-to-rebuild Rust package is desired.
Expanded `sources/`, temporary `build/`, installed library prefixes and the
relocated `tools/root/` are reproducible workspace artifacts and need not be
duplicated in the downloadable source package. The small
`tools/ubuntu-packages.json` records the provenance of developer tools used for
this build. The Windows application's top-level documentation describes how
to rebuild and redistribute the complete corresponding source.

## Verification

`python3 video-native/verify_video.py --matrix` creates short synthetic media
and exercises the native payload across every container pair, odd dimensions,
audio and HDR. `--wine-env /path/to/wine-env.json` runs the Windows payloads in
an explicitly selected isolated Wine environment. The JSON environment must
identify `WINEPREFIX`, `WINELOADER`, `WINESERVER`, and any loader paths needed by
that developer's Wine installation. Wine is a development test tool only.

The integrated application matrix is run with:

```sh
python3 video-native/verify_app_windows.py \
  --exe /absolute/path/to/Roshan-HEIC.exe \
  --wine-env /path/to/wine-env.json \
  --report verification/videos-v2-windows.json
```

This test generates two distinct audio tracks per input, checks all twelve
directed container changes, compares copied compressed packet hashes, verifies
physical rotation, odd dimensions, tone mapping, high-depth PCM, corrupt-file
isolation and preservation of existing files. It needs NumPy for the decoded
rotation-frame comparison. The supplied final report includes the exact app
hash and test-script hashes. For a later GUI-only rebuild, the focused
`verify_runtime_windows.py --final-smoke` script accepts the same path arguments
and appends a mixed image/video smoke and observed worker SHA-256/cleanup result
without repeating the complete matrix.
