#!/usr/bin/env python3
"""Exercise the actual offline codec payloads with short synthetic media.

No network or external media are used. --wine-env selects the Windows payloads
and a caller-supplied isolated Wine environment; otherwise run Linux tools.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wine-env", type=Path)
    parser.add_argument("--matrix", action="store_true")
    args = parser.parse_args()
    target = "windows" if args.wine_env else "linux"
    qa = ROOT / "qa" / target
    fixtures = qa / "fixtures"
    output = qa / "خروجی ویدیو"
    fixtures.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    if args.wine_env:
        env.update(json.loads(args.wine_env.read_text()))
        env.pop("DISPLAY", None)
    report: dict = {"platform": target, "tools": {}, "checks": [], "cases": []}
    for tool in ("ffmpeg", "ffprobe"):
        path = ROOT / target / "bin" / (tool + (".exe" if args.wine_env else ""))
        report["tools"][path.name] = {
            "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        }

    def sanitize(text: str) -> str:
        return text.replace(str(ROOT), "<video-native>").replace(
            "Z:" + str(ROOT).replace("/", "\\"), "<video-native>")

    def invoke(tool: str, arguments: list[str | Path], name: str) -> str:
        exe = ROOT / target / "bin" / (tool + (".exe" if args.wine_env else ""))
        command = [env["WINELOADER"], str(exe)] if args.wine_env else [str(exe)]
        for item in arguments:
            if isinstance(item, Path) and args.wine_env:
                command.append("Z:" + str(item.resolve()).replace("/", "\\"))
            else:
                command.append(str(item))
        result = subprocess.run(command, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, errors="replace", timeout=60)
        entry = {"name": name, "exit_code": result.returncode,
                 "command": [sanitize(x) for x in command[1 if args.wine_env else 0:]],
                 "stderr": sanitize(result.stderr[-2000:])}
        report["cases"].append(entry)
        if result.returncode:
            raise RuntimeError(f"{name}: {result.stderr[-2500:]}")
        return result.stdout

    def encode(arguments: list[str | Path], name: str) -> None:
        invoke("ffmpeg", ["-hide_banner", "-loglevel", "error", "-nostdin", "-y",
                          "-protocol_whitelist", "file,pipe"] + arguments, name)

    def probe(path: Path, name: str) -> dict:
        return json.loads(invoke("ffprobe", ["-v", "error", "-protocol_whitelist", "file,pipe",
                                           "-show_streams", "-show_format", "-of", "json", path], name))

    def settings(fmt: str, odd: bool = False) -> list[str]:
        if fmt in ("mp4", "mov"):
            return ["-c:v", "libx264", "-crf", "16", "-preset", "medium", "-pix_fmt",
                    "yuv444p" if odd else "yuv420p", "-c:a", "aac", "-b:a", "256k"]
        if fmt == "webm":
            return ["-c:v", "libvpx-vp9", "-crf", "20", "-b:v", "0", "-deadline", "good",
                    "-cpu-used", "2", "-pix_fmt", "yuv444p" if odd else "yuv420p",
                    "-c:a", "libopus", "-b:a", "192k"]
        return (["-c:v", "ffv1", "-level", "3", "-pix_fmt", "yuv444p"] if odd else
                ["-c:v", "mpeg4", "-q:v", "2", "-pix_fmt", "yuv420p"]) + ["-c:a", "pcm_s16le"]

    def check_probe(info: dict, width: int, height: int, name: str,
                    audio: bool = True, duration: float = 1.0) -> None:
        videos = [s for s in info["streams"] if s["codec_type"] == "video"]
        audios = [s for s in info["streams"] if s["codec_type"] == "audio"]
        assert len(videos) == 1 and (videos[0]["width"], videos[0]["height"]) == (width, height), name
        assert bool(audios) == audio, name
        if audio:
            assert audios[0]["sample_rate"] == "48000" and audios[0]["channels"] == 2, name
        assert abs(float(info["format"]["duration"]) - duration) < 0.07, name
        rate = videos[0]["avg_frame_rate"].split("/")
        assert abs(int(rate[0]) / int(rate[1]) - 25.0) < 0.01, name
        report["checks"].append({"name": name, "passed": True,
                                  "width": width, "height": height, "video_codec": videos[0]["codec_name"],
                                  "pixel_format": videos[0].get("pix_fmt"), "fps": videos[0]["avg_frame_rate"],
                                  "audio_codec": audios[0]["codec_name"] if audio else None,
                                  "duration": info["format"]["duration"]})

    try:
        base = fixtures / "نمونه حرکت.mp4"
        encode(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=1",
                "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000:duration=1",
                "-ac", "2", "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709"]
               + settings("mp4") + ["-x264-params", "colorprim=bt709:transfer=bt709:colormatrix=bt709",
                                     "-shortest", base], "create_h264_aac_fixture")
        formats = ("mp4", "mov", "avi", "webm")
        sources = {"mp4": base}
        for fmt in formats[1:]:
            path = fixtures / ("نمونه حرکت." + fmt)
            encode(["-i", base, "-map", "0:v:0", "-map", "0:a:0"] + settings(fmt) + [path], "create_" + fmt)
            sources[fmt] = path
        for fmt, path in sources.items():
            check_probe(probe(path, "probe_fixture_" + fmt), 320, 180, "fixture_" + fmt)

        if args.matrix:
            for source_format, source in sources.items():
                for dest_format in formats:
                    name = source_format + "_to_" + dest_format
                    path = output / (name + "." + dest_format)
                    encode(["-i", source, "-map", "0:v:0", "-map", "0:a:0"] + settings(dest_format) + [path], name)
                    check_probe(probe(path, "probe_" + name), 320, 180, name)

        # RGB testsrc preserves odd dimensions; testsrc2's default YUV420 input
        # would itself round them down before the encoder sees the frame.
        odd = fixtures / "odd-321x181.mov"
        encode(["-f", "lavfi", "-i", "testsrc=size=321x181:rate=25:duration=1"]
               + settings("mov", True) + [odd], "create_odd_exact_dimensions")
        for fmt in formats:
            path = output / ("odd-321x181." + fmt)
            encode(["-i", odd] + settings(fmt, True) + [path], "odd_to_" + fmt)
            check_probe(probe(path, "probe_odd_" + fmt), 321, 181, "odd_" + fmt, audio=False)

        # Explicit 24-bit PCM must be available for high-depth AVI audio.
        pcm24 = fixtures / "audio-24-bit.avi"
        encode(["-i", base, "-c:v", "copy", "-c:a", "pcm_s24le", pcm24], "pcm_24bit_encode")
        info = probe(pcm24, "probe_24bit_audio")
        a = next(s for s in info["streams"] if s["codec_type"] == "audio")
        assert a["codec_name"] == "pcm_s24le" and a.get("bits_per_sample") == 24
        report["checks"].append({"name": "pcm_s24le_encoder", "passed": True, "bits_per_sample": 24})

        # Exercise the exact PQ/BT2020 -> SDR BT709 pipeline used by the app.
        hdr = fixtures / "pq-bt2020-10bit.mov"
        encode(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=1",
                "-c:v", "libx264", "-crf", "16", "-pix_fmt", "yuv420p10le",
                "-x264-params", "colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc", hdr],
               "create_pq_10bit_fixture")
        sdr = output / "pq-to-sdr.mp4"
        chain = ("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,"
                 "tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p")
        encode(["-i", hdr, "-vf", chain, "-c:v", "libx264", "-crf", "16",
                "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", sdr], "pq_to_sdr")
        info = probe(sdr, "probe_pq_to_sdr")
        check_probe(info, 320, 180, "HDR_tonemap_dimensions_fps", audio=False)
        video = info["streams"][0]
        assert video["color_primaries"] == video["color_transfer"] == video["color_space"] == "bt709"
        report["checks"].append({"name": "HDR_tonemap_BT709_tags", "passed": True})
        report["passed"] = True
    except Exception as exc:
        report["passed"] = False
        report["failure"] = sanitize(str(exc))
        raise
    finally:
        dest = ROOT / "reports" / (target + "-codec-qa.json")
        dest.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
        print(str(dest), flush=True)
    print(f"{target}: {len(report['checks'])} checks passed", flush=True)


if __name__ == "__main__":
    main()
