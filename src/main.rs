#![cfg_attr(windows, windows_subsystem = "windows")]

mod engine;
mod metadata;
mod color;
mod safe_output;
mod image_engine;
mod video_engine;
mod video_tools;
#[cfg(windows)]
mod gui;

use std::{ffi::OsString, path::PathBuf, sync::atomic::AtomicBool};

fn cli(args: Vec<OsString>) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
    if args.iter().any(|s| s == "--version") {
        println!("Roshan Converter 2.0.0 | {} | offline image + video conversion", engine::decoder_version());
        return Ok(());
    }
    if args.iter().any(|s| s == "--licenses") {
        println!("{}\n\n{}", include_str!("../LICENSE"), include_str!("../THIRD_PARTY_NOTICES.txt"));
        return Ok(());
    }
    if args.iter().any(|s| s == "--help" || s == "-h") {
        println!("Roshan Converter 2.0.0\n\nOpen without arguments for the English/Persian Windows interface.\n\nCLI:\n  Roshan-HEIC.exe --convert --to png [--output DIR] [--strip-exif] [--report FILE] FILE...\n  Roshan-HEIC.exe --convert --image-format webp --video-format mp4 FILE...\n  Roshan-HEIC.exe --version\n  Roshan-HEIC.exe --licenses\n\nImages: HEIC/HEIF, PNG, JPG/JPEG, WebP. Video: MP4, MOV, AVI, WebM.\n--to applies to the corresponding media type; mixed batches use separate image/video targets.\nDefaults: JPG for images and MP4 for videos, in a Converted subfolder beside each input.\nOriginal dimensions; JPEG 100 / 4:4:4; lossless PNG, WebP and 8-bit HEIC encoding.\nCompatible video streams are copied; other codecs use high-quality transcoding.\nExisting files are never replaced. ICC color data stay even with --strip-exif.\nAll codecs are bundled. The Windows app extracts verified video workers temporarily, offline.\n\nProgramming and development by Peyman Abedinpour — peymaanabedinpour.ir\n");
        return Ok(());
    }
    let mut settings = engine::ConversionSettings::default();
    let mut inputs = Vec::new();
    let mut report_path: Option<PathBuf> = None;
    let mut values = args.into_iter();
    let mut literal = false;
    while let Some(a) = values.next() {
        if literal { inputs.push(PathBuf::from(a)); continue; }
        match a.to_str() {
            Some("--") => literal = true,
            Some("--convert") => {},
            Some("--output") => settings.output_dir = Some(PathBuf::from(values.next().ok_or("--output needs a directory")?)),
            Some("--report") => report_path = Some(PathBuf::from(values.next().ok_or("--report needs a file path")?)),
            Some("--strip-exif") => settings.preserve_metadata = false,
            Some("--to") => {
                let value=values.next().ok_or("--to needs a format")?.to_string_lossy().into_owned();
                if let Some(format)=engine::ImageFormat::parse(&value) {settings.image_format=format;}
                else if let Some(format)=engine::VideoFormat::parse(&value) {settings.video_format=format;}
                else {return Err(format!("Unsupported output format: {value}"));}
            },
            Some("--image-format") => settings.image_format=engine::ImageFormat::parse(&values.next().ok_or("--image-format needs a format")?.to_string_lossy()).ok_or("Choose heic, png, jpg or webp")?,
            Some("--video-format") => settings.video_format=engine::VideoFormat::parse(&values.next().ok_or("--video-format needs a format")?.to_string_lossy()).ok_or("Choose mp4, mov, avi or webm")?,
            Some(s) if s.starts_with("--") => return Err(format!("Unknown option: {s}")),
            _ => inputs.push(PathBuf::from(a)),
        }
    }
    if inputs.is_empty() { return Err("Choose one or more supported image or video files".into()); }
    let cancel = AtomicBool::new(false);
    let mut reports = Vec::new();
    let mut errors = 0;
    for input in inputs {
        match engine::convert_file(&input, &settings, &cancel) {
            Ok(outputs) => {
                let images: Vec<_> = outputs.iter().map(|x| {
                    println!("{} -> {} ({}x{})", input.display(), x.path.display(), x.width, x.height);
                    serde_json::json!({"path":x.path, "width":x.width,"height":x.height,"bytes":x.bytes,"warnings":x.warnings})
                }).collect();
                reports.push(serde_json::json!({"input":input,"status":"ok","outputs":images}));
            },
            Err(error) => {
                errors += 1;
                eprintln!("{}: {error}", input.display());
                reports.push(serde_json::json!({"input":input,"status":"error","error":error}));
            }
        }
    }
    if let Some(path) = report_path {
        let report = serde_json::json!({"app":"Roshan Converter", "version":"2.0.0", "decoder":engine::decoder_version(),"image_output":settings.image_format.extension(),"video_output":settings.video_format.extension(),"preserve_metadata":settings.preserve_metadata,"files":reports});
        let bytes = serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?;
        // CLI reports also require a new destination, so they cannot clobber a source.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|e|format!("Cannot create report {}: {e}",path.display()))?;
        file.write_all(&bytes).map_err(|e|e.to_string())?;
    }
    if errors > 0 { Err(format!("{errors} file(s) could not be converted")) } else { Ok(()) }
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    #[cfg(windows)]
    let result = if args.iter().all(|a| !a.to_string_lossy().starts_with("--") && a != "-h") {
        gui::run()
    } else { cli(args) };
    #[cfg(not(windows))]
    let result = cli(args);
    video_tools::cleanup_runtime();
    if let Err(error) = result {
        eprintln!("{error}");
        #[cfg(windows)]
        if std::env::args_os().len() == 1 {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "Roshan Converter".encode_utf16().chain(Some(0)).collect();
            unsafe { MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR); }
        }
        std::process::exit(1);
    }
}

