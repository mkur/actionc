#!/usr/bin/env python3
"""Authenticate SIEVE artifacts and execute only the focused SIEVE VM probe."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
from sieve import ROOT, digest, verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--debug', action='store_true')
    args = parser.parse_args()
    path = args.manifest.resolve()
    manifest = json.loads(path.read_text())
    hashes = verify(manifest)
    hashes[str(path)] = digest(path)
    tag = ('debug' if args.debug else 'release')
    result = path.parent / (tag + '.json')
    attestation = path.parent / (tag + '-qualification.json')
    result.unlink(missing_ok=True)
    attestation.unlink(missing_ok=True)
    env = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_RELEASE_DEBUG='0',
               A816_SIEVE_MANIFEST=str(path), A816_SIEVE_RESULTS=str(result))
    command = [sys.executable, '-B', str(ROOT / 'tools/native65816-runtime-tests/qualify.py'),
               *([] if args.debug else ['--release']), '--test', 'sieve_bench', '--', '--include-ignored', '--nocapture']
    log = path.parent / (tag + '.log')
    with log.open('w') as stream:
        completed = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT)
    for name, expected in hashes.items():
        assert digest(name) == expected, f'Input changed during execution: {name}'
    completed.check_returncode()
    text = log.read_text()
    qualified = next(line.removeprefix('Qualification manifest: ') for line in text.splitlines() if line.startswith('Qualification manifest: '))
    attestation.write_text(json.dumps(dict(manifest_sha256=digest(path), results_sha256=digest(result), log_sha256=digest(log),
                                         vm_qualification=json.loads(Path(qualified).read_text())), indent=2) + '\n')
    print(f'{result}\n{attestation}')


if __name__ == '__main__':
    main()
