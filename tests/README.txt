Native converter verification (V2)
==================================

The source distribution includes these test programs and a fixture manifest;
it does not include upstream photos, decoded pixel dumps, or downloaded test
data. Fixture use is for local regression verification. Source URL, immutable
tag/commit, exact size and SHA-256 are recorded in fixture_manifest.json.

To download the public test data (network is used by this developer command
only, never by the offline application):

  python tests/download_fixtures.py --out ../heic-test-fixtures

To compile and verify the bridge against already built Linux native codecs:

  python tests/verify_bridge.py --native-dir native-prefix/linux \
    --fixtures ../heic-test-fixtures --build-dir ../heic-test-build

Dependencies: Python 3, g++ (C++17 for the bridge), and the pinned native prefix
containing libheif 1.23.6, libde265 1.1.3, x265 4.3 and Little CMS 2.19.1. The native
libheif source itself requires C++20. See native-build/build_native.py for
building the libraries. No photo is written by verify_bridge.py.

HEIC output and transparency tests (no photos/downloads required):

  python tests/verify_roundtrip.py --native-dir native-prefix/linux \
    --build-dir ../heic-roundtrip-build

The synthetic heic_roundtrip.cpp is compiled against the same bridge and
static codec libraries. Nine real encode/decode roundtrips test:
- RGB8 and RGBA8, including hidden RGB at alpha=0, partial and full opacity.
- 1x1, 2x3, 29x100, 73x67, 128x128 and 257x193 dimensions, without crop loss.
- Actual x265 lossless coding and full-range GBR identity matrix; all RGB
  and alpha samples compare exactly after decoding; 4:4:4 planes and 8-bit
  output precision are checked.
- A custom P3/gamma2.2 ICC and canonical sRGB ICC are preserved byte-for-byte.
- Normalized TIFF EXIF (width, height, Orientation=1) is preserved byte-for-byte.
- Independent simultaneous encoders with different image dimensions.
- Linear-light white alpha composition, unchanged opaque samples, rejected
  invalid ICC/pixel buffers, and deterministic canonical sRGB profile bytes.

The same program is cross-compiled and run with Wine 11. For a standard
Wine installation the script accepts --target windows --cxx /path/to/clang++
--runner /path/to/wine. An optional --runner-env JSON supplies a configured
runner environment. --llvm-readobj /path/to/llvm-readobj checks imports.
Compiler runtime environment/TMPDIR can be provided by the invoking shell.
roundtrip_linux_report.json and roundtrip_windows_report.json contain the
observed results, hashes of bridge/header/test/native libraries, and stdout.
The Windows report also records system-only DLL imports. Wine emitted
nonfatal x265 NUMA affinity diagnostics; all assertions and exit status pass.

Actual Rust application image matrix:

  python tests/v2_image_matrix.py --cli target/release/Roshan-HEIC \
    --native-dir native-prefix/linux --work-dir ../roshan-image-matrix \
    --report verification/images-v2.json

Requires Pillow and NumPy in addition to the native C++ toolchain. The script
generates its own mathematical test patterns, tagged ICC/EXIF data and PNG
chunks; no photographic fixtures or network access are needed. Each run uses
a new directory under --work-dir. The recorded V2 run passed 68 CLI commands
across 24 groups: all 12 directed HEIC/PNG/JPG/WebP conversions, same-format
byte copies, 24 EXIF orientation conversions, transparency/hidden RGB,
JPEG100/4:4:4 and linear white, 16-bit PNG preservation, large EXIF limits,
PNG cICP color precedence, legacy gamma, invalid color/input rejection,
no-overwrite publication, Unicode paths and batch continuation after error.
verification/images-v2.json includes current source and binary SHA-256s,
per-command reports and measured pixel errors. PNG/WebP/HEIC lossless paths
matched source samples exactly. Comparing two JPEG decoders permits at most
two levels of normal rounding variation; JPEG re-encoding remains lossy.

To exercise the Windows application executable with the same complete matrix:

  python tests/v2_image_matrix.py --cli release/Roshan-HEIC.exe \
    --native-dir native-prefix/linux --work-dir ../roshan-windows-image-matrix \
    --report verification/images-v2-windows.json --wine /path/to/wine \
    --wine-env /path/to/existing-wine-environment.json

The optional Wine environment is a JSON object of environment variables for an
existing test prefix. The script uses Wine's Z: host-file mapping, passes Unicode
Windows paths to the real EXE, and inspects its resulting files on the host.
The Linux native prefix builds an inspection helper only; every tested conversion
is performed by the Windows EXE and its embedded static codecs. The recorded
Windows run also passed all 68 commands and 24 groups. Both reports include the
exact executable SHA-256. Generated cICP ICC comparison ignores only the ICC
creator-platform header (MSFT on Windows versus APPL on Linux); all color data
must match byte-for-byte. Source ICC pass-through comparisons ignore no bytes.

After a GUI-only executable rebuild, v2_image_smoke.py checks four conversions
against a copied executable snapshot whose required hash is supplied explicitly:

  python tests/v2_image_smoke.py --cli release/Roshan-HEIC.exe \
    --expected-sha256 THE_EXACT_EXECUTABLE_SHA256 --work-dir ../final-image-smoke \
    --report verification/images-v2-windows-final.json --wine /path/to/wine \
    --wine-env /path/to/existing-wine-environment.json

It verifies Persian paths, PNG -> HEIC -> PNG exact RGBA including hidden RGB,
PNG -> WebP exact RGBA, ICC/EXIF preservation and JPEG100/4:4:4/linear-white.
This final artifact smoke supplements the full matrix; the reports preserve
separate hashes for the fully tested pipeline build and the final GUI rebuild.

Recorded verification on 2026-10-17:
- All 14 public input files decoded successfully into 16 top-level still images.
- The test checks original decoded dimensions (including HEIF crop/rotation),
  exact packed RGB buffer length, output ICC presence, expected flags, and
  frame count. See bridge_report.json.
- Included real 10-bit BT.2020 HLG photo converted to 4240x2832 SDR sRGB,
  inspected visually (sky, shore and water retain natural detail and color).
- Included odd dimensions, a 1x1 crop, a 64x64 clean-aperture crop, 10-bit
  RGB, 12-bit RGBA, alpha on white, two-image spatial HEIC, Sony HIF,
  iPhone rotation, EXIF-only mirror/rotation, and oversized EXIF metadata.
- RGBA_12__29x100.heif is expected to report flags14: alpha composited (2),
  source precision reduced to RGB8 (4), and missing source color data assumed
  (8). This is a successful conversion with limitations made explicit.
- The Sony guitar sample reports a libheif decoder warning (flag16) in
  addition to high precision and orientation. The warning is retained.
- colour_math.cpp independently tests PQ EOTF reference values, HLG inverse,
  the published BT.2020-to-sRGB matrix coefficients, the generated sRGB ICC
  transfer curve, correct premultiplied linear-light alpha, 10-bit range
  scaling, and neutrality/monotonicity of the HDR tone-map gray ramp.

Orientation boundary:
The bridge applies HEIF irot/imir/clap properties. Flag32 marks an explicit
irot/imir property. Raw HEIF EXIF is returned separately; Rust applies the
remaining EXIF orientation only when flag32 is absent, then writes EXIF1.
The mirrored EXIF-only sample has a valid 1209x1547 clean-aperture result,
despite older metadata saying1210x1548. Rust's final image is1547x1209.

Color boundary:
SDR RGB ICC is preserved. Supported NCLX primaries/transfer curves are
represented by an equivalent generated ICC. Unknown metadata assumptions
are surfaced. Gray ICC is converted to RGB sRGB. rh_decode flattens alpha
onto white in linear light. rh_decode_rgba returns unassociated RGBA8 and
retains transparency for PNG/WebP/HEIC paths. The standalone flatten helper
uses the same ICC-aware linear math for PNG/WebP-to-JPEG conversion. PQ and
HLG are explicitly rendered to ordinary SDR
sRGB using the deterministic formula documented in native/bridge.h.
This does not preserve HDR brightness/gamut or 10/12-bit precision inside
ordinary 8-bit JPEG. JPEG quality100 with4:4:4 remains a lossy encoding.
Auxiliary depth/gain-map images and Live Photo video are not exported.

HEIC encoding uses lossless 8-bit 4:4:4 with RGB/GBR identity, preserving the
supplied 8-bit samples and alpha. A supplied valid RGB ICC is embedded
unchanged. Unknown NCLX primaries/transfer accompany that authoritative ICC,
avoiding a false sRGB/P3 label. With no ICC, sRGB is assumed and explicitly
flagged. This mode can be larger and have less hardware-decoder/viewer
compatibility than common lossy 4:2:0 HEIC. It does not preserve source HDR
or higher precision that was already converted by an earlier pipeline step.
Pixel limits are 200 megapixels and 65,535 per dimension for decoding,
142,606,336 coded pixels (including padding) for x265 HEIC output. Codec
allocation/format constraints can reject otherwise excessive dimensions.

PNG cICP:
Full-range SDR RGB cICP data take precedence over iCCP, sRGB, gAMA and cHRM,
as specified by PNG Third Edition 11.3.2.6. rh_nclx_profile creates a matching
ICC for supported primaries/transfer functions without altering pixel samples.
Display-P3 (12,13,0,1), SDR BT.2020 and sRGB cases are checked. Malformed CRC,
duplicate chunks, unsupported color codes, nonzero RGB matrix coefficients,
narrow-range PNG and PQ/HLG PNG transcoding are explicitly rejected to avoid
incorrect color tagging. Same-format original PNG copy preserves those files.
PNG gAMA uses its inverse exponent for an ICC decoding curve. Alpha compositing
under gAMA=1 is checked against linear source-coordinate values.

Primary implementation references:
https://github.com/strukturag/libheif/releases/tag/v1.23.6
https://github.com/strukturag/libheif/blob/v1.23.6/libheif/api/libheif/heif_decoding.h
https://github.com/strukturag/libheif/blob/v1.23.6/libheif/api/libheif/heif_image.h
https://github.com/strukturag/libheif/blob/v1.23.6/libheif/api/libheif/heif_color.h
https://github.com/strukturag/libheif/blob/v1.23.6/libheif/api/libheif/heif_properties.h
https://github.com/strukturag/libheif/blob/v1.23.6/libheif/api/libheif/heif_metadata.h
https://github.com/mm2/Little-CMS/releases/tag/lcms2.19.1
https://github.com/Multicorewareinc/x265/releases/tag/4.3
https://www.w3.org/TR/png-3/#cICP-chunk
https://github.com/strukturag/libheif/blob/v1.23.6/examples/heif_enc.cc
https://github.com/strukturag/libheif/blob/v1.23.6/libheif/plugins/encoder_x265.cc
https://github.com/ente/test-fixtures/blob/e2b65015d855aaee98e3d66be74dfcd681c38c05/media/heic/v1/manifest.json

Practical limits:
The observed native Linux results verify the same bridge sources and codec
versions used in the Windows build. They do not by themselves prove native
Windows GUI behavior, Windows color management, or compatibility with every
possible camera/HEIF extension. Windows build/UI checks are reported separately.
