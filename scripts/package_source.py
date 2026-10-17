#!/usr/bin/env python3
"""Package the complete Roshan Converter source and the pinned codec sources.

Run from any directory with Python 3.12+. No build caches, public photographs,
toolchains, generated fixture media or previous-version reports are included.
"""
from pathlib import Path
import argparse
import hashlib
import json
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_files():
    selected = set()

    def file(relative):
        path = ROOT / relative
        if not path.is_file():
            raise FileNotFoundError(path)
        selected.add(path)

    def tree(relative, predicate=lambda p: True):
        for path in (ROOT / relative).rglob('*'):
            if path.is_file() and '__pycache__' not in path.parts and predicate(path):
                selected.add(path)

    for relative in [
        'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'build.rs',
        'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'README-EN.txt', 'README-FA.txt',
        'BUILD.txt', '.cargo/config.toml',
        'native-build/dependencies.json', 'native-build/build_native.py',
        'native-build/README.txt', 'native-build/native_smoke.c',
        'video-native/dependencies.json', 'video-native/build_video.py',
        'video-native/toolchain.json',
        'video-native/verify_video.py', 'video-native/verify_app_windows.py',
        'video-native/verify_runtime_windows.py', 'video-native/README.md',
        'video-native/THIRD_PARTY_NOTICES.txt',
        'video-native/tools/ubuntu-packages.json',
        'video-native/windows/bin/ffmpeg.exe',
        'video-native/windows/bin/ffprobe.exe', 'release/windows-imports.json',
        'verification/README.txt',
    ]:
        file(relative)
    tree('src', lambda p: p.suffix == '.rs')
    tree('native', lambda p: p.suffix in {'.cpp', '.h'})
    tree('assets')
    tree('scripts', lambda p: p.suffix == '.py')
    tree('vendor')
    for relative in ['native-build/archives', 'native-build/licenses',
                     'native-build/reports', 'video-native/archives',
                     'video-native/licenses', 'video-native/reports']:
        tree(relative)
    tree('tests', lambda p: p.suffix in {'.py', '.cpp', '.c', '.h', '.json', '.txt', '.md'})
    tree('verification', lambda p: '-v2' in p.name and 'predecessor' not in p.name and
         p.suffix in {'.json', '.txt', '.png'})
    return sorted(selected, key=lambda p: p.relative_to(ROOT).as_posix())


def validate_dependencies():
    archives = []
    for prefix in ['native-build', 'video-native']:
        for dep in json.loads((ROOT / prefix / 'dependencies.json').read_text()):
            path = ROOT / prefix / 'archives' / dep['filename']
            actual = digest(path)
            if actual != dep['sha256']:
                raise ValueError(f'Archive checksum mismatch: {path}')
            if path.stat().st_size != dep['bytes']:
                raise ValueError(f'Archive size mismatch: {path}')
            archives.append(path.relative_to(ROOT).as_posix())
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    vendored = set()
    for path in (ROOT / 'vendor').glob('*/Cargo.toml'):
        package = tomllib.loads(path.read_text())['package']
        vendored.add((package['name'], package['version']))
    needed = {(p['name'], p['version']) for p in lock['package']
              if p.get('source', '').startswith('registry+')}
    missing = needed - vendored
    if missing:
        raise ValueError(f'Cargo sources missing from vendor: {sorted(missing)}')
    return {'pinned_archives_verified': len(archives),
            'locked_registry_crates_present': len(needed)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    checks = validate_dependencies()
    files = source_files()
    manifest = ROOT / 'SOURCE_MANIFEST.sha256'
    manifest.write_text(''.join(f'{digest(p)}  {p.relative_to(ROOT).as_posix()}\n'
                                for p in files), encoding='utf-8')
    files.append(manifest)
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED,
                         compresslevel=9, strict_timestamps=False) as archive:
        for path in sorted(files):
            archive.write(path, 'Roshan-Converter-Source/' + path.relative_to(ROOT).as_posix())
    with zipfile.ZipFile(output) as archive:
        bad = archive.testzip()
        if bad:
            raise ValueError(f'ZIP CRC failure: {bad}')
    print(json.dumps({'path': str(output), 'bytes': output.stat().st_size,
                      'sha256': digest(output), 'files': len(files),
                      'zip_crc': 'passed', **checks}, indent=2))


if __name__ == '__main__':
    main()
