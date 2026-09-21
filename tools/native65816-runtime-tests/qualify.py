#!/usr/bin/env python3
"""Run against the pinned VM plus the committed status-timing correction.

The patch is exported from actionc-vm da81c1e. It keeps a fresh checkout
reproducible before that independent repository's commit is published.
Only a private, ignored target/ checkout is populated; no user's VM is edited.
"""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
BASE = '56ddc5c5de41f0e7294e87c440869550eaf53292'
URL = 'https://github.com/mkur/actionc-vm.git'
PATCH = HERE / 'vm-status-timing.patch'
DIGEST = hashlib.sha256(PATCH.read_bytes()).hexdigest()
CACHE = HERE / 'target' / ('qualified-vm-' + DIGEST[:16])


def run(args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def prepare():
    marker = CACHE / 'qualification-source.json'
    if not marker.exists():
        CACHE.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='vm-prepare-', dir=CACHE.parent) as temporary:
            tmp = Path(temporary)
            sibling = ROOT.parent / 'actionc-vm'
            probe = subprocess.run(['git', '-C', str(sibling), 'cat-file', '-e', BASE], capture_output=True)
            if probe.returncode == 0:
                source = sibling
            else:
                source = tmp / 'repository'
                run(['git', 'init', str(source)], stdout=subprocess.DEVNULL)
                run(['git', '-C', str(source), 'fetch', '--depth=1', URL, BASE])
            archive = tmp / 'vm.tar'
            with archive.open('wb') as output:
                run(['git', '-C', str(source), 'archive', BASE], stdout=output)
            checkout = tmp / 'checkout'
            checkout.mkdir()
            with tarfile.open(archive) as files:
                files.extractall(checkout, filter='data')
            run(['git', 'init', str(checkout)], stdout=subprocess.DEVNULL)
            run(['git', 'apply', '--check', str(PATCH)], cwd=checkout)
            run(['git', 'apply', str(PATCH)], cwd=checkout)
            hashes = {str(p.relative_to(checkout)): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in (checkout / 'crates/w65c816').rglob('*') if p.is_file()}
            (checkout / marker.name).write_text(json.dumps({'base': BASE, 'patch': DIGEST, 'hashes': hashes}, indent=2) + '\n')
            checkout.rename(CACHE)
    facts = json.loads(marker.read_text())
    assert facts['base'] == BASE and facts['patch'] == DIGEST
    for relative, digest in facts['hashes'].items():
        assert hashlib.sha256((CACHE / relative).read_bytes()).hexdigest() == digest, f'Changed qualified CPU input: {relative}'
    return CACHE


def main():
    checkout = prepare()
    args = sys.argv[1:]
    if '--prepare-only' in args:
        print(checkout)
        return
    cpu_only = '--cpu' in args
    if cpu_only:
        args.remove('--cpu')
        run(['cargo', 'test', '--locked', '--manifest-path', str(checkout / 'crates/w65c816/Cargo.toml'), '--features', 'reference', *args])
    else:
        patch = 'patch.' + json.dumps(URL) + '.actionc-w65c816.path=' + json.dumps(str(checkout / 'crates/w65c816'))
        output = HERE / 'target' / 'qualification'
        output.mkdir(parents=True, exist_ok=True)
        artifacts = Path(tempfile.mkdtemp(prefix='run-', dir=output))
        command = ['cargo', 'test', '--locked', '--config', patch,
                   '--manifest-path', str(HERE / 'Cargo.toml'), *args]
        run(command, cwd=ROOT, env=dict(os.environ, A816_QUALIFICATION_DIR=str(artifacts)))
        artifact_hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                           for p in artifacts.iterdir() if p.is_file()}
        inputs = (list((ROOT / 'src').rglob('*.rs'))
                  + list((ROOT / 'runtime/65816').glob('*'))
                  + list((HERE / 'tests').rglob('*'))
                  + list((ROOT / 'fixtures/o65').glob('*'))
                  + [ROOT / 'tools/inspect_o65.py',
                     ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', HERE / 'Cargo.toml',
                     HERE / 'Cargo.lock', Path(__file__), ROOT / 'tools/disassemble65816.py'])
        input_hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                        for p in inputs if p.is_file()}
        manifest = {
            'compiler_revision': subprocess.check_output(
                ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            'compiler_and_fixture_inputs': input_hashes,
            'vm_base': BASE,
            'vm_patch_sha256': DIGEST,
            'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(),
            'ca65': subprocess.run(['ca65', '--version'], capture_output=True, text=True).stderr.strip(),
            'ld65': subprocess.run(['ld65', '--version'], capture_output=True, text=True).stderr.strip(),
            'command': command,
            'artifacts': artifact_hashes,
            'irq_schedule': 'each distinct reachable enabled instruction address in preemption.rs',
            'seeds': [0x81620260916, 0x5eedcafe],
        }
        (artifacts / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print('Qualification manifest:', artifacts / 'manifest.json')


if __name__ == '__main__':
    main()
