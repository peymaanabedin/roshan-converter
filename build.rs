use std::{env, fs, io::Write, path::PathBuf, process::Command};
use sha2::{Digest,Sha256};

fn embed_video_tools() {
    println!("cargo:rerun-if-env-changed=ROSHAN_FFMPEG_DIR");
    let directory=PathBuf::from(env::var_os("ROSHAN_FFMPEG_DIR").unwrap_or_else(||"video-native/windows/bin".into()));
    let out=PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut declarations=String::new();
    for (name,prefix) in [("ffmpeg","FFMPEG"),("ffprobe","FFPROBE")] {
        let source=directory.join(format!("{name}.exe"));
        println!("cargo:rerun-if-changed={}",source.display());
        let bytes=fs::read(&source).unwrap_or_else(|e|panic!("Build the bundled offline video tools first ({}): {e}",source.display()));
        assert!(bytes.starts_with(b"MZ"),"Invalid Windows video engine payload");
        let hash:[u8;32]=Sha256::digest(&bytes).into();
        let destination=out.join(format!("{name}.exe.gz"));
        let mut encoder=flate2::write::GzEncoder::new(fs::File::create(&destination).unwrap(),flate2::Compression::best());
        encoder.write_all(&bytes).unwrap();encoder.finish().unwrap();
        declarations.push_str(&format!("pub static {prefix}_GZ:&[u8]=include_bytes!({:?});\npub const {prefix}_SIZE:u64={};\npub const {prefix}_SHA:[u8;32]={hash:?};\n",destination,bytes.len()));
    }
    fs::write(out.join("video_payloads.rs"),declarations).unwrap();
}

fn main() {
    let target = env::var("TARGET").unwrap();
    let is_windows = target.contains("windows");
    let native = PathBuf::from(env::var_os("ROSHAN_NATIVE_DIR").unwrap_or_else(|| {
        if is_windows { "native-prefix/windows".into() } else { "native-prefix/linux".into() }
    }));
    println!("cargo:rerun-if-env-changed=ROSHAN_NATIVE_DIR");
    println!("cargo:rerun-if-env-changed=ROSHAN_CXX_RUNTIME");
    println!("cargo:rerun-if-env-changed=ROSHAN_WINDRES");
    for f in ["native/bridge.cpp", "native/bridge.h", "assets/app.manifest", "assets/app.rc", "assets/app.ico"] {
        println!("cargo:rerun-if-changed={f}");
    }
    cc::Build::new()
        .cpp(true)
        .cpp_link_stdlib(None)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-fno-strict-aliasing")
        .define("LIBHEIF_STATIC_BUILD", None)
        .define("LIBDE265_STATIC_BUILD", None)
        .include(native.join("include"))
        .file("native/bridge.cpp")
        .compile("roshan_bridge");
    println!("cargo:rustc-link-search=native={}", native.join("lib").display());
    println!("cargo:rustc-link-lib=static=heif");
    println!("cargo:rustc-link-lib=static=x265");
    println!("cargo:rustc-link-lib=static=de265");
    println!("cargo:rustc-link-lib=static=lcms2");
    if is_windows {
        embed_video_tools();
        println!("cargo:rustc-link-lib=advapi32");
        let runtime = env::var("ROSHAN_CXX_RUNTIME").unwrap_or_else(|_| "libc++".into());
        if runtime == "libc++" {
            for lib in ["c++", "c++abi", "unwind", "winpthread"] {
                println!("cargo:rustc-link-lib=static={lib}");
            }
        } else {
            for lib in ["stdc++", "gcc", "winpthread"] {
                println!("cargo:rustc-link-lib=static={lib}");
            }
        }
        println!("cargo:rustc-link-arg=-static");
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app-resource.o");
        let tool = env::var_os("ROSHAN_WINDRES")
            .unwrap_or_else(|| "x86_64-w64-mingw32-windres".into());
        let status = Command::new(tool)
            .args(["-I", "assets", "-i", "assets/app.rc", "-O", "coff", "-o"])
            .arg(&out)
            .status()
            .expect("Windows resource compiler is required");
        assert!(status.success(), "Could not compile the application icon/manifest");
        println!("cargo:rustc-link-arg={}", out.display());
    } else {
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m");
    }
}

