#!/usr/bin/env python3
"""Build a portable, offline archive using only the Python standard library."""
import argparse
import hashlib
from pathlib import Path
import shutil
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--name', required=True)
    args = parser.parse_args()
    if Path(args.name).name != args.name or args.name in ('.', '..'):
        parser.error('--name must be one directory name')
    bundle = ROOT / 'dist' / args.name
    bundle.mkdir(parents=True, exist_ok=False)
    shutil.copy2(args.binary, bundle / args.binary.name)
    for name in ['README.md', 'LICENSE', 'CREDITS.md', 'THIRD_PARTY_NOTICES.md', 'start.sh', 'start.command']:
        shutil.copy2(ROOT / name, bundle / name)
    shutil.copytree(ROOT / 'licenses', bundle / 'licenses')
    (bundle / 'data').mkdir()
    for name in ['brain.bin', 'points.bin', 'labels.json', 'README.md', 'SHA256SUMS']:
        shutil.copy2(ROOT / 'data' / name, bundle / 'data' / name)
    artifacts = ROOT / 'artifacts'
    artifacts.mkdir(exist_ok=True)
    if args.binary.suffix == '.exe':
        output = artifacts / (args.name + '.zip')
        with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(bundle.rglob('*')):
                if path.is_file():
                    archive.write(path, path.relative_to(bundle.parent))
    else:
        output = artifacts / (args.name + '.tar.gz')
        with tarfile.open(output, 'w:gz') as archive:
            archive.add(bundle, arcname=args.name)
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_name(output.name + '.sha256').write_text(f'{digest}  {output.name}\n')
    print(output)


if __name__ == '__main__':
    main()
