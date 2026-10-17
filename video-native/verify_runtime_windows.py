#!/usr/bin/env python3
"""Observe actual embedded Windows worker files and cleanup during HLG conversion."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess
import time

ROOT = Path(__file__).resolve().parent
WORKSPACE = ROOT.parent.parent
QA = ROOT / "qa/app-windows"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--exe", type=Path, default=ROOT.parent / "release/Roshan-HEIC.exe")
parser.add_argument("--wine-env", type=Path, default=WORKSPACE / "native/qa-windows/wine11-env.json")
parser.add_argument("--report", type=Path, default=ROOT.parent / "verification/videos-v2-windows.json")
parser.add_argument("--final-smoke", action="store_true", help="Append a final-binary mixed image/video smoke without repeating the matrix")
args = parser.parse_args()
ENV_FILE, EXE, REPORT = args.wine_env, args.exe.resolve(), args.report


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def win(path):
    return "Z:" + str(path.resolve()).replace("/", "\\")


env = os.environ.copy()
env.update(json.loads(ENV_FILE.read_text()))
env.pop("DISPLAY", None)
wine_profile = Path(env["WINEPREFIX"]) / "drive_c"
temp_roots = [wine_profile / "windows/temp"] + list((wine_profile / "users").glob("*/AppData/Local/Temp"))


def directories():
    return {p for root in temp_roots if root.exists() for p in root.glob("RoshanConverter-*") if p.is_dir()}


fixture = QA / "fixtures/hlg-bt2020-10bit.mov"
subprocess.run([str(ROOT / "linux/bin/ffmpeg"), "-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i",
                "testsrc2=size=640x360:rate=25:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p10le",
                "-x264-params", "colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc", str(fixture)], check=True)
expected = json.loads((ROOT / "reports/windows-build.json").read_text())["outputs"]
before = directories()
# Each run gets a fresh path: the product deliberately refuses to overwrite a
# report, and reusing a pathname can expose stale handles in Wine/filesystems.
run_id = f"{sha(EXE)[:12]}-{time.time_ns():x}"
cli_report = QA / f"cli-reports/13-hlg-runtime-{run_id}.json"
cli_report.parent.mkdir(parents=True, exist_ok=True)
dest = QA / "hlg-runtime-output" / run_id
dest.mkdir(parents=True, exist_ok=False)
observed = {}
command = [env["WINELOADER"], str(EXE), "--convert", "--to", "webm", "--output", win(dest),
           "--report", win(cli_report), win(fixture)]
if args.final_smoke:
    # A small, precisely defined PNG requires no downloaded fixture or image
    # package: PNG chunks and DEFLATE are part of Python's standard library.
    import struct
    import zlib
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    png = QA / "fixtures/mixed-smoke-32x24.png"
    pixels = (b"\x00" + bytes((12, 110, 180)) * 32) * 24
    png.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 32, 24, 8, 2, 0, 0, 0))
                    + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))
    command += [win(png)]
start = time.monotonic()
process = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
while True:
    for directory in directories() - before:
        for name, value in expected.items():
            path = directory / name
            key = directory.name + "/" + name
            if key not in observed:
                try:
                    if path.stat().st_size == value["bytes"]:
                        observed[key] = sha(path)
                        assert observed[key] == value["sha256"]
                except FileNotFoundError:
                    pass
    try:
        stdout, stderr = process.communicate(timeout=0.015)
        break
    except subprocess.TimeoutExpired:
        assert time.monotonic() - start < 120, "Runtime verification timed out"
assert process.returncode == 0, stderr.decode(errors="replace")
assert len(observed) == 2, observed
remaining = directories() - before
if remaining:
    diagnostic = {"app_sha256": sha(EXE), "exit_code": process.returncode,
        "observed_extracted_hashes": observed, "remaining_runtime_directories": sorted(p.name for p in remaining),
        "cli_report": cli_report.name, "stdout": stdout.decode(errors="replace"), "stderr": stderr.decode(errors="replace")}
    path = QA / f"runtime-observation-{run_id}.json"
    path.write_text(json.dumps(diagnostic, ensure_ascii=False, indent=2) + "\n")
    print("Runtime observation retained:", path, flush=True)
assert not remaining, "A newly extracted worker directory remained"
data = json.loads(cli_report.read_text())
row = data["files"][0]["outputs"][0]
path = Path(row["path"][2:].replace("\\", "/"))
probe = subprocess.run([str(ROOT / "linux/bin/ffprobe"), "-v", "error", "-show_streams", "-of", "json", str(path)], stdout=subprocess.PIPE, text=True, check=True)
video = json.loads(probe.stdout)["streams"][0]
assert (video["width"], video["height"]) == (640, 360)
assert video["color_primaries"] == video["color_transfer"] == video["color_space"] == "bt709"
assert any("tone-mapped" in w for w in row["warnings"])
report = json.loads(REPORT.read_text())
entry = {"name": "hlg_tonemap_and_actual_runtime", "exit_code": process.returncode,
    "expected_exit_code": 0, "elapsed_seconds": round(time.monotonic() - start, 3),
    "cli_report": cli_report.name,
    "runtime_cleaned": True, "observed_extracted_hashes": observed,
    "runtime_cleanup_observation_scope": "Observed immediately after application exit, before the enclosing execution command returned; later filesystem synchronization is outside this assertion.",
    "report": {"input": fixture.name, "output": path.name, "warnings": row["warnings"]}}
if args.final_smoke:
    assert [r["status"] for r in data["files"]] == ["ok", "ok"]
    image = data["files"][1]["outputs"][0]
    assert (image["width"], image["height"]) == (32, 24)
    assert image["path"].lower().endswith(".jpg")
    entry.update(name="final_binary_mixed_image_video_smoke", app=EXE.name, sha256=sha(EXE), bytes=EXE.stat().st_size,
                 image_output={"width": 32, "height": 24, "format": "jpg"}, passed=True)
    report["final_release_smoke"] = entry
else:
    assert report["sha256"] == sha(EXE)
    for batch in report["cli_batches"]:
        if not batch["observed_extracted_hashes"]:
            batch["runtime_cleaned"] = None
            batch["runtime_observation_note"] = "Initial custom-TEMP observation was superseded by the dedicated actual-Wine-temp check."
    report["cli_batches"].append(entry)
    report["checks"].extend([
        {"name": "HLG_10bit_to_BT709_SDR", "passed": True, "width": 640, "height": 360},
        {"name": "actual_embedded_worker_SHA256_and_normal_exit_cleanup", "passed": True, "workers_observed": 2},
    ])
report["test_scripts"] = [{"path": "video-native/" + name, "sha256": sha(ROOT / name)}
                           for name in ("verify_app_windows.py", "verify_runtime_windows.py")]
report["passed"] = True
report.pop("failure", None)
REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
print("HLG and observed worker extraction/cleanup passed:", observed, flush=True)
