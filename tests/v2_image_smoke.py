#!/usr/bin/env python3
"""Four conversions against an exact executable snapshot after a GUI-only rebuild.

This supplements the complete v2_image_matrix.py evidence. It requires Pillow
and NumPy, creates its own synthetic RGBA image, and uses no external files.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
import numpy as np
from PIL import Image, ImageCms
from v2_image_matrix import save_png, metadata, linear_flatten, require, sha


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--cli', type=Path, required=True)
    p.add_argument('--expected-sha256', required=True)
    p.add_argument('--work-dir', type=Path, required=True)
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--wine', type=Path)
    p.add_argument('--wine-env', type=Path)
    args = p.parse_args()
    args.cli = args.cli.resolve()
    data = args.cli.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    require(digest == args.expected_sha256, 'The requested application SHA-256 does not match')
    args.work_dir.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='final-image-smoke-', dir=args.work_dir.resolve()))
    executable = work / ('application.exe' if args.wine else 'application')
    executable.write_bytes(data)
    executable.chmod(0o755)
    env = os.environ.copy()
    if args.wine_env:
        env.update(json.loads(args.wine_env.read_text()))
    if args.wine:
        env.pop('DISPLAY', None)

    def cli_path(path):
        value = str(path.resolve())
        return 'Z:' + value.replace('/', '\\') if args.wine else value

    records = []

    def convert(source, target):
        directory = work / ('output-' + str(len(records) + 1) + '-' + target)
        report = work / ('command-' + str(len(records) + 1) + '.json')
        command = ([str(args.wine)] if args.wine else []) + [str(executable), '--convert', '--to', target,
                   '--output', cli_path(directory), '--report', cli_path(report), cli_path(source)]
        result = subprocess.run(command, env=env, text=True, encoding='utf-8', errors='replace',
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        require(result.returncode == 0 and report.is_file(), 'Conversion failed: ' + result.stderr)
        record = json.loads(report.read_text())['files'][0]
        require(record['status'] == 'ok' and len(record['outputs']) == 1, 'Unexpected CLI report')
        name = record['outputs'][0]['path']
        if args.wine:
            if name.startswith('\\\\?\\'):
                name = name[4:]
            require(name[:2].lower() == 'z:', 'Unexpected Wine output drive')
            name = name[2:].replace('\\', '/')
        output = Path(name)
        records.append({'target': target, 'exit': result.returncode, 'record': record,
                        'output_sha256': sha(output)})
        return output

    w, h = 73, 67
    y, x = np.mgrid[:h, :w]
    rgba = np.empty((h, w, 4), np.uint8)
    rgba[..., 0] = (x * 37 + y * 11) & 255
    rgba[..., 1] = (x * 3 + y * 71 + 13) & 255
    rgba[..., 2] = (x * 139 + y * 7 + 191) & 255
    rgba[..., 3] = np.array([0, 64, 128, 192, 255], np.uint8)[(x // 9 + y // 11) % 5]
    icc = ImageCms.ImageCmsProfile(ImageCms.createProfile('sRGB')).tobytes()
    source = work / 'تصویر شفاف ۱۲.png'
    save_png(source, rgba, icc, metadata(w, h))
    heic = convert(source, 'heic')
    png = convert(heic, 'png')
    webp = convert(source, 'webp')
    jpg = convert(source, 'jpg')
    for output in [png, webp]:
        with Image.open(output) as image:
            require(np.array_equal(np.array(image.convert('RGBA')), rgba), 'Lossless RGBA samples changed')
            require(image.info.get('icc_profile') == icc, 'ICC bytes changed')
            exif = image.getexif()
            require(exif.get(274) == 1 and exif.get(256) == w and exif.get(257) == h,
                    'EXIF orientation/dimensions changed')
    with Image.open(jpg) as image:
        require(image.size == (w, h) and image.info.get('icc_profile') == icc, 'JPEG dimensions/ICC changed')
        require(all(layer[1:3] == (1, 1) for layer in image.layer), 'JPEG subsampling is not 4:4:4')
        require(all(v == 1 for table in image.quantization.values() for v in table), 'JPEG quality is not 100')
        delta = np.abs(np.array(image.convert('RGB')).astype(int) - linear_flatten(rgba).astype(int))
        require(int(delta.max()) <= 7 and float(delta.mean()) < 1.3, 'JPEG linear-white error exceeds tolerance')
    require(sha(executable) == digest, 'Executable snapshot changed during verification')
    root = Path(__file__).resolve().parent.parent
    result = {'result': 'passed', 'application': str(args.cli), 'application_sha256': digest,
              'executed_snapshot': str(executable), 'application_bytes': len(data),
              'target': 'windows-under-wine' if args.wine else 'linux-native',
              'script_sha256': sha(Path(__file__)), 'matrix_helper_sha256': sha(root / 'tests/v2_image_matrix.py'),
              'native_bridge_sha256': sha(root / 'native/bridge.cpp'),
              'source_sha256': {name: sha(root / 'src' / name) for name in
                                ['image_engine.rs', 'color.rs', 'metadata.rs', 'safe_output.rs', 'engine.rs']},
              'checks': ['Persian Unicode input/output paths', 'PNG -> HEIC -> PNG exact RGBA including hidden RGB',
                         'PNG -> WebP exact RGBA', 'ICC byte preservation', 'normalized EXIF',
                         'JPEG quality100, 4:4:4, linear-light white alpha'],
              'jpeg_max_sample_difference': int(delta.max()), 'jpeg_mean_sample_difference': float(delta.mean()),
              'cli_runs': records, 'fixture_policy': 'Synthetic mathematical RGBA pattern generated by this script.',
              'limits': 'Four-command smoke after a GUI-only rebuild; the complete 68-command matrix is recorded separately.'}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')
    print('PASS: four final-EXE image conversions, exact lossless alpha/ICC/EXIF and JPEG100/444/linear-white')
    print(digest, args.report)


if __name__ == '__main__':
    main()
