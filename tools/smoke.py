#!/usr/bin/env python3
"""Run the real binary from a detached portable directory; stdlib only."""
import argparse
import json
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='drosophila-smoke-') as tmp:
        root = Path(tmp)
        package = root / 'portable folder with spaces'
        package.mkdir()
        binary = package / args.binary.name
        shutil.copy2(args.binary.resolve(), binary)
        shutil.copytree(ROOT / 'data', package / 'data', ignore=shutil.ignore_patterns('*.gz', '*.obj', 'neuropils'))
        cwd = root / 'unrelated'
        cwd.mkdir()
        subprocess.run([str(binary), 'stats'], cwd=cwd, check=True, capture_output=True)
        subprocess.run([str(binary), '--help'], cwd=cwd, check=True, capture_output=True)
        for options in [['run', '--seconds', 'NaN'], ['run', '--brain', 'inf'], ['run', '--bogus'], ['run', '--open'], ['run', '--serve', '0.0.0.0:8770']]:
            result = subprocess.run([str(binary), *options], cwd=cwd, capture_output=True, timeout=10)
            assert result.returncode != 0, f'bad arguments accepted: {options}'

        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        with (root / 'server.log').open('w+') as log:
            process = subprocess.Popen([str(binary), 'run', '--seconds', '0', '--serve', str(port)], cwd=cwd, stdout=log, stderr=log)
            try:
                base = f'http://127.0.0.1:{port}'
                def get(path):
                    with urllib.request.urlopen(base + path, timeout=3) as response:
                        return response.read()
                for _ in range(100):
                    if process.poll() is not None:
                        log.seek(0)
                        raise AssertionError(log.read())
                    try:
                        page = get('/')
                        break
                    except (OSError, urllib.error.URLError):
                        time.sleep(0.1)
                else:
                    raise AssertionError('server never became ready')
                assert b'brain-toggle' in page
                for asset in ['app.js', 'model.js', 'world_view.js', 'protocol.js', 'brain_view.js', 'vendor/three.module.js', 'vendor/three.core.js']:
                    assert len(get('/' + asset)) > 100, asset
                world = json.loads(get('/world.json'))
                assert len(world['pillars']) == 3
                assert len(world['pebbles']) == 12
                first = get('/state')
                time.sleep(0.15)
                second = get('/state')
                assert struct.unpack_from('<I', first)[0] == 0x41545344
                assert struct.unpack_from('<I', second, 4)[0] > struct.unpack_from('<I', first, 4)[0]
                for path in ['/vendor/../../Cargo.toml', '/Cargo.toml']:
                    try:
                        get(path)
                    except urllib.error.HTTPError as error:
                        assert error.code == 404
                    else:
                        raise AssertionError(f'private file exposed: {path}')
                occupied = subprocess.run([str(binary), 'run', '--serve', str(port), '--seconds', '0.1'], cwd=cwd, capture_output=True, timeout=10)
                assert occupied.returncode != 0, 'port conflict did not fail'
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
    print('PASS: portable data, embedded assets, CLI validation, live state, collision layout, port conflicts')


if __name__ == '__main__':
    main()
