#!/usr/bin/env python3
"""Record compact per-slice CRC/sieve and compile-only frozen Exec deltas."""
import argparse
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from build import ROOT, digest, image_guard_ranges, run
from crc import verify

CACHE = ROOT / 'target/crc-sieve-series'
REPORT = ROOT / 'docs/benchmarks/65816-crc-sieve-optimization'


def exec_image(compiler, output):
    cache = ROOT / 'target/exec-current-audit'
    frozen = cache / 'frozen-exec'
    for item in json.loads((cache / 'frozen-inputs.json').read_text()):
        assert digest(frozen / item['path']) == item['snapshot_sha256'], item['path']
    native = frozen / 'build/play-622b139-actionc-32af3e2b/native'
    command = [compiler, '--layout', cache / 'terminal-pointer-value.layout.json', '-o', output]
    for path in (native / 'task-kernel', native, frozen / 'examples', frozen / 'lib'):
        command += ['--module-path', path]
    command += [native / 'kernel-program.act']
    run(command)
    return json.loads(output.read_text())


def exec_summary(before, after):
    for key in ('zero_fill', 'data', 'imports', 'task_headroom', 'irq_headroom'):
        assert before[key] == after[key], key
    assert len(before['routines']) == len(after['routines']) == 631
    rows, guards = [], 0
    for a, b in zip(before['routines'], after['routines']):
        assert {k: v for k, v in a.items() if k not in ('size', 'address')} == {k: v for k, v in b.items() if k not in ('size', 'address')}, a['name']
        pair = []
        for image, routine in ((before, a), (after, b)):
            segment = next(s for s in image['segments'] if s['address'] == routine['address'])
            pair.append([(hi-lo, segment['bytes'][lo-routine['address']:hi-routine['address']-4])
                         for lo, hi in image_guard_ranges(image, segment)])
        assert pair[0] == pair[1], ('guard', a['name'])
        guards += sum(n for n, _ in pair[1])
        if a['size'] != b['size']:
            rows.append(dict(routine=a['name'], before=a['size'], after=b['size'], saved=a['size']-b['size']))
    assert guards == 72252
    code = sum(r['size'] for r in after['routines'])
    return dict(compiler_code=code, saved=sum(r['saved'] for r in rows), guard_bytes=guards,
                estimated_loaded_without_guard_regions=code-guards+8300+2307,
                unchanged_frames_abi_guards=True, changed_routines=rows,
                full_qualification_run=False, guard_disabled_build=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('slice', type=int)
    parser.add_argument('--families', nargs='+', choices=['crc', 'sieve'], default=['crc', 'sieve'])
    parser.add_argument('--exec-only', action='store_true')
    args = parser.parse_args()
    CACHE.mkdir(exist_ok=True)
    REPORT.mkdir(parents=True, exist_ok=True)
    base_path = CACHE / 'baseline-exec.json'
    if not base_path.exists():
        exec_image(CACHE / 'baseline-actionc-65816', base_path)
    after_path = CACHE / f'slice-{args.slice}-exec.json'
    after = exec_image(ROOT / 'target/release/actionc-65816', after_path)
    before_path = CACHE / (f'slice-{args.slice-1}-exec.json' if args.slice > 1 else 'baseline-exec.json')
    result = dict(slice=args.slice, parent_revision=run(['git', 'rev-parse', 'HEAD']).strip(),
                  compiler_sha256=digest(ROOT / 'target/release/actionc-65816'),
                  compiler_changes=run(['git', 'diff', '--stat', '--', 'src']),
                  exec=exec_summary(json.loads(before_path.read_text()), after), benchmarks=[])
    result['source_hashes'] = {str(p.relative_to(ROOT)): digest(p) for p in sorted((ROOT / 'src/mir65816/emit').rglob('*.rs'))}
    if not args.exec_only:
        for family in args.families:
            directory = CACHE / f'slice-{args.slice}-{family}'
            manifest = json.loads((directory / 'manifest.json').read_text())
            verify(manifest)
            debug = json.loads((directory / 'debug.json').read_text())
            release = json.loads((directory / 'release.json').read_text())
            assert debug == release
            for host in ('debug', 'release'):
                att = json.loads((directory / f'{host}-qualification.json').read_text())
                assert att['manifest_sha256'] == digest(directory / 'manifest.json')
                assert att['results_sha256'] == digest(directory / f'{host}.json')
            baseline = json.loads((ROOT / f'docs/benchmarks/65816-{family}-speed/profiles.json').read_text())
            records = release['records']
            failures = [r for r in records if r['errors']]
            assert len(failures) == (10 if family == 'crc' else 0)
            assert all(r['compiler']=='calypsi' and r['mode']=='O2-speed' and r.get('width')==8
                       and all(e.startswith('wrong CRC:') for e in r['errors']) for r in failures)
            for r in baseline:
                keys = ('compiler','mode','width','vector') if family=='crc' else ('compiler','mode','variant','placement','n')
                new = next(v for v in records if all(v[k]==r[k] for k in keys))
                if r['compiler'] != 'actionc':
                    assert new == r, ('foreign control changed', family)
                    continue
                row = {k:r[k] for k in keys}
                row.update(family=family, code_bytes=new['code_bytes'], baseline_code_bytes=r['code_bytes'],
                           cycles=new['cycles'], baseline_cycles=r['cycles'], guard_bytes=new['guard_bytes'],
                           guard_cycles=new['guard_cycles'], peak_stack=new['peak_below_entry_s'])
                result['benchmarks'].append(row)
    result['evidence'] = {str(p.relative_to(ROOT)): digest(p) for p in (before_path, after_path, base_path)}
    (REPORT / f'slice-{args.slice}.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k not in ('source_hashes','evidence','compiler_changes','exec')} | dict(exec={k:v for k,v in result['exec'].items() if k!='changed_routines'}), indent=2))


if __name__ == '__main__':
    main()
