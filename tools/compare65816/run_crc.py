#!/usr/bin/env python3
"""Authenticate CRC artifacts and execute only the focused CRC VM probe."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
from crc import ROOT, digest, verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--debug', action='store_true')
    parser.add_argument('--short', action='store_true')
    parser.add_argument('--allow-external-result-errors', action='store_true',
                        help='retain foreign compiler CRC mismatches; all ABI/access checks still must pass')
    args = parser.parse_args()
    path = args.manifest.resolve()
    manifest = json.loads(path.read_text())
    hashes = verify(manifest)
    hashes[str(path)] = digest(path)
    tag = ('debug' if args.debug else 'release') + ('-short' if args.short else '')
    result = path.parent / (tag + '.json')
    attestation = path.parent / (tag + '-qualification.json')
    result.unlink(missing_ok=True)
    attestation.unlink(missing_ok=True)
    env = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_RELEASE_DEBUG='0',
               A816_CRC_MANIFEST=str(path), A816_CRC_RESULTS=str(result))
    env.pop('A816_CRC_SHORT', None)
    if args.short:
        env['A816_CRC_SHORT'] = '1'
    env.pop('A816_CRC_ALLOW_EXTERNAL_RESULT_ERRORS', None)
    if args.allow_external_result_errors:
        env['A816_CRC_ALLOW_EXTERNAL_RESULT_ERRORS'] = '1'
    command = [sys.executable, '-B', str(ROOT / 'tools/native65816-runtime-tests/qualify.py'),
               *([] if args.debug else ['--release']), '--test', 'crc_bench', '--', '--include-ignored', '--nocapture']
    log = path.parent / (tag + '.log')
    with log.open('w') as stream:
        completed = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT)
    for name, expected in hashes.items():
        assert digest(name) == expected, f'Input changed during execution: {name}'
    completed.check_returncode()
    text = log.read_text()
    qualified = next(line.removeprefix('Qualification manifest: ') for line in text.splitlines() if line.startswith('Qualification manifest: '))
    attestation.write_text(json.dumps(dict(manifest_sha256=digest(path), results_sha256=digest(result), log_sha256=digest(log),
                                         allow_external_result_errors=args.allow_external_result_errors,
                                         vm_qualification=json.loads(Path(qualified).read_text())), indent=2) + '\n')
    print(f'{result}\n{attestation}')


if __name__ == '__main__':
    main()
