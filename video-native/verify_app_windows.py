#!/usr/bin/env python3
"""Verify the integrated Rust Windows executable using its embedded workers."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parent
WORKSPACE = ROOT.parent.parent


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def win(path: Path) -> str:
    return "Z:" + str(path.resolve()).replace("/", "\\")


def local(path: str) -> Path:
    if path[:2].lower() != "z:":
        raise ValueError("Expected an isolated workspace file")
    return Path(path[2:].replace("\\", "/"))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--wine-env", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--resume", action="store_true", help="Continue after the completed matrix and odd-size checks")
    args = parser.parse_args()
    exe = args.exe.resolve()
    qa = ROOT / "qa/app-windows"
    fixtures = qa / "fixtures"
    reports = qa / "cli-reports"
    runtime = qa / "private-runtime"
    for p in (fixtures, reports, runtime):
        p.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(json.loads(args.wine_env.read_text()))
    env.pop("DISPLAY", None)
    env["TEMP"] = env["TMP"] = win(runtime)
    # Wine can replace TEMP/TMP with its registry profile value on launch.
    # Observe both the requested path and the actual isolated profile folders.
    profile = Path(env["WINEPREFIX"]) / "drive_c"
    runtime_roots = [runtime, profile / "windows/temp"] + list((profile / "users").glob("*/AppData/Local/Temp"))

    def runtime_directories() -> set[Path]:
        return {p for root in runtime_roots if root.exists() for p in root.glob("RoshanConverter-*") if p.is_dir()}
    native_ffmpeg = ROOT / "linux/bin/ffmpeg"
    native_ffprobe = ROOT / "linux/bin/ffprobe"
    payloads = json.loads((ROOT / "reports/windows-build.json").read_text())["outputs"]
    report: dict = {"app": exe.name, "version": "2.0.0", "bytes": exe.stat().st_size,
                   "sha256": sha(exe), "platform": "Windows x64 under Wine 11",
                   "embedded_payloads_expected": payloads, "checks": [], "cli_batches": []}
    counter = 0
    if args.resume:
        prior = json.loads(args.report.read_text())
        assert prior["sha256"] == report["sha256"], "Resume requires the identical app executable"
        report = prior
        report.pop("failure", None)
        counter = len(report["cli_batches"])

    def clean(value):
        if isinstance(value, str):
            return value.replace(win(WORKSPACE), "<workspace>").replace(str(WORKSPACE), "<workspace>")
        if isinstance(value, list):
            return [clean(x) for x in value]
        if isinstance(value, dict):
            return {k: clean(v) for k, v in value.items()}
        return value

    def native(arguments: list[str | Path], probe: bool = False, binary: bool = False):
        tool = native_ffprobe if probe else native_ffmpeg
        cmd = [str(tool)] + [str(x) for x in arguments]
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
        if p.returncode:
            raise RuntimeError(p.stderr.decode(errors="replace"))
        return p.stdout if binary else p.stdout.decode(errors="replace")

    def encode(arguments: list[str | Path]):
        native(["-hide_banner", "-v", "error", "-nostdin", "-y"] + arguments)

    def probe(path: Path) -> dict:
        return json.loads(native(["-v", "error", "-show_streams", "-show_format", "-of", "json", path], True))

    def description(path: Path) -> dict:
        p = probe(path)
        video = next(s for s in p["streams"] if s["codec_type"] == "video")
        audio = [s for s in p["streams"] if s["codec_type"] == "audio"]
        rotation = next((s["rotation"] for s in video.get("side_data_list", []) if "rotation" in s), 0)
        width, height = video["width"], video["height"]
        if int(rotation) % 180 == 90:
            width, height = height, width
        return {"width": width, "height": height, "coded_width": video["width"], "coded_height": video["height"],
                "rotation": rotation, "fps": video["avg_frame_rate"], "duration": float(p["format"]["duration"]),
                "video_codec": video["codec_name"], "pix_fmt": video["pix_fmt"],
                "color_transfer": video.get("color_transfer"), "color_primaries": video.get("color_primaries"),
                "color_space": video.get("color_space"),
                "audio": [{k: s.get(k) for k in ("codec_name", "channels", "sample_rate", "bits_per_sample", "bits_per_raw_sample")} for s in audio]}

    def packet_hashes(path: Path) -> list[str]:
        p = json.loads(native(["-v", "error", "-select_streams", "v:0", "-show_packets", "-show_data_hash", "sha256", "-of", "json", path], True))
        return [v["data_hash"] for v in p["packets"]]

    def cli(name: str, target: str, inputs: list[Path], extra: list[str] | None = None,
            expected_exit: int = 0, output: Path | None = None) -> dict:
        nonlocal counter
        counter += 1
        output = output or qa / "خروجی آزمون" / name
        output.mkdir(parents=True, exist_ok=True)
        json_path = reports / (f"{counter:02d}-" + name + ".json")
        json_path.unlink(missing_ok=True)
        before_runtime = runtime_directories()
        command = [env["WINELOADER"], str(exe), "--convert", "--to", target,
                   "--output", win(output), "--report", win(json_path)] + (extra or []) + [win(p) for p in inputs]
        started = time.monotonic()
        proc = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        observed: dict = {}
        while True:
            for directory in runtime_directories() - before_runtime:
                for filename, expected in payloads.items():
                    path = directory / filename
                    key = directory.name + "/" + filename
                    if key not in observed and path.exists() and path.stat().st_size == expected["bytes"]:
                        try:
                            observed[key] = sha(path)
                            assert observed[key] == expected["sha256"], "Extracted payload hash differs"
                        except FileNotFoundError:
                            pass
            try:
                stdout, stderr = proc.communicate(timeout=0.03)
                break
            except subprocess.TimeoutExpired:
                if time.monotonic() - started > 180:
                    proc.kill()
                    proc.communicate()
                    raise RuntimeError(name + " timed out")
        elapsed = time.monotonic() - started
        leftovers = list(runtime_directories() - before_runtime)
        assert not leftovers, "Private extracted workers were not cleaned up"
        data = json.loads(json_path.read_text()) if json_path.exists() else {}
        entry = {"name": name, "exit_code": proc.returncode, "expected_exit_code": expected_exit,
                 "elapsed_seconds": round(elapsed, 3), "runtime_cleaned": not leftovers,
                 "observed_extracted_hashes": observed, "report": clean(data),
                 "stderr": clean(stderr.decode(errors="replace")[-3000:])}
        report["cli_batches"].append(entry)
        assert proc.returncode == expected_exit, f"{name}: {entry['stderr']}"
        assert data, name + " did not write a report"
        if expected_exit == 0:
            assert all(f["status"] == "ok" for f in data["files"]), name
        print(name, "exit", proc.returncode, "seconds", round(elapsed, 2), flush=True)
        return data

    def compare(name: str, source: Path, output: Path, audio_count: int | None = None):
        src, dst = description(source), description(output)
        assert (src["width"], src["height"]) == (dst["width"], dst["height"]), name + " dimensions"
        assert len(dst["audio"]) == (len(src["audio"]) if audio_count is None else audio_count), name + " audio count"
        assert abs(src["duration"] - dst["duration"]) < 0.1, name + " duration"
        assert src["fps"] == dst["fps"], name + " frame rate"
        for a, b in zip(src["audio"], dst["audio"]):
            assert a["channels"] == b["channels"], name + " channel count"
            assert a["sample_rate"] == b["sample_rate"], name + " sample rate"
        report["checks"].append({"name": name, "passed": True, "source": source.name, "output": output.name,
                                 "before": src, "after": dst})
        return dst

    try:
        base = fixtures / "دو صدا MP4.mp4"
        sources = {fmt: fixtures / ("دو صدا " + fmt.upper() + "." + fmt) for fmt in ("mp4", "mov", "avi", "webm")}
        if not args.resume:
            base = fixtures / "دو صدا MP4.mp4"
            quick = ROOT / "qa/windows/fixtures/نمونه حرکت.mp4"
            encode(["-i", quick, "-f", "lavfi", "-i", "sine=frequency=660:sample_rate=48000:duration=1",
                    "-map", "0:v:0", "-map", "0:a:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac", "-ac", "2",
                    "-b:a", "256k", "-metadata:s:a:0", "title=First 1000 Hz", "-metadata:s:a:1", "title=Second 660 Hz", "-shortest", base])
            sources = {"mp4": base}
            for fmt, options in (("mov", ["-c", "copy"]),
                                 ("avi", ["-c:v", "mpeg4", "-q:v", "2", "-c:a", "pcm_s16le"]),
                                 ("webm", ["-c:v", "libvpx-vp9", "-crf", "20", "-b:v", "0", "-cpu-used", "2", "-c:a", "libopus", "-b:a", "192k"])):
                path = fixtures / ("دو صدا " + fmt.upper() + "." + fmt)
                encode(["-i", base, "-map", "0"] + options + [path])
                sources[fmt] = path
            input_hashes = {str(path): sha(path) for path in sources.values()}
            for target in sources:
                inputs = [p for fmt, p in sources.items() if fmt != target]
                batch = cli("matrix_to_" + target, target, inputs)
                for row in batch["files"]:
                    source, output = local(row["input"]), local(row["outputs"][0]["path"])
                    name = source.suffix[1:] + "_to_" + target
                    compare(name, source, output, 2)
                    if source.suffix in (".mp4", ".mov") and target in ("mp4", "mov"):
                        assert packet_hashes(source) == packet_hashes(output), "Compatible video was recompressed"
                        report["checks"].append({"name": name + "_compressed_video_packets_identical", "passed": True})

            odd = ROOT / "qa/windows/fixtures/odd-321x181.mov"
            for target in ("mp4", "avi", "webm"):
                batch = cli("odd_to_" + target, target, [odd])
                compare("odd_to_" + target, odd, local(batch["files"][0]["outputs"][0]["path"]))

        input_hashes = {str(path): sha(path) for path in sources.values()}

        rotated = fixtures / "rotate90.mp4"
        encode(["-display_rotation:v:0", "90", "-i", base, "-map", "0", "-c", "copy", rotated])
        assert description(rotated)["rotation"] in (90, -270)
        for target in ("mov", "avi"):
            batch = cli("rotation_to_" + target, target, [rotated])
            result = local(batch["files"][0]["outputs"][0]["path"])
            compare("rotation_to_" + target, rotated, result)
            if target == "mov":
                assert packet_hashes(rotated) == packet_hashes(result)
            else:
                import numpy as np
                frames = [native(["-v", "error", "-i", path, "-frames:v", "1", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"], binary=True) for path in (rotated, result)]
                a, b = (np.frombuffer(x, dtype=np.uint8).astype(float) for x in frames)
                mse = float(np.mean((a-b)**2))
                psnr = 99.0 if mse == 0 else 10*math.log10(255**2/mse)
                assert psnr > 25, "Rotation changed the displayed picture"
                report["checks"].append({"name": "rotated_decoded_frame_matches", "passed": True, "psnr_db": round(psnr, 3)})

        hdr = ROOT / "qa/windows/fixtures/pq-bt2020-10bit.mov"
        batch = cli("hdr_pq_to_webm", "webm", [hdr])
        row = batch["files"][0]["outputs"][0]
        dst = compare("hdr_pq_to_webm", hdr, local(row["path"]))
        assert dst["color_transfer"] == dst["color_primaries"] == dst["color_space"] == "bt709"
        assert any("tone-mapped" in w for w in row["warnings"])
        report["checks"].append({"name": "HDR_explicit_BT709_tone_map_warning", "passed": True})

        pcm = fixtures / "audio-24-bit.mov"
        encode(["-i", base, "-map", "0", "-c:v", "copy", "-c:a", "pcm_s24le", pcm])
        batch = cli("pcm24_to_avi", "avi", [pcm])
        dst = compare("pcm24_to_avi", pcm, local(batch["files"][0]["outputs"][0]["path"]))
        assert all(a["bits_per_sample"] == 24 for a in dst["audio"])

        corrupt = fixtures / "corrupt-first.webm"
        corrupt.write_bytes(b"This is deliberately not a video.\x00")
        dest = qa / "no-overwrite"
        dest.mkdir(exist_ok=True)
        sentinel = dest / base.name
        sentinel.write_bytes(b"Existing file must remain unchanged.")
        sentinel_hash = sha(sentinel)
        batch = cli("corrupt_and_duplicate", "mp4", [corrupt, base, base], expected_exit=1, output=dest)
        assert [r["status"] for r in batch["files"]] == ["error", "ok", "ok"]
        outputs = [local(r["outputs"][0]["path"]) for r in batch["files"][1:]]
        assert outputs[0] != outputs[1] and all(p != sentinel for p in outputs)
        assert sha(sentinel) == sentinel_hash and all(sha(p) == sha(base) for p in outputs)
        assert all(sha(Path(path)) == value for path, value in input_hashes.items())
        report["checks"].append({"name": "corrupt_input_isolated_duplicates_originals_preserved", "passed": True})
        report["checks"].append({"name": "same_container_is_byte_identical", "passed": True})
        assert any(b["observed_extracted_hashes"] for b in report["cli_batches"]), "No embedded payload extraction observed"
        report["passed"] = True
    except Exception as exc:
        report["passed"] = False
        report["failure"] = clean(str(exc))
        raise
    finally:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(clean(report), ensure_ascii=False, indent=2) + "\n")
        print(str(args.report), flush=True)
    print(f"Integrated Windows app: {len(report['checks'])} checks passed", flush=True)


if __name__ == "__main__":
    main()
