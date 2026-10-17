ROSHAN CONVERTER 2.0.0
======================
Programming and development by Peyman Abedinpour
برنامه نویسی و توسعه توسط پیمان عابدین پور
peymaanabedinpour.ir

A compact native Windows image and video converter, written in Rust.
Runs on Windows 10/11 x64. Open the single EXE; no installation, account,
codec pack, .NET, WebView, internet connection or separate FFmpeg is needed.

QUICK START
1. Open Roshan-HEIC.exe. The interface starts in English.
2. Choose Add files or drop one or more files onto the window.
   Dropping shows a modal with the accepted files, skipped items and output
   choices. Choose Convert now or Review queue.
3. Choose the image and/or video output format. Both media types can share a
   queue. The app shows only the output selectors relevant to the queue.
4. Keep the default Converted subfolder beside each source, or choose one
   destination folder. Existing files are never overwritten.
5. Choose Convert. Each row shows progress and its outcome. Select a row for
   output paths and details, then Open output to find the finished files.

The Persian button switches the interface immediately. The English button
switches back. About includes the developer credit and an optional website
button. Visiting that website is a separate user action; conversion is offline.
Change an output setting to reuse completed items in the queue.

SUPPORTED CONVERSIONS
Images: HEIC/HEIF, PNG, JPG/JPEG and WebP to any of HEIC, PNG, JPG or WebP.
Videos: MP4, MOV, AVI and WebM to any of MP4, MOV, AVI or WebM.
Images and videos have separate output selectors; converting a video into a
still image or an image into a video is outside this version's workflow.

IMAGE QUALITY
- Original dimensions are retained, including the displayed orientation.
- JPG uses quality 100, 4:4:4 sampling and optimized Huffman coding. Ordinary
  JPG is lossy and uses 8-bit channels, even at quality 100.
- PNG and WebP use lossless encoding. PNG-to-PNG retains 16-bit samples when
  present; new WebP and HEIC encodes use 8-bit channels.
- HEIC output uses x265 lossless RGB 4:4:4. Its 8-bit RGB/RGBA samples are
  preserved exactly in the tested roundtrips. Some external HEIC viewers
  support fewer HEIC profiles than this converter.
- Transparency is retained in PNG, WebP and HEIC. For JPG it is composited
  onto white in linear light using the image's ICC profile.
- RGB ICC profiles and supported SDR color descriptions are preserved or
  represented by matching ICC profiles. Grayscale ICC data are color-managed.
  Tagged CMYK/non-RGB inputs outside this workflow report an explicit error.
- HEIC PQ/HLG HDR is tone-mapped to SDR, and high-bit-depth HEIC decoding
  currently produces 8-bit output. This is not archival HDR preservation.
  HDR PNG transcoding is explicitly rejected; PNG output can copy it intact.
- Supported EXIF is copied when Keep metadata is on, with orientation and
  dimensions normalized. This may include GPS. Turn the option off to omit
  EXIF and transferable video metadata; ICC color profiles remain.
  The option does not promise to retain every camera-specific tag or XMP/IPTC.
- HEIC top-level still images are exported primary first with _01, _02, etc.
  Depth maps, gain maps and embedded motion tracks are not separate outputs.
- Animated PNG/WebP conversion exports the default/first still frame and
  reports that limitation. Same-format copies retain the complete original.
- Image inputs are limited to 512 MiB and 200 megapixels. JPG has a maximum
  side length of 65,535 pixels; WebP 16,383. Images are not silently resized.

VIDEO QUALITY
Compatible encoded streams are copied into the destination container whenever
possible, preserving their compressed picture/audio data. Otherwise:
- MP4/MOV: H.264, CRF 16, medium preset; AAC up to 320 kbit/s for audio needing
  conversion. Compatible audio tracks are copied.
- WebM: VP9, CRF 18, quality mode; Opus for audio needing conversion.
- AVI: MPEG-4 Part 2 at quality 2, with PCM audio. Odd dimensions use FFV1.
The picture dimensions are verified before publication. There is no deliberate
resize or frame-rate reduction. AVI may require constant-rate timing.
HDR video is retained during compatible stream copy, or explicitly tone-mapped
to SDR when its video codec must change. Re-encoding is normally lossy.

The first main video view and all supported audio tracks (up to 32) are mapped.
Additional video views, subtitles, attachments and auxiliary tracks are reported
as omitted during a transcode/remux. Same-format copies keep the whole source.
Common codecs are supported, including H.264, HEVC, MPEG-1/2/4, ProRes, MJPEG,
VP8/VP9 and AV1. A container extension is not a guarantee for every historical,
proprietary, encrypted or damaged codec variant. Failed files are isolated;
other queue items can finish. New WebM Opus tracks support up to eight channels.

FILES, CANCELLATION AND PORTABILITY
Original files are never edited or deleted. Outputs are written to temporary
files in the destination, completed and then published without replacing an
existing path. Name collisions become (2), (3), etc. Cancel interrupts video
workers and cleans unfinished output. An image operation may finish its native
codec step before cancellation is observed. Completed outputs are retained.

The EXE contains all codec payloads. Image libraries are linked into it; the
video workers are compressed inside it. The first video operation extracts
verified copies into a private temporary directory, with SHA-256 validation.
The app attempts to remove them on normal exit. No companion files are needed
beside the distributed EXE and nothing is downloaded. Abrupt power loss or a
forced process termination may leave a temporary directory that Windows can
later clean. The embedded video engines have network protocols disabled.

SHORTCUTS
Ctrl+O: Add files. Ctrl+Enter: Convert. Delete: Remove selected queue items.
Esc: Cancel conversion. F1: About. The interface is resizable and DPI aware.

COMMAND LINE
Roshan-HEIC.exe --convert --to webp --output C:\Converted photo.heic photo.png
Roshan-HEIC.exe --convert --image-format png --video-format mp4 photo.heic clip.mov
Roshan-HEIC.exe --convert --to webm --report report.json clip.mp4
Roshan-HEIC.exe --help
Roshan-HEIC.exe --licenses
Reports are JSON and require a new filename. A failed input returns a nonzero
exit status, after processing the other inputs. Source code, dependency sources,
licenses, build recipes and verification reports accompany the Source ZIP.
See BUILD.txt for the offline build instructions; see verification/ for the
recorded checks and their actual Windows-under-Wine execution environment.

TESTING LIMIT
The actual Windows EXE was tested under Wine 11, not on physical Windows.
Conversion, interface and cleanup observations are recorded separately. Some
Wine test sessions showed temporary artifacts reappearing after an execution
session ended, even when removal had been observed within that session. The
reports retain this environment limitation; durable cleanup on a physical
Windows installation has not been independently verified.
