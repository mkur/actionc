#!/usr/bin/env python3
"""Measure warm-cache native CLI compilation using per-child wait4 accounting."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import tempfile
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, log):
    start = time.perf_counter()
    with log.open('wb') as output:
        child = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    elapsed = time.perf_counter() - start
    if child.returncode:
        raise RuntimeError(f'compiler failed ({child.returncode}): {log.read_text()}')
    # Darwin reports bytes; Linux reports KiB. Do not use cumulative RUSAGE_CHILDREN.
    scale = 1 if sys.platform == 'darwin' else 1024
    return dict(wall_seconds=elapsed, user_seconds=usage.ru_utime,
                system_seconds=usage.ru_stime, peak_rss_bytes=int(usage.ru_maxrss * scale))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=7)
    parser.add_argument('--expected-builds', type=int, default=28,
                        help='Exact Action build count (use the generated ladder count for scaling probes)')
    args = parser.parse_args()
    if sys.platform not in ('darwin', 'linux') or not hasattr(os, 'wait4'):
        parser.error('per-child peak RSS accounting requires Darwin or Linux wait4')
    if args.rounds < 3:
        parser.error('use at least three measured rounds')
    binaries = dict(before=args.before.resolve(), after=args.after.resolve())
    for binary in binaries.values():
        if not binary.is_file():
            parser.error(f'missing compiler: {binary}')
    manifest = json.loads(args.manifest.read_text())
    builds = [a for a in manifest['artifacts'] if a['compiler'] == 'actionc']
    assert args.expected_builds > 0
    assert len(builds) == args.expected_builds
    assert len({(a['case'], a['mode']) for a in builds}) == args.expected_builds
    samples = []
    with tempfile.TemporaryDirectory(prefix='actionc-host-measurement-') as directory:
        directory = Path(directory)
        image = directory / 'image.json'
        log = directory / 'compiler.log'
        commands = []
        for build in builds:
            command = list(build['commands'][0])
            command[command.index('-o') + 1] = str(image)
            expected = build['hashes']['image.json']
            commands.append((build, command, expected))
        # Warm binaries, shared libraries and all input files before timing.
        for build, command, expected in commands:
            for binary in binaries.values():
                run([str(binary), *command[1:]], log)
                assert digest(image) == expected, (build['case'], build['mode'], str(binary))
        for round_number in range(args.rounds):
            order = ('before', 'after') if round_number % 2 == 0 else ('after', 'before')
            sequence = commands if round_number % 2 == 0 else list(reversed(commands))
            for build, command, expected in sequence:
                for compiler in order:
                    result = run([str(binaries[compiler]), *command[1:]], log)
                    assert digest(image) == expected, (build['case'], build['mode'], compiler)
                    samples.append(dict(round=round_number, case=build['case'],
                                        mode=build['mode'], compiler=compiler,
                                        image_sha256=expected, **result))
    per_build = []
    for build in builds:
        row = dict(case=build['case'], mode=build['mode'])
        for compiler in binaries:
            selected = [s for s in samples if (s['case'], s['mode'], s['compiler']) ==
                        (build['case'], build['mode'], compiler)]
            row[compiler] = {key: statistics.median(s[key] for s in selected)
                             for key in ('wall_seconds', 'user_seconds', 'system_seconds', 'peak_rss_bytes')}
        row['wall_ratio'] = row['after']['wall_seconds'] / row['before']['wall_seconds']
        row['rss_ratio'] = row['after']['peak_rss_bytes'] / row['before']['peak_rss_bytes']
        per_build.append(row)
    totals = {}
    for compiler in binaries:
        totals[compiler] = dict(
            median_corpus_wall_seconds=statistics.median(
                sum(s['wall_seconds'] for s in samples if s['compiler'] == compiler and s['round'] == r)
                for r in range(args.rounds)),
            median_compile_peak_rss_bytes=statistics.median(s['peak_rss_bytes'] for s in samples if s['compiler'] == compiler),
            maximum_compile_peak_rss_bytes=max(s['peak_rss_bytes'] for s in samples if s['compiler'] == compiler))
    report = dict(schema=1, methodology='Separate release CLI process per build; one warm-up of all inputs per compiler, then alternating compiler/build order. Wall time includes process launch, parsing, compilation and JSON output. wait4 reports per-child CPU and peak RSS; outputs must match the frozen image hash on every run. No concurrent build or native qualification is intentionally run during measurement.',
                  platform=platform.platform(), machine=platform.machine(), cpu_count=os.cpu_count(),
                  python=sys.version, rounds=args.rounds, builds=len(builds),
                  binaries={name: dict(path=str(path), sha256=digest(path)) for name, path in binaries.items()},
                  manifest=dict(path=str(args.manifest.resolve()), sha256=digest(args.manifest)),
                  script_sha256=digest(Path(__file__)), summary=totals, per_build=per_build, samples=samples)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(totals, indent=2))


if __name__ == '__main__':
    main()
