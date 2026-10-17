# Roshan Converter

**A native Windows image and video converter written in Rust.**

Developed by **[Peymaan Abedinpour — پیمان عابدین پور](https://peymaanabedinpour.ir)**  
**Website: [peymaanabedinpour.ir](https://peymaanabedinpour.ir)**  
**GitHub: [@peymaanabedin](https://github.com/peymaanabedin)**

**English** · [فارسی](README.fa.md) · [Windows downloads][releases] · [Report an issue][issues]

Roshan Converter is a portable desktop application for converting individual files or batches of images and videos, entirely on your computer. It combines a compact native Windows interface, drag-and-drop input, a file picker, and English/Persian language support.

The application is **written in Rust**. Native media libraries and bundled FFmpeg tools provide the codec implementations. This project brings together Rust application development, Windows integration, media processing, and a bilingual interface in a practical, inspectable open-source application.

## What you can do

- Convert still images between **HEIC, PNG, JPG/JPEG, and WebP**.
- Convert videos between **MP4, MOV, AVI, and WebM**.
- Add one file or a batch through drag-and-drop or the file picker.
- Review a drop summary showing accepted files, duplicates, and unsupported or unavailable items.
- Use separate image and video output selectors in a mixed queue.
- Follow progress, cancel a running conversion, and retry an item.
- Choose an output folder and use the available metadata option.
- Start in English and switch to Persian with a right-to-left layout.
- Run the downloadable Windows application offline, with its media components included.

The interface adapts to the media in the queue. “Smart” behavior means context-sensitive controls, format classification, and conversion decisions.

## Download and run

Open **[Releases][releases]** and download the Windows executable for the version you want.

| Release asset | Purpose |
| --- | --- |
| `Roshan-Converter-v2.0.0-Windows-x64.exe` | Portable Windows application |
| `Roshan-Converter-v2.0.0-Portable.zip` | The same executable in a ZIP archive |
| `Roshan-Converter-v2.0.0-Source.zip` | Complete source package and accompanying build materials |

The application targets **Windows 10 and Windows 11, x64**. End users do not need to install Rust, a separate FFmpeg package, or a codec pack to perform conversions. Choose an output directory you can write to and allow enough free space for converted media and temporary working files.

For release 2.0.0, the recorded executable has file version **2.0.0.0** and size **24,910,336 bytes**—approximately 24.9 MB, or 23.8 MiB. Verify a downloaded asset against the `SHA256SUMS.txt` accompanying its release. Later builds may have a different size and hash.

### Quick start

1. Run the `.exe`, or extract it from the portable ZIP and run it.
2. Drop your files into the window or add them through the file picker.
3. Choose **Review queue** in the import summary to configure the next steps. **Convert now** starts with the current settings.
4. Select an image output format and/or a video output format, as appropriate for the queue.
5. Choose the output folder and metadata preference, then start conversion.
6. Check the status of each item and open the output folder to use the results.

The source files are protected. Output files receive available names, and completed results are published through the output protection layer to avoid overwriting existing files.

## Supported format groups

| Media group | Accepted formats | Destination formats |
| --- | --- | --- |
| Still images | HEIC, PNG, JPG/JPEG, WebP | HEIC, PNG, JPG, WebP |
| Videos | MP4, MOV, AVI, WebM | MP4, MOV, AVI, WebM |

Each group supports the 12 directed conversions between its four distinct formats. Image-to-video creation, video frame extraction, and audio-only conversion are outside this workflow.

A video file extension identifies its container. The streams inside that container still need to be supported by the bundled build and compatible with the chosen output. Unusual or damaged files can therefore fail even when their extension is listed above.

## Quality, color, and metadata

Roshan Converter keeps source dimensions without deliberately resizing media. Encoding choices depend on the destination format and the source characteristics.

| Output or path | Behavior in the 2.0 implementation | What this means |
| --- | --- | --- |
| JPG | Quality 100 with 4:4:4 chroma sampling | High-quality JPEG output; JPEG encoding remains lossy |
| PNG | Lossless image encoding | Preserves the pixels supported by the conversion path; does not recreate information already lost in the input |
| WebP | Lossless image encoding | Supports the application's still-image RGB/RGBA workflow |
| HEIC | Lossless encoding for the supported RGB8/RGBA8 path | Applies to that 8-bit path; it is not a promise of universal HDR or higher-bit-depth preservation |
| Compatible video streams | Stream copying when the selected path allows it | Avoids re-encoding the copied stream; the output container can still differ |
| MP4/MOV video re-encoding | H.264 at CRF 16 when that encoding path is selected | High-quality lossy encoding |
| WebM video re-encoding | VP9 at CRF 18 when that encoding path is selected | High-quality lossy encoding |
| AVI video re-encoding | MPEG-4 quality 2, with an FFV1 path for the supported odd-dimension case | Encoding and compatibility depend on the selected path |

ICC profiles, EXIF information, orientation, and alpha transparency are handled where the conversion path and destination support them. The metadata control is subject to those format limits. JPEG output cannot retain an alpha channel.

The image workflow is designed for **still images**. It does not promise preservation of animations, Live Photo pairs, all images in a HEIF collection, every metadata field, or every high-bit-depth/HDR representation. A lossless encoder preserves its supported input representation; it does not make every cross-format transformation reversible.

The video workflow can preserve compatible encoded streams and can re-encode when necessary. Its supported HDR conversion path tone-maps PQ/HLG material to **BT.709 SDR**. Subtitle tracks, attachments, chapters, uncommon stream types, and exact container bytes are not covered by a blanket preservation guarantee.

## Technical architecture

### Rust application layer

Rust implements the desktop application, native Windows interface, queue behavior, conversion orchestration, and output handling. The UI uses native Win32 integration, including file selection, drag-and-drop, modal feedback, progress, and English/Persian presentation.

The original project code is organized around these responsibilities:

| Source area | Responsibility |
| --- | --- |
| `src/main.rs` | Application entry point |
| `src/gui.rs` | Native Windows UI, user interaction, and language presentation |
| `src/engine.rs` | Conversion job orchestration |
| `src/image_engine.rs` | Still-image conversion pipeline |
| `src/color.rs`, `src/metadata.rs` | Color and metadata processing |
| `src/safe_output.rs` | Collision-safe output creation and temporary-file handling |
| `src/video_engine.rs` | Video probing, conversion decisions, and processing |
| `src/video_tools.rs` | Bundled video tool preparation and execution |
| `native/bridge.cpp` | C/C++ bridge for native image components |
| `native-build/`, `video-native/` | Native dependency sources, build materials, and notices |

### Image and video components

Image conversion integrates native components including **libheif, libde265, x265, and Little CMS**, alongside the image support used by the Rust application. Video processing uses a bundled **FFmpeg/FFprobe** build and its selected codec libraries. Upstream components retain their own authorship and license notices.

This division lets the application express its UI and processing workflow in Rust while using established implementations for complex media formats. Native integration includes a foreign-function boundary; the project does not claim that every codec is implemented in Rust.

### How the single executable works

The download is one Windows `.exe`. Image dependencies are included in the release build, and compressed video worker executables are embedded in the application.

When video processing needs them, the application prepares the bundled workers in a private temporary directory, verifies their payload hashes, and invokes them locally. Temporary support files are part of this runtime design. Conversion does not download codecs or send media to a server. The developer website opens only when the user chooses its link.

The output layer keeps the original media separate from generated results, reserves available output names, and handles incomplete temporary outputs when conversion fails or is canceled.

## Building from source

Start with **[BUILD.txt](BUILD.txt)** in the complete source package. It describes the build environment and the release build sequence for this project.

The release build combines the Rust application, native image components, and embedded video workers. Keep the pinned native sources, dependency notices, Cargo lockfile, vendored Rust packages, and build scripts together when working on a matching release.

Relevant source materials include `Cargo.toml`, `Cargo.lock`, `vendor/`, `native/`, `native-build/`, `video-native/`, and `scripts/`. Building also requires the Rust and native toolchains described in the build instructions. The offline runtime does not imply that an unprepared development machine already has those toolchains.

The supplied pins and scripts document the build inputs. Bit-for-bit reproducible builds require separate verification.

## Verification status

The recorded 2.0 validation exercised the actual Windows executable under **Wine 11**, including native GUI interaction, English/Persian presentation, drag-and-drop feedback, mixed conversion queues, cancellation, retry, and output protection.

Image and video matrices covered all 12 directed cross-format pairs within each group. Those full matrices were recorded on an earlier build with the same codec implementation. The final executable subsequently received targeted image/video smoke checks, GUI checks, and Windows Rust unit tests. The hashes and detailed scope belong to the reports in **[verification/](verification/)**.

Physical Windows 10/11 validation is pending for this recorded baseline. Check the individual release notes for Windows hardware results added afterward.

## Contributing and reporting issues

Reproducible bug reports, documentation improvements, Persian/English language fixes, and focused code contributions are welcome.

For a conversion issue, include the application version, Windows version, input and output formats, expected behavior, actual result, and the visible error message. A small sample you can share publicly helps reproduce media-specific failures. For UI issues, include the selected language and display scaling.

Use **[GitHub Issues][issues]** for reports. For changes to codecs, output safety, color handling, or metadata, explain the affected conversion path and the checks performed. Keep upstream notices and source provenance with dependency changes.

## License and third-party acknowledgments

The project is distributed under **GNU GPLv3**, as provided in the existing `LICENSE` file. See that file for the exact grant, together with `THIRD_PARTY_NOTICES` and the license files accompanying the bundled components.

Release source packages include the corresponding dependency sources and build materials used for the distributed components. Preserve these materials and notices when preparing a redistribution. FFmpeg's licensing depends on its enabled components; its [upstream license explanation](https://ffmpeg.org/legal.html) provides additional context.

Application authorship and upstream component authorship are credited separately. Third-party codecs are the work of their respective projects and contributors.

## About the developer

**Programming and development by [Peymaan Abedinpour](https://peymaanabedinpour.ir).**  
**برنامه نویسی و توسعه توسط پیمان عابدین پور**

I publish Roshan Converter as part of my software portfolio to share practical work in **Rust**, native Windows development, media processing, and bilingual interface design. The source and engineering notes make the implementation available for review, learning, and improvement.

Explore my work at **[peymaanabedinpour.ir](https://peymaanabedinpour.ir)**. If you find the project useful, feedback, a reproducible issue, or a link to the project helps others discover it.

[releases]: https://github.com/peymaanabedin/roshan-converter/releases
[issues]: https://github.com/peymaanabedin/roshan-converter/issues
