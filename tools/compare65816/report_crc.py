#!/usr/bin/env python3
"""Archive verified CRC timings and retain Calypsi's incorrect CRC8 output."""
import argparse
import csv
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
from crc import ROOT, digest, verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, default=ROOT / 'target/crc65816-comparison')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/65816-crc-speed')
    args = parser.parse_args()
    src, out = args.input.resolve(), args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    load = lambda name: json.loads((src / name).read_text())
    manifest, debug, release = load('manifest.json'), load('debug.json'), load('release.json')
    verify(manifest)
    assert debug == release and not release['short'] and manifest['crlf_checked']
    records = release['records']
    assert len(records) == len(manifest['artifacts']) * len(manifest['vectors']) == 112
    key = lambda r: (r['compiler'], r['mode'], r['width'], r['vector'])
    assert len({key(r) for r in records}) == len(records)
    failures = [r for r in records if r['errors']]
    assert len(failures) == 10
    assert all(r['compiler'] == 'calypsi' and r['width'] == 8 and r['mode'] == 'O2-speed'
               and all(e.startswith('wrong CRC:') for e in r['errors']) for r in failures)
    quals = {}
    for mode in ('debug', 'release'):
        qualification = load(mode + '-qualification.json')
        assert qualification['manifest_sha256'] == digest(src / 'manifest.json')
        assert qualification['results_sha256'] == digest(src / (mode + '.json'))
        assert qualification['log_sha256'] == digest(src / (mode + '.log'))
        assert qualification['allow_external_result_errors']
        vm = qualification['vm_qualification']
        harness = 'tools/native65816-runtime-tests/tests/crc_bench.rs'
        assert vm['compiler_and_fixture_inputs'][harness] == digest(ROOT / harness)
        quals[mode] = {k: vm[k] for k in ('compiler_revision', 'vm_base', 'vm_patch_sha256', 'rust', 'command')}
        quals[mode]['attestation_sha256'] = digest(src / (mode + '-qualification.json'))
    def table(name, rows):
        with (out / name).open('w', newline='') as stream:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0]), lineterminator='\n')
            writer.writeheader()
            writer.writerows(rows)
    keys = ['width', 'compiler', 'mode', 'vector', 'length', 'result', 'code_bytes', 'guard_bytes',
            'cycles', 'guard_cycles', 'instructions', 'stack_reads', 'stack_writes', 'dp_reads',
            'dp_writes', 'mode_switches', 'input_reads', 'peak_below_entry_s']
    table('measurements.csv', [{k: r[k] for k in keys} | dict(correct=not r['errors']) for r in records])
    rows, profiles = [], []
    for r in records:
        if r['vector'] != 'benchmark-8192':
            continue
        all_correct = all(not p['errors'] for p in records if key(p)[:3] == key(r)[:3])
        rows.append({k: r[k] for k in ('width', 'compiler', 'mode', 'code_bytes', 'guard_bytes', 'cycles', 'guard_cycles')}
                    | dict(cycles_per_byte=r['cycles'] / r['length'], all_vectors_correct=all_correct))
        profiles.append(r)
    table('summary.csv', rows)
    (out / 'profiles.json').write_text(json.dumps(profiles, indent=2) + '\n')
    (out / 'failures.json').write_text(json.dumps(failures, indent=2) + '\n')
    for artifact in manifest['artifacts']:
        lines = []
        for line in (Path(artifact['directory']) / 'code.linked.lst').read_text().splitlines():
            if line and line[0] in '0123456789ABCDEF' and not any(
                    r['address'] <= int(line[:6], 16) < r['address'] + r['size'] for r in artifact['routines']):
                continue  # Exclude the unused Action! Main routine.
            lines.append(line.rstrip())
        name = f'crc{artifact["width"]}-{artifact["compiler"]}-{artifact["mode"]}.lst'
        (out / name).write_text('\n'.join(lines) + '\n')
    provenance = {k: manifest[k] for k in ('compiler_revision', 'compiler_worktree_status', 'suite_revision',
                                         'suite_crc_status', 'versions', 'tools', 'inputs', 'crlf_checked')}
    provenance.update(paired_records=112, executions_per_host=224, debug_release_identical=True,
                      failed_paired_records=10, qualifications=quals,
                      archived_outputs={p.name: digest(p) for p in sorted(out.iterdir())
                                        if p.suffix in ('.csv', '.lst') or p.name in ('profiles.json', 'failures.json')},
                      artifacts=[{k: a[k] for k in ('compiler', 'mode', 'width', 'commands', 'hashes')} for a in manifest['artifacts']],
                      evidence={p.name: digest(p) for p in [src / 'manifest.json', src / 'debug.json', src / 'release.json']},
                      tools_hashes={str(p.relative_to(ROOT)): digest(p) for p in [Path(__file__).resolve(),
                          ROOT / 'tools/compare65816/run_crc.py', ROOT / 'tools/native65816-runtime-tests/tests/crc_bench.rs']})
    (out / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    print(json.dumps(rows, indent=2))


if __name__ == '__main__':
    main()
