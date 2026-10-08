#!/usr/bin/env python3
"""Measure a pinned call-flow CLI generation serially, with image authentication."""
import argparse
import platform
from pathlib import Path
import shutil
import statistics
import subprocess

from exec_call_candidate import ROOT, authenticate
from exec_record_baseline import PROFILES, digest, save, verify_inputs
from measure_host import run


def commands(base, output, binary):
    # Match record_probe's explicit module providers and per-profile layout.
    # No ignored historical host manifest is required to reproduce this run.
    result = []
    for profile, (optimize, _) in PROFILES.items():
        generated = base/'profiles'/profile
        modules = [generated/'task-kernel', generated, base/'exec/examples/shell',
                   *(base/'exec/lib'/name for name in ('exec', 'dos', 'fs', 'console',
                       'display', 'input', 'mydos', 'spartados', 'io')), ROOT/'runtime/65816']
        command = [str(binary)]
        for directory in modules:
            command.extend(['--module-path', str(directory)])
        command.extend(['--layout', str(output/profile/'probe.layout.json'), '-o', str(output/'host-image.json')])
        if not optimize:
            command.append('--no-opt')
        command.append(str(generated/'kernel-program.act'))
        result.append((profile, command))
    return result


def measure(base, output, binary, rounds):
    facts = authenticate(output)
    verify_inputs(base)
    pinned = output/'actionc-65816'
    if binary.resolve() != pinned.resolve():
        shutil.copyfile(binary, pinned)
        pinned.chmod(0o755)
    binary_hash = digest(pinned)
    jobs = commands(base, output, pinned)
    samples = []
    for name, command in jobs:
        run(command, output/f'host-warm-{name}.log')
        if digest(output/'host-image.json') != facts['profiles'][name]['artifacts']['probe.image.json']:
            raise ValueError('CLI/probe image mismatch: '+name)
        print('Warm-up:', name, flush=True)
    for round_number in range(rounds):
        for name, command in jobs if round_number % 2 == 0 else reversed(jobs):
            sample = run(command, output/f'host-{round_number}-{name}.log')
            if digest(output/'host-image.json') != facts['profiles'][name]['artifacts']['probe.image.json']:
                raise ValueError('Measured image drift: '+name)
            samples.append(dict(round=round_number, profile=name, **sample))
            print(f"Round {round_number+1}/{rounds}: {name}: {sample['wall_seconds']:.3f}s", flush=True)
    if digest(pinned) != binary_hash:
        raise ValueError('Pinned CLI drift')
    authenticate(output)
    verify_inputs(base)
    medians = {name: {key: statistics.median(s[key] for s in samples if s['profile'] == name)
                     for key in ('wall_seconds', 'user_seconds', 'system_seconds', 'peak_rss_bytes')}
               for name in PROFILES}
    save(output/'host-results.json', dict(schema=1, rounds=rounds, samples=samples, medians=medians,
        methodology='One warm-up per profile, serial CLI jobs, alternating profile order; all wait4 samples retained; no concurrent builds or qualification.',
        rust=facts['rust'], platform=platform.platform(), binary_sha256=binary_hash,
        candidate_provenance_sha256=digest(output/'provenance.json'),
        measurement_tools={str(p.relative_to(ROOT)): digest(p) for p in (Path(__file__), Path(__file__).with_name('measure_host.py'))},
        commands={name: command for name, command in jobs}))
    (output/'host-image.json').unlink()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, default=ROOT/'target/record-placement-stage0')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=3)
    args = parser.parse_args()
    if args.rounds < 3:
        parser.error('At least three measured rounds are required')
    measure(args.base.resolve(), args.output.resolve(), args.binary.resolve(), args.rounds)
