#!/usr/bin/env python3
"""Collect exact upstream notices for the executable's offline --licenses view."""
from pathlib import Path
import hashlib
import tomllib

ROOT=Path(__file__).resolve().parents[1]

def main():
    parts=["""ROSHAN CONVERTER 2.0.0 — THIRD-PARTY NOTICES

Programming and development by Peyman Abedinpour
برنامه نویسی و توسعه توسط پیمان عابدین پور
peymaanabedinpour.ir

Application source: GPL-3.0-or-later. Individual dependencies retain the
licenses reproduced below. Complete application sources, exact vendored Rust
sources, pristine native source archives and build scripts accompany the EXE
in the Source package. Image libraries are linked statically; the offline
FFmpeg/ffprobe worker executables are compressed inside the single EXE and
extracted privately for local execution. They require no separate installation.

Image components: libheif 1.23.6, libde265 1.1.3, x265 4.3, LittleCMS 2.19.1.
Video components and exact build licensing are listed in the video section.
Source URLs and hashes: native-build/dependencies.json and
video-native/dependencies.json. Rust source/version/license metadata: vendor/.
Rust standard library: MIT OR Apache-2.0. LLVM and Windows runtime notices follow.
"""]
    seen={}
    def include(path,label):
        data=path.read_text(encoding="utf-8",errors="replace")
        digest=hashlib.sha256(data.encode()).hexdigest()
        if digest in seen:
            parts.append(f"\n{label}: identical license text reproduced under {seen[digest]}.\n")
        else:
            seen[digest]=label
            parts.append("\n"+"="*72+"\n"+label+"\n"+"="*72+"\n"+data+"\n")
    for path in sorted((ROOT/"native-build/licenses").iterdir()):
        if path.is_file():include(path,"Native/runtime: "+path.name)
    parts.append("\n"+"="*72+"\nRUST DEPENDENCIES\n"+"="*72+"\n")
    for crate in sorted((ROOT/"vendor").iterdir()):
        cargo=crate/"Cargo.toml"
        if not cargo.is_file():continue
        info=tomllib.loads(cargo.read_text())["package"]
        label=f"{info['name']} {info['version']} — {info.get('license','see included source license')}"
        parts.append("\n"+label+"\n")
        licenses=[p for p in crate.iterdir() if p.is_file() and p.name.upper().startswith(("LICENSE","LICENCE","COPYING","NOTICE","UNLICENSE"))]
        for path in sorted(licenses):include(path,label+" / "+path.name)
    include(ROOT/"video-native/THIRD_PARTY_NOTICES.txt","BUNDLED VIDEO EXECUTABLES AND CODECS")
    (ROOT/"THIRD_PARTY_NOTICES.txt").write_text("".join(parts),encoding="utf-8")
    print("Updated complete offline third-party notices.")

if __name__=="__main__":main()
