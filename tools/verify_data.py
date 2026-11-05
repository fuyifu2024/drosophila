#!/usr/bin/env python3
"""Verify the distributed data bytes, not their scientific validity."""
import hashlib
from pathlib import Path

root = Path(__file__).resolve().parents[1] / 'data'
for line in (root / 'SHA256SUMS').read_text().splitlines():
    digest, name = line.split(None, 1)
    actual = hashlib.sha256((root / name.strip()).read_bytes()).hexdigest()
    if actual != digest:
        raise SystemExit(f'Checksum mismatch: {name}')
print('PASS: bundled data checksums')
