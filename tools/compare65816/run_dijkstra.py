#!/usr/bin/env python3
"""Authenticate external Dijkstra artifacts, run the qualified VM, retain provenance."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest


def verify(manifest):
    hashes = dict(manifest['inputs']) | manifest['generated']
    hashes.update({v['path']: v['sha256'] for v in manifest['tools'].values()})
    for artifact in manifest['artifacts']:
        hashes.update(artifact['hashes'])
    for path, expected in hashes.items():
        if digest(path) != expected:
            raise ValueError(f'Changed comparison input: {path}')
    return hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--debug', action='store_true')
    parser.add_argument('--filter', default='')
    parser.add_argument('--crlf-vectors', action='store_true')
    args = parser.parse_args()
    path = args.manifest.resolve()
    manifest = json.loads(path.read_text())
    hashes = verify(manifest)
    profile = 'debug' if args.debug else 'release'
    tag = profile+('-crlf' if args.crlf_vectors else '')+('-'+args.filter if args.filter else '')
    result = path.parent/(tag+'-results.json')
    attestation = path.parent/(tag+'-qualification.json')
    attestation.unlink(missing_ok=True)
    result.unlink(missing_ok=True)
    manifest_hash = digest(path)
    if args.crlf_vectors:
        for key in ('graphs', 'vectors'):
            copy = path.parent/('crlf-'+key+'.txt')
            copy.write_bytes(Path(manifest[key]).read_text().replace('\r\n','\n').replace('\n','\r\n').encode())
            manifest[key] = str(copy)
            hashes[str(copy)] = digest(copy)
        path = path.parent/('crlf-manifest.json')
        path.write_text(json.dumps(manifest,indent=2)+'\n')
    hashes[str(path)] = digest(path)
    env = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0',
               A816_DIJKSTRA_MANIFEST=str(path), A816_DIJKSTRA_RESULTS=str(result), A816_DIJKSTRA_FILTER=args.filter)
    command = [sys.executable,'-B',str(ROOT/'tools/native65816-runtime-tests/qualify.py'),
               *([] if args.debug else ['--release']), '--test','dijkstra','--','--include-ignored','--nocapture']
    log = path.parent/(tag+'.log')
    completed = subprocess.run(command, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    log.write_text(completed.stdout)
    print(completed.stdout, end='')
    for name, expected in hashes.items():
        if digest(name)!=expected:
            raise ValueError(f'Input changed during execution: {name}')
    completed.check_returncode()
    qualification = next(line.removeprefix('Qualification manifest: ') for line in completed.stdout.splitlines() if line.startswith('Qualification manifest: '))
    payload = dict(manifest_sha256=manifest_hash, execution_manifest_sha256=digest(path),
                   results_sha256=digest(result), profile=profile, filter=args.filter,
                   crlf_vectors=args.crlf_vectors, log_sha256=digest(log),
                   vm_qualification=json.loads(Path(qualification).read_text()))
    attestation.write_text(json.dumps(payload,indent=2)+'\n')
    print(attestation)


if __name__ == '__main__':
    main()
