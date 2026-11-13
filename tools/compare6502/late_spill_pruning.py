#!/usr/bin/env python3
"""Qualify opt-in spill pruning and frozen default CLI artifact compatibility."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(args):
    started = time.monotonic()
    result = subprocess.run([str(a) for a in args], cwd=ROOT, capture_output=True, check=True)
    return result.stdout, time.monotonic() - started


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, default=ROOT / 'build/checked-arena/shared-target/debug/actionc-emit')
    parser.add_argument('--api-rlib', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'build/mir6502-late-spills/qualification')
    args = parser.parse_args()
    compiler = args.compiler.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='run-', dir=args.output)).resolve()
    frozen = json.loads((ROOT / 'docs/benchmarks/6502-late-producers/qualification.json').read_text())
    catalog = json.loads((ROOT / 'docs/benchmarks/6502-phase-tracing/baseline.json').read_text())
    expected = {c['id']: c for c in frozen['cases']}
    revision, _ = run(['git', 'rev-parse', 'HEAD'])
    report = dict(baseline_revision='83236a1f', implementation_revision=revision.decode().strip(),
                  compiler_sha256=sha(compiler.read_bytes()), cli_cases=[])
    for case in catalog['cases']:
        source = ROOT / case['source']
        assert sha(source.read_bytes().replace(b'\r\n', b'\n')) == case['source_sha256']
        command = [compiler, '--profile', case['profile'], '--backend', 'mir6502',
                   '--runtime', case['runtime'], '--origin', case['origin']]
        directory = work / case['id']
        directory.mkdir()
        row = dict(id=case['id'], artifacts={})
        for suffix, mode in [('xex', '--emit-load'), ('asm', '--emit-listing'), ('map', '--emit-map')]:
            output, seconds = run(command + [mode, case['source']])
            previous = expected[case['id']]['artifacts'][suffix]
            assert sha(output) == previous['after_sha256'], (case['id'], suffix)
            (directory / f'output.{suffix}').write_bytes(output)
            row['artifacts'][suffix] = dict(sha256=sha(output), bytes=len(output), seconds=seconds)
        report['cli_cases'].append(row)
        print(case['id'] + ': all default artifacts unchanged', flush=True)
    rlib = args.api_rlib or max((compiler.parent / 'deps').glob('libactionc-*.rlib'), key=lambda p: p.stat().st_mtime)
    report['api_rlib_sha256'] = sha(rlib.read_bytes())
    sources = ['fixtures/runtime/home_value_invalidation.act',
               'fixtures/runtime/argument_staging.act', 'fixtures/runtime/checked_pointer_ranges.act',
               'fixtures/runtime/word_load_sharing.act', 'fixtures/runtime/late_producer_cleanup.act',
               'samples/tn/modern/TN.ACT', 'fixtures/runtime/late_spill_pruning.act']
    report['api_source_sha256_lf'] = {p: sha((ROOT / p).read_bytes().replace(b'\r\n', b'\n')) for p in sources}

    probe = work / ('api-probe.exe' if sys.platform == 'win32' else 'api-probe')
    run(['rustc', '--edition=2024', ROOT / 'tools/compare6502/late_spill_pruning.rs',
         '--extern', f'actionc={rlib}', '-L', f'dependency={rlib.parent}', '-o', probe])
    output, seconds = run([probe, work / 'api'])
    (work / 'api.jsonl').write_bytes(output)
    report['api'] = [json.loads(line) for line in output.decode().splitlines()]
    report['api_seconds'] = seconds
    assert len(report['api']) == 32
    assert all(c['after_bytes'] == c['before_bytes'] for c in report['api'])
    report['source_fixture_image_savings'] = 0
    report['cli_artifact_comparisons'] = sum(len(c['artifacts']) for c in report['cli_cases'])
    report['full_api_comparisons'] = len(report['api'])
    (work / 'qualification.json').write_text(json.dumps(report, indent=2) + '\n')
    print(work / 'qualification.json')


if __name__ == '__main__':
    main()
