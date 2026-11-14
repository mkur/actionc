#!/usr/bin/env python3
"""Reproduce parameter-home measurements using an explicitly selected rlib."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-rlib', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--enabled', action='store_true')
    parser.add_argument('--suite', choices=['leaf', 'handoff'], default='leaf')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    rlib = args.api_rlib.resolve()
    probe = output / 'probe'
    subprocess.run(['rustc', '--edition=2024', str(ROOT / 'tools/compare6502/parameter_homes.rs'),
                    '--extern', f'actionc={rlib}', '-L', f'dependency={rlib.parent}', '-o', str(probe)],
                   cwd=ROOT, check=True)
    started = time.monotonic()
    result = subprocess.run(['/usr/bin/time', '-l', str(probe), str(output / 'images')] + (['--enabled'] if args.enabled else []) + (['--suite-handoff'] if args.suite == 'handoff' else []), cwd=ROOT, check=True,
                            text=True, capture_output=True)
    elapsed = time.monotonic() - started
    cases = [json.loads(line) for line in result.stdout.splitlines()]
    sources = ['samples/tn/modern/TN.ACT', 'samples/tn/modern/LIB.ACT', 'fixtures/runtime/parameter_homes.act', 'fixtures/runtime/parameter_registers.act', 'fixtures/runtime/parameter_scalars.act', 'fixtures/runtime/mir6502_phase_trace_homes.act', 'fixtures/runtime/late_producer_cleanup.act']
    if args.suite == 'handoff':
        sources = ['samples/tn/modern/TN.ACT', 'samples/tn/modern/LIB.ACT', 'fixtures/runtime/parameter_argument_handoff.act']
    receipt = dict(revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                   parameter_home_selection=args.enabled,
                   suite=args.suite,
                   probe_elapsed_seconds=elapsed,
                   probe_time_report=result.stderr,
                   timing_scope='One sequential process, all cases traced and untraced; not a statistical performance claim.',
                   worktree_diff_sha256=hashlib.sha256(subprocess.check_output(['git', 'diff', '--', 'src/mir6502'], cwd=ROOT)).hexdigest(),
                   api_rlib_sha256=hashlib.sha256(rlib.read_bytes()).hexdigest(),
                   source_sha256_lf={s: hashlib.sha256((ROOT / s).read_bytes().replace(b'\r\n', b'\n')).hexdigest() for s in sources},
                   cases=cases)
    for row in cases:
        image = output / 'images' / (row['id'] + '.xex')
        row['image_sha256'] = hashlib.sha256(image.read_bytes()).hexdigest()
    (output / 'measurements.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(output / 'measurements.json')


if __name__ == '__main__':
    main()
