ROSHAN CONVERTER 2.0.0 — RELEASE VERIFICATION
=============================================
Programming and development by Peyman Abedinpour
peymaanabedinpour.ir

TARGET AND RUNTIME
Windows 10/11 x64; Rust x86_64-pc-windows-gnullvm.
The actual Windows PE executable was run under Wine 11 on Ubuntu 24.04 with
an isolated virtual display. Testing did not use a physical Windows machine.

The distributed app is one EXE. Its static image libraries and Rust image
codecs require no separately installed DLLs. FFmpeg and ffprobe are compressed
inside the EXE, extracted into a private temporary directory when video is
used, and verified against embedded sizes and SHA-256 hashes before execution.
The app attempts removal on normal exit. FFmpeg has networking disabled and exposes
only the file and pipe protocols. The app has no downloader, update service,
telemetry, WebView or runtime installer. The About website opens only when
the user presses its website button.

release-v2.json identifies the final executable, exact source hashes and
import evidence. The release EXE and both embedded workers import Windows
system DLLs only. See ../release/windows-imports.json and the native/video
reports. Forced termination can leave the private temporary worker folder;
Windows Job Objects stop the child workers if the parent process ends.

HOW TO READ THE BUILD HASHES
The complete Windows image and video matrices tested integrated EXE
a1d44780be89d259c98c27851a6f93e5b8fa4e30996ac5b2c4bcde9d1858d63b.
Later changes fixed GUI repaint, compact Persian button labels and About's
expand/collapse caption. A subsequent, isolated change to safe_output.rs adds
up to one second of cleanup retries for transient Windows access/sharing/lock
errors. Codec selection, image encoding, color, metadata and native/video
payloads stayed unchanged. A real Windows exclusive-handle regression test
and fresh final GUI cancellation test cover that cleanup change. Final image
and video smoke reports identify the exact shipped EXE.

The image matrix captured backend source hashes: all match the final source
except the documented safe_output.rs cleanup change. The video matrix captured
binary/payload/script hashes without a separate historical video source hash
baseline. Early GUI observations overlapped a build-file replacement and are
preliminary only; the final GUI checks use a distinct immutable EXE snapshot.
The full conversion matrix is not presented as a repeat on every rebuild.

IMAGE VALIDATION
images-v2.json and images-v2-windows.json each record 68 real CLI runs in 24
groups, using the integrated Rust converter on Linux and Windows respectively:

* All 12 directed pairs among HEIC, PNG, JPG and WebP.
* Four byte-identical same-format copies when metadata is retained.
* All eight EXIF orientations for PNG/JPG/WebP: 24 orientation conversions.
* Lossless RGBA paths, including RGB stored under fully transparent pixels.
* ICC byte preservation, normalized EXIF dimensions/orientation, 16-bit PNG.
* Quality-100 JPEG quantization tables and 4:4:4 component sampling.
* White compositing calculated in linear light and checked independently.
* 70 KiB EXIF retained outside JPEG, with explicit omission warning for JPEG.
* PNG cICP precedence, full-range SDR Display-P3/BT.2020/sRGB, gamma and color.
* Rejection of unsupported, malformed, narrow-range and HDR PNG transcoding;
  same-format original copying remains byte-identical for those valid inputs.
* Persian filenames, corrupt-input isolation, batch continuation and no overwrite.

images-v2-windows-final.json repeats four focused image commands on the final
EXE. The lossless paths compare pixel arrays, not just file-open success.
LittleCMS records the operating-system creator in generated ICC profiles;
Windows and Linux ICC comparisons normalize only that informational header.

The C++ native bridge additionally passed nine lossless HEIC roundtrip groups
on Linux and Windows. These cover alpha, hidden RGB, 1x1/odd dimensions,
ICC/EXIF and concurrent encoders. Existing decoder regression uses fourteen
public HEIC/HEIF/HIF files yielding sixteen top-level still images, including
rotations, crops, multi-image files, high bit depth and real HLG. The public
photos are not redistributed; ../tests/ retains URLs, hashes and test scripts.
See ../tests/README.txt and the bridge/roundtrip/color-math JSON reports.

VIDEO VALIDATION
videos-v2-windows.json records 27 integrated application checks across 13
Windows CLI batches. It includes:

* All 12 directed pairs among MP4, MOV, AVI and WebM, with two audio tracks.
* Preserved display dimensions, frame rate, duration and audio-track counts
  for the synthetic fixtures; no deliberate resizing by the app.
* Byte-identical compressed video packets for compatible MP4/MOV remuxes.
* 321x181 dimensions, metadata rotation and physically applied rotation.
* 10-bit PQ and HLG conversion to explicitly tagged BT.709 SDR with warnings.
* 24-bit PCM output where required, corrupt-file isolation and safe duplicates.
* Same-container original copying and preserved input bytes.
* Independent observation of extracted worker hashes and normal-exit cleanup.

videos-v2-windows-final.json records the focused final-EXE smoke. It separates
correct conversions and in-command runtime removal from a later environment
observation: the same worker directory and an original zero-byte reservation
reappeared after the execution command returned. Native Windows queries also
saw the restored artifacts. Filesystem synchronization in the executor/Wine
environment is an inference from that timing, not a proven Windows product
failure or proof of durable cleanup on physical Windows. The report preserves
both observations and their scope. Separate codec-level Linux/Windows checks
and all enabled encoders/decoders/protocols
are recorded in ../video-native/reports/. Reproduction scripts are in
../video-native/; generated test videos stay outside the source package.

NATIVE USER INTERFACE
gui-v2.json records actual application interactions, tested build hashes,
English and Persian layout checks, About credit/domain, native file selection,
mixed queues, drop-modal behavior and cancellation observations. Screenshots
named gui-v2-* are native application captures, not HTML mockups or a simulated
conversion interface. An automated DROPFILES delivery tests the native drop
handler and modal; it does not establish a physical Explorer drag gesture.

RUST UNIT TESTS
Six Rust tests passed on Linux. Seven passed on Windows, including a new
regression holding a real Windows file handle that prevents deletion, then
releasing it after 150 ms. Temporary cleanup must wait and remove the file.
The other checks cover file classification/defaults, compatible video stream
copying, bounded EXIF parsing and both byte orders, large EXIF limits,
collision-safe publication and ordinary temporary-output cleanup.
See rust-unit-tests-v2.* and rust-unit-tests-windows-v2.*. The Linux run predates
the Windows-specific retry; the Windows tests use the final cleanup source.

FIDELITY AND COVERAGE LIMITS
JPEG quality 100 remains lossy. PNG/WebP encoding is lossless for the supplied
decoded samples; the HEIC encoder preserves the supplied RGB8/RGBA8 samples
using lossless 4:4:4 coding. This cannot recover earlier compression loss.
High precision, HDR, alpha and animation handling are documented precisely in
README-EN.txt and README-FA.txt. Lossless RGB 4:4:4 HEIC may be larger and less
widely supported by viewers than typical lossy 4:2:0 HEIC.

Compatible video/audio streams are copied without recompression. Necessary
video/audio transcoding uses fixed high-quality settings and is generally
lossy. Container support does not imply every possible codec, subtitle or
auxiliary track is preserved. The app reports material conversions/omissions.

These fixtures and checks do not establish support for every camera variant,
every malformed media file, every Windows configuration or every codec within
a supported container. Publisher signing and a physical Windows hardware run
were not performed. No independent antivirus certification is claimed.
